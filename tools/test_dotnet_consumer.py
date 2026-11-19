"""Check the C# consumers against a real Windows scanner and a loopback adapter."""

import argparse
from contextlib import ExitStack
import json
from pathlib import Path
import sys
import time
import tomllib
from uuid import uuid4

from test_integration import free_port, get
from test_recovery import PROFILE, adapter, fixture_report, process

SAMPLES = 60


def frame(sample):
    return (sample["stream_start_utc_us"], sample["generation"], sample["packet_sequence"],
            sample["cip_sequence"], sample["elapsed_us"], bytes.fromhex(sample["payload_hex"]))


def collect(reader, root, transport):
    stdout, stderr = reader.communicate(timeout=10)
    assert reader.returncode == 0, (stdout, stderr)
    (root / f"{transport}.jsonl").write_text(stdout, encoding="utf-8")
    records = [json.loads(line) for line in stdout.splitlines()]
    assert len(records) == SAMPLES
    indexed = {}
    previous = 0
    for record in records:
        sample = record["sample"]
        index = sample["sample_index"]
        assert record["transport"] == transport
        assert record["active_generation"] == sample["generation"] in (1, 2)
        assert 0 <= record["sample_age_us"] <= 500_000
        assert index > previous
        assert record["skipped_export_indexes"] == (index - previous - 1 if previous else 0)
        assert sample["input_assembly"] == 101
        payload = bytes.fromhex(sample["payload_hex"])
        assert len(payload) == 4
        if transport == "http":
            assert set(sample["values"]) == {"counter", "low_bit", "swapped_word"}
            assert int(sample["values"]["counter"]) == int.from_bytes(payload, "little")
        else:
            assert sample["values"] is None
        indexed[index] = frame(sample)
        previous = index
    assert {frame[1] for frame in indexed.values()} == {1, 2}
    assert len({frame[0] for frame in indexed.values()}) == 1
    return indexed


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
        for transport, target in (("http", str(port)), ("memory", name)):
            reader = stack.enter_context(process(["dotnet", dll, transport, target, str(SAMPLES)]))
            assert reader.stderr.readline().strip() == f"Waiting for {transport} input."
            readers[transport] = reader
        time.sleep(0.05)
        fixture = adapter(stack, root, "source", "127.0.0.1", "recover")
        scanner = stack.enter_context(process([exe, "serve", profile, "0:12:1:305419896", "6"]))
        http = collect(readers["http"], root, "http")
        memory = collect(readers["memory"], root, "memory")
        verify_history(port, (http, memory))
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
    print(f"All {2 * SAMPLES} outputs matched the scanner's retained packets, timestamps, and payloads.")


if __name__ == "__main__":
    main()
