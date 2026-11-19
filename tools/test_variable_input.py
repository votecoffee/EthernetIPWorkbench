"""Verify variable-size Class 1 input with independent loopback clients."""

import argparse
import json
from pathlib import Path
import socket
import subprocess
import sys
import threading
import time
import tomllib
from uuid import uuid4

from test_cli import FIXTURE, NO_WINDOW, PROFILE, check_capture_window, parts_for, rows, with_adapter
from test_integration import free_port, get, read_memory, receive_mqtt, shared_memory


def check_sample(sample):
    payload = bytes.fromhex(sample["payload_hex"])
    sequence = sample["packet_sequence"]
    assert sequence not in (10, 20)
    assert len(payload) == (4, 7, 12)[(sequence // 8) % 3]
    assert int.from_bytes(payload[:4], "little") == sequence // 16
    assert payload[4:] == b"\xA5" * (len(payload) - 4)
    return len(payload)


def check_report(report, maximum):
    assert report["input_min_size"] == 4 and report["input_size"] == maximum
    assert report["actual_ot_us"] == report["actual_to_us"] == 8000
    assert report["invalid"] == report["sequence_gaps"] == 2
    assert report["forward_close"] == {"outcome": "confirmed"}
    samples = list(rows(parts_for(Path(report["csv_prefix"]))))
    check_capture_window(report, samples)
    assert len(samples) > 0 and report["buffer_overruns"] == 0
    assert all(int(row[-1]) == int(row[4]) // 16 for row in samples)
    return samples


def check_exports(exe, root, source):
    http_port = free_port()
    name = f"variable-{uuid4().hex}"
    profile = root / "standard.toml"
    with socket.socket() as broker:
        broker.bind(("127.0.0.1", 0))
        broker.listen()
        broker.settimeout(6)
        profile.write_text(source + f"""
[app.integration]
http_port = {http_port}
shared_memory = "{name}"
[app.integration.mqtt]
broker = "127.0.0.1"
port = {broker.getsockname()[1]}
topic = "bench/test/samples"
period_ms = 16
""", encoding="utf-8")
        messages, errors = [], []
        worker = threading.Thread(target=receive_mqtt, args=(broker, messages, errors, False), daemon=True)
        worker.start()
        fixture = subprocess.Popen(
            [sys.executable, "-B", FIXTURE, root / "standard-adapter.json", "--mode", "variable-input"],
            stdout=subprocess.PIPE, text=True, creationflags=NO_WINDOW,
        )
        capture = None
        seen_http, seen_memory = set(), set()
        try:
            assert fixture.stdout.readline().strip() == "Ready on loopback"
            capture = subprocess.Popen(
                [exe, "capture", profile, root / "standard", "2", "0:12:1"],
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, encoding="utf-8", creationflags=NO_WINDOW,
            )
            deadline = time.monotonic() + 2
            while True:
                try:
                    status, reply = get(http_port, "/v1/latest")
                    if status == 200 and reply["sample"]:
                        break
                except OSError:
                    pass
                assert time.monotonic() < deadline, "Timed out waiting for the first input"
                time.sleep(0.01)
            with shared_memory(name) as memory:
                cursor = 0
                while time.monotonic() < deadline and (seen_http != {4, 7, 12} or seen_memory != {4, 7, 12}):
                    status, reply = get(http_port, f"/v1/samples?after={cursor}")
                    assert status == 200
                    for sample in reply["samples"]:
                        seen_http.add(check_sample(sample))
                        cursor = sample["sample_index"]
                        assert int(sample["values"]["value"]) == sample["packet_sequence"] // 16
                    seen_memory.add(check_sample(read_memory(*memory)))
                    time.sleep(0.01)
                stdout, stderr = capture.communicate(timeout=5)
                assert capture.returncode == 0, (stdout, stderr)
                assert read_memory(*memory)["active_generation"] == 0
            assert fixture.wait(timeout=5) == 0
            worker.join(timeout=3)
            assert not worker.is_alive() and not errors, errors
        finally:
            for process in (capture, fixture):
                if process is not None and process.poll() is None:
                    process.terminate()
                    process.wait(timeout=5)
    assert seen_http == seen_memory == {4, 7, 12}
    assert {check_sample(message) for message in messages} == {4, 7, 12}
    report = tomllib.loads(stdout)
    assert report["integration"]["received"] == report["accepted"]
    assert report["integration"]["queue_drops"] == 0
    assert report["unchanged"] == 0
    return len(check_report(report, 12))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("exe", type=Path)
    parser.add_argument("results", type=Path)
    args = parser.parse_args()
    exe, root = args.exe.resolve(strict=True), args.results.resolve()
    root.mkdir(parents=True, exist_ok=False)
    source = "input_min_size = 4\n" + PROFILE.read_text(encoding="utf-8")
    source = (source.replace("input_size = 4", "input_size = 12")
              .replace("input_run_idle = false", "input_run_idle = true").replace("rpi_us = 1000", "rpi_us = 8000"))

    profile = root / "variable.toml"
    profile.write_text(source, encoding="utf-8")
    def call(*command):
        return subprocess.run([exe, *command], capture_output=True, encoding="utf-8",
                              timeout=5, creationflags=NO_WINDOW)

    decoded = call("decode-input", profile, "01 02 03 04")
    assert decoded.returncode == 0, decoded.stderr
    report = tomllib.loads(decoded.stdout)
    assert (report["input_min_size"], report["input_size"], report["payload_bytes"]) == (4, 12, 4)
    assert report["fields"][0]["value"] == "67305985"
    assert call("decode-input", profile, "01 02 03").returncode == 1
    assert call("decode-input", profile, "00 " * 13).returncode == 1

    eds = root / "variable.eds"
    eds.write_text(PROFILE.with_suffix(".eds").read_text(encoding="utf-8")
                   .replace("0x22440305", "0x22440309").replace("Param1,,Assem101", "Param1,12,Assem101"),
                   encoding="utf-8")
    preflight = call("validate", profile, eds, "Connection1")
    assert preflight.returncode == 0, (preflight.stdout, preflight.stderr)
    verified = tomllib.loads(preflight.stdout)
    assert verified["input"]["payload_bytes"] == 12 and verified["input"]["minimum_payload_bytes"] == 4
    assert any(row["field"] == "T->O size mode" and row["state"] == "eds" for row in verified["origins"])

    invalid = root / "mapping-too-wide.toml"
    invalid.write_text(source.replace("offset = 0", "offset = 1"), encoding="utf-8")
    preflight = call("validate", invalid)
    assert preflight.returncode == 1 and not tomllib.loads(preflight.stdout)["connection_valid"]

    captured = check_exports(exe, root, source)
    large = root / "large.toml"
    large.write_text('large_forward_open = true\ncapture_mode = "on_change"\n' +
                     source.replace("input_size = 12", "input_size = 600"), encoding="utf-8")
    result, fixture = with_adapter(root, "large", [exe, "capture", large, root / "large", "1", "0:12:1"],
                                   "large-variable-input")
    assert result.returncode == 0, (result.stdout, result.stderr)
    stats = tomllib.loads(result.stdout)
    samples = check_report(stats, 600)
    assert fixture["forward_close"] and stats["unchanged"] > 0
    # Changes in only the unmapped tail or its length must not trigger rows.
    assert len(set(row[-1] for row in samples)) == len(samples)
    print(f"Variable input passed at 8 ms. Standard capture saved {captured} rows. "
          f"Large capture saved {len(samples)} changed rows. HTTP, MQTT and shared memory preserved lengths.")


if __name__ == "__main__":
    main()
