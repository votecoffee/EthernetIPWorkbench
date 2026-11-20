"""Compare localhost CSV timing with HTTP, MQTT and shared memory enabled and disabled."""

import argparse
from concurrent.futures import ThreadPoolExecutor
from contextlib import ExitStack
import csv
import hashlib
from http.client import HTTPException
import json
from pathlib import Path
import socket
from statistics import median
import subprocess
import sys
import threading
import time
import tomllib
from uuid import uuid4

from join_capture import check_report, parts_for, read_header, transfer_checked
from test_cli import PROFILE, with_adapter
from test_integration import free_port, get, read_memory, receive_mqtt, shared_memory

SCANNER_FIELDS = (
    "requested_ot_us", "requested_to_us", "actual_ot_us", "actual_to_us",
    "receive_spacing", "send_spacing", "sequence_gaps", "duplicates", "late", "invalid",
    "timeouts", "skipped_deadlines", "accepted", "sent", "written", "buffer_overruns",
    "duration_reached", "forward_close", "integration", "schedule",
)
ADAPTER_FIELDS = (
    "to_send_spacing", "ot_receive_spacing", "to_skipped_deadline_slots",
    "to_sent", "ot_received", "ot_sequence_gaps", "ot_invalid", "forward_close",
)


class Observations:
    def __init__(self):
        self.first = self.last = None
        self.repeats = self.index_gaps = 0
        self.ages = []

    def record(self, sample):
        index = sample["sample_index"]
        if index == 0:
            return
        if self.last == index:
            self.repeats += 1
            return
        if self.last is None:
            self.first = index
        elif index < self.last:
            raise ValueError("Feed sample index went backwards during a steady-state run")
        else:
            self.index_gaps += index - self.last - 1
        self.last = index
        self.ages.append(time.time_ns() // 1000 - sample["stream_start_utc_us"] - sample["elapsed_us"])

    def summary(self):
        ages = sorted(self.ages)
        timing = {
            f"p{p}_us": ages[(len(ages) * p - 1) // 100] for p in (50, 95, 99)
        } if ages else {}
        return {
            "distinct_samples": len(ages), "first_sample_index": self.first, "last_sample_index": self.last,
            "repeated_observations": self.repeats, "sample_index_gaps": self.index_gaps,
            "apparent_age": timing | ({"min_us": ages[0], "max_us": ages[-1]} if ages else {}),
        }


def observe_clients(pending, port, name, poll_seconds):
    http, memory = Observations(), Observations()
    http_errors = memory_errors = 0
    with ExitStack() as handles:
        snapshot = None
        while not pending.done():
            try:
                status, document = get(port, "/v1/latest", timeout=0.15)
                if status == 200 and document["sample"] is not None:
                    http.record(document["sample"])
            except (OSError, HTTPException):
                http_errors += 1
            if http.first is not None:
                try:
                    if snapshot is None:
                        snapshot = handles.enter_context(shared_memory(name))
                    memory.record(read_memory(*snapshot))
                except OSError:
                    memory_errors += 1
            time.sleep(poll_seconds)
    return {
        "http": http.summary() | {"poll_errors": http_errors},
        "shared_memory": memory.summary() | {"poll_errors": memory_errors},
    }


def benchmark_run(exe, root, label, source, enabled, seconds, rpi):
    root.mkdir()
    profile = root / "profile.toml"
    capture_dir = root / "capture"
    command = [exe, "capture", profile, capture_dir, str(seconds), "0:12:1:305419896"]
    fixture_args = ("--tcp-timeout", str(seconds + 10))
    result = {"profile": label, "errors": []}
    try:
        with ExitStack() as stack:
            if enabled:
                broker = stack.enter_context(socket.socket())
                broker.bind(("127.0.0.1", 0))
                broker.listen()
                broker.settimeout(6)
                port = free_port()
                name = f"bench-{uuid4().hex}"
                source += f"""
[app.integration]
http_port = {port}
shared_memory = "{name}"
[app.integration.mqtt]
broker = "127.0.0.1"
port = {broker.getsockname()[1]}
topic = "bench/test/samples"
period_ms = 25
"""
                mqtt, broker_errors = Observations(), []
                workers = stack.enter_context(ThreadPoolExecutor(max_workers=2))
                broker_job = workers.submit(receive_mqtt, broker, None, broker_errors, False, observe=mqtt.record)
            profile.write_text(source, encoding="utf-8")
            if enabled:
                pending = workers.submit(with_adapter, root, "capture", command,
                                         fixture_args=fixture_args, timeout=seconds + 15)
                result["consumers"] = observe_clients(pending, port, name, 0.020)
                cli, fixture = pending.result()
                broker_job.result(timeout=7)
                result["consumers"]["mqtt"] = mqtt.summary()
                result["errors"].extend(f"MQTT broker fixture: {error}" for error in broker_errors)
                for transport, observed in result["consumers"].items():
                    if observed["distinct_samples"] == 0:
                        result["errors"].append(f"No samples were observed over {transport}")
            else:
                cli, fixture = with_adapter(root, "capture", command, fixture_args=fixture_args, timeout=seconds + 15)
        if cli.stderr:
            (root / "scanner.stderr.txt").write_text(cli.stderr, encoding="utf-8")
        if cli.returncode != 0:
            result["errors"].append(f"Capture exited with code {cli.returncode}")
        result["adapter"] = {key: fixture[key] for key in ADAPTER_FIELDS if key in fixture}
        if not fixture.get("forward_close"):
            result["errors"].append("Simulator did not confirm Forward_Close")
    except (OSError, ValueError, AssertionError, RuntimeError, subprocess.TimeoutExpired) as error:
        result["errors"].append(f"Fixture or consumer failed: {error}")

    try:
        with (capture_dir / "capture.toml").open("rb") as file:
            report = tomllib.load(file)
        result["scanner"] = {key: report[key] for key in SCANNER_FIELDS if key in report}
        if any(report.get(key) != rpi for key in ("requested_ot_us", "requested_to_us", "actual_ot_us", "actual_to_us")):
            result["errors"].append("Requested and Forward_Open intervals must match in both directions")
        if not report.get("duration_reached") or report.get("forward_close") != {"outcome": "confirmed"}:
            result["errors"].append("Timed capture did not reach its deadline and confirm Forward_Close")
        if enabled != ("integration" in report):
            result["errors"].append("Reported integration state differs from the benchmark profile")
        prefix = Path(report["csv_prefix"])
        parts = parts_for(prefix)
        checked = check_report(report, prefix, len(parts))
        result["csv"] = transfer_checked(parts, read_header(parts[0]), checked, None)
        if not result["errors"]:
            comparison_fields(result)
    except (OSError, ValueError, KeyError, csv.Error) as error:
        result["errors"].append(f"CSV or report verification failed: {error}")
    return result


def comparison_fields(run):
    s, a = run["scanner"], run["adapter"]
    return {
        "scanner_receive_p99_us": s["receive_spacing"]["p99_us"],
        "scanner_receive_max_us": s["receive_spacing"]["max_us"],
        "scanner_send_p99_us": s["send_spacing"]["p99_us"],
        "scanner_send_max_us": s["send_spacing"]["max_us"],
        "scanner_sequence_gaps": s["sequence_gaps"],
        "scanner_skipped_deadlines": s["skipped_deadlines"],
        "csv_buffer_overruns": s["buffer_overruns"],
        "adapter_skipped_deadlines": a["to_skipped_deadline_slots"],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("exe", type=Path, help="Path to eipmaster-cli.exe")
    parser.add_argument("directory", type=Path, help="New directory for profiles, captures, and the summary")
    parser.add_argument("--pairs", type=int, default=3)
    parser.add_argument("--seconds", type=int, default=5, help="Capture time per run")
    parser.add_argument("--rpi-us", type=int, choices=(1000, 8000), default=1000)
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("the shared-memory benchmark runs on Windows")
    if not (1 <= args.pairs <= 10 and 1 <= args.seconds <= 60):
        parser.error("use 1 to 10 pairs and 1 to 60 seconds")
    exe = args.exe.resolve(strict=True)
    root = args.directory.resolve()
    root.mkdir(parents=True)
    source = PROFILE.read_text(encoding="utf-8").replace("rpi_us = 1000", f"rpi_us = {args.rpi_us}")
    pairs, differences = [], []
    for index in range(args.pairs):
        directory = root / f"pair-{index + 1}"
        directory.mkdir()
        order = ["disabled", "enabled"] if index % 2 == 0 else ["enabled", "disabled"]
        runs = {}
        for label in order:
            print(f"Pair {index + 1}/{args.pairs}, feeds {label}", flush=True)
            runs[label] = benchmark_run(exe, directory / label, label, source, label == "enabled",
                                        args.seconds, args.rpi_us)
        delta = None
        if not any(run["errors"] for run in runs.values()):
            before, after = (comparison_fields(runs[label]) for label in ("disabled", "enabled"))
            delta = {key: after[key] - value for key, value in before.items()}
            differences.append(delta)
        pairs.append({"number": index + 1, "order": order, "runs": runs, "enabled_minus_disabled": delta})
    summary = {
        "source": "headless_localhost",
        "executable_sha256": hashlib.sha256(exe.read_bytes()).hexdigest(),
        "requested_rpi_us": args.rpi_us,
        "seconds_per_run": args.seconds,
        "http_and_shared_memory_delay_ms": 20,
        "mqtt_publish_period_ms": 25,
        "compared_pairs": len(differences),
        "median_enabled_minus_disabled": {
            key: median(pair[key] for pair in differences) for key in differences[0]
        } if differences else {},
        "pairs": pairs,
    }
    path = root / "comparison.json"
    path.write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    print(f"Compared {len(differences)} of {args.pairs} pairs. {path}")
    return 0 if len(differences) == args.pairs else 1


if __name__ == "__main__":
    raise SystemExit(main())
