"""Check the C# consumers against a real Windows scanner and a loopback adapter."""

import argparse
from contextlib import ExitStack
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
from pathlib import Path
import sys
import threading
import time
import tomllib
from uuid import uuid4

from test_integration import free_port, get
from test_recovery import PROFILE, adapter, fixture_report, process

SAMPLES = 60
HISTORY_SAMPLES = 180


def frame(sample):
    return (sample["stream_start_utc_us"], sample["generation"], sample["packet_sequence"],
            sample["cip_sequence"], sample["elapsed_us"], bytes.fromhex(sample["payload_hex"]))


def collect(reader, root, transport):
    stdout, stderr = reader.communicate(timeout=10)
    assert reader.returncode == 0, (stdout, stderr)
    (root / f"{transport}.jsonl").write_text(stdout, encoding="utf-8")
    records = [json.loads(line) for line in stdout.splitlines()]
    assert len(records) == (HISTORY_SAMPLES if transport == "history" else SAMPLES)
    indexed = {}
    previous = 0
    for record in records:
        sample = record["sample"]
        index = sample["sample_index"]
        assert record["transport"] == transport
        assert index > previous
        assert record["skipped_export_indexes"] == (index - previous - 1 if previous else 0)
        if transport == "history":
            assert index == previous + 1 and record["indexes_before_retained_history"] == 0
        else:
            assert record["active_generation"] == sample["generation"] in (1, 2)
            assert 0 <= record["sample_age_us"] <= 500_000
        assert sample["input_assembly"] == 101
        payload = bytes.fromhex(sample["payload_hex"])
        assert len(payload) == 4
        if transport in ("http", "history"):
            assert set(sample["values"]) == {"counter", "low_bit", "swapped_word"}
            assert int(sample["values"]["counter"]) == int.from_bytes(payload, "little")
        else:
            assert sample["values"] is None
        indexed[index] = frame(sample)
        previous = index
    assert {frame[1] for frame in indexed.values()} == {1, 2}
    assert len({frame[0] for frame in indexed.values()}) == 1
    return indexed


def check_history_boundaries(dll, root):
    def batch(start, oldest, latest, indexes, active):
        return {
            "schema_version": 1, "stream_start_utc_us": start, "active_generation": active,
            "oldest_sample_index": oldest, "latest_sample_index": latest,
            "samples": [
                {"schema_version": 1, "stream_start_utc_us": start, "sample_index": index, "generation": 1,
                 "elapsed_us": index * 8000, "packet_sequence": index, "cip_sequence": index,
                 "input_assembly": 101, "payload_hex": "01 00 00 00", "values": {"counter": "1"}}
                for index in indexes
            ],
        }

    start = 1700000000000000
    pages = {
        0: (0, 200, batch(start, 11, 76, range(11, 75), 0)),
        1: (74, 503, {}),
        2: (74, 200, batch(start, 77, 79, (77, 79), 2)),
        3: (79, 200, batch(start + 1, 1, 1, (), 1)),
        4: (0, 200, batch(start + 1, 1, 1, (1,), 1)),
    }
    paths = []

    class Fixture(BaseHTTPRequestHandler):
        def do_GET(self):
            expected, status, body = pages.get(len(paths), (None, 400, {}))
            paths.append(self.path)
            if self.path != f"/v1/samples?after={expected}":
                status, body = 400, {}
            encoded = json.dumps(body).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(encoded)))
            self.end_headers()
            self.wfile.write(encoded)

        def log_message(self, *_):
            pass

    with HTTPServer(("127.0.0.1", 0), Fixture) as server:
        worker = threading.Thread(target=server.serve_forever, kwargs={"poll_interval": 0.05})
        worker.start()
        try:
            with process(["dotnet", dll, "history", str(server.server_port), "67"]) as reader:
                stdout, stderr = reader.communicate(timeout=6)
                assert reader.returncode == 0, (stdout, stderr, paths)
        finally:
            server.shutdown()
            worker.join(timeout=2)
            assert not worker.is_alive()

    (root / "history-boundaries.jsonl").write_text(stdout, encoding="utf-8")
    records = [json.loads(line) for line in stdout.splitlines()]
    assert len(records) == 67 and len(paths) == len(pages)
    for record, expected in zip(records, [*range(11, 75), 77, 79, 1], strict=True):
        assert record["sample"]["sample_index"] == expected
    assert [records[i]["skipped_export_indexes"] for i in (0, 64, 65, 66)] == [10, 2, 1, 0]
    assert [records[i]["indexes_before_retained_history"] for i in (0, 64, 65, 66)] == [10, 2, 0, 0]
    assert records[0]["active_generation"] == 0 and records[-1]["sample"]["stream_start_utc_us"] == start + 1
    print("History reader kept its cursor on HTTP 503, reported retention gaps, and restarted on a new stream.")


def verify_history(port, reads):
    last = max(max(reader) for reader in reads)
    expected = {}
    cursor = 0
    while cursor < last:
        status, batch = get(port, f"/v1/samples?after={cursor}")
        assert status == 200 and batch["samples"]
        for sample in batch["samples"]:
            expected[sample["sample_index"]] = frame(sample)
        cursor = batch["samples"][-1]["sample_index"]
    for reader in reads:
        assert all(expected[index] == sample for index, sample in reader.items())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("scanner", type=Path)
    parser.add_argument("reader_dll", type=Path)
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("This test requires Windows for the named shared-memory feed.")
    exe, dll = args.scanner.resolve(strict=True), args.reader_dll.resolve(strict=True)
    root = args.directory.resolve()
    root.mkdir(parents=True)
    check_history_boundaries(dll, root)
    name = f"dotnet-{uuid4().hex}"
    port = free_port()
    profile = root / "profile.toml"
    source = PROFILE.with_name("mapped-loopback.toml").read_text(encoding="utf-8")
    profile.write_text(source.replace("rpi_us = 1000", "rpi_us = 8000") + f"""

[app]
auto_reconnect = true
[app.integration]
http_port = {port}
shared_memory = "{name}"
""", encoding="utf-8")
    with ExitStack() as stack:
        readers = {}
        for transport, target, count in (
            ("http", str(port), SAMPLES), ("memory", name, SAMPLES), ("history", str(port), HISTORY_SAMPLES)
        ):
            reader = stack.enter_context(process(["dotnet", dll, transport, target, str(count)]))
            assert reader.stderr.readline().strip() == f"Waiting for {transport} input."
            readers[transport] = reader
        time.sleep(0.05)
        fixture = adapter(stack, root, "source", "127.0.0.1", "recover")
        scanner = stack.enter_context(process([exe, "serve", profile, "0:12:1:305419896", "6"]))
        http = collect(readers["http"], root, "http")
        memory = collect(readers["memory"], root, "memory")
        history = collect(readers["history"], root, "history")
        verify_history(port, (http, memory, history))
        stdout, stderr = scanner.communicate(timeout=8)
        assert scanner.returncode == 0, (stdout, stderr)
        peer = fixture_report(fixture)

    (root / "serve.toml").write_text(stdout, encoding="utf-8")
    report = tomllib.loads(stdout)
    assert [run["state"] for run in report["attempts"]] == ["Timed out", "Closed"]
    assert all(run["forward_close"] == {"outcome": "confirmed"} for run in report["attempts"])
    assert all(run["forward_close"] for run in peer["attempts"])
    assert report["sequence_gaps"] == report["integration"]["queue_drops"] == 0
    print(f"C# HTTP and shared-memory readers each followed {SAMPLES} live values across a reconnect.")
    print(f"C# history read {HISTORY_SAMPLES} consecutive indexes across the same reconnect.")
    print(f"All {2 * SAMPLES + HISTORY_SAMPLES} outputs matched the scanner's retained packets, timestamps, and payloads.")


if __name__ == "__main__":
    main()
