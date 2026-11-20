"""Verify offline qualification review with synthetic GUI, timeout, and incomplete evidence."""

import argparse
import csv
import json
from pathlib import Path
import subprocess
import sys

from join_capture import METADATA

HERE = Path(__file__).resolve().parent
SERIAL = 305419896


def fixture(folder, index, rpi):
    folder.mkdir()
    prefix = folder / "capture"
    start = f"2026-01-01T00:00:0{index}.000000Z"
    elapsed = range(0, 1_000_000, rpi)
    for part, samples in enumerate((elapsed[:20], elapsed[20:]), 1):
        with prefix.with_name(f"capture-{part:06}.csv").open("x", encoding="utf-8", newline="") as file:
            writer = csv.writer(file, lineterminator="\r\n")
            writer.writerow(METADATA + ["value"])
            for stamp in samples:
                sample = stamp // rpi
                writer.writerow([start, stamp, sample, 1, sample + 1, sample + 1, "0"])
    connection = f'''
generation = 1
state = "Closed"
adapter_ip = "127.0.0.1"
local_ip = "127.0.0.1"
input_assembly = 101
input_bytes = 4
output_assembly = 254
input_run_idle = false
output_run_idle = false
input_only = true
output_bytes = 0
configuration_bytes = 0
schedule = "deadline"
requested_ot_us = {rpi}
requested_to_us = {rpi}
actual_ot_us = {rpi}
actual_to_us = {rpi}
accepted = {len(elapsed) + 5}
sequence_gaps = 0
duplicates = 0
late = 0
invalid = 0
timeouts = 0
skipped_deadlines = 2
forward_close = {{ outcome = "confirmed" }}
receive_spacing = {{ intervals = 1, p99_us = {rpi + 50}, max_us = {rpi + 80} }}
send_spacing = {{ intervals = 1, p99_us = {rpi + 10}, max_us = {rpi + 20} }}
connection_timeout = {{ multiplier_code = 3 }}
identity = {{ vendor = 0, device_type = 12, code = 1, serial = {SERIAL}, major = 1, minor = 0 }}
'''
    report = folder / "capture-report.toml"
    report.write_text(
        f'''mode = "gui"
version = "0.1.1"
[capture]
csv_prefix = {json.dumps(str(prefix))}
started_utc = "{start}"
mode = "every_packet"
duration_seconds = 1
duration_reached = true
csv_parts = 2
seen = {len(elapsed)}
written = {len(elapsed)}
after_capture_deadline = 3
unchanged = 0
buffer_overruns = 0
unconfirmed_rows = 0
[connection]{connection}
[[attempts]]{connection}''',
        encoding="utf-8",
    )
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    root = parser.parse_args().directory.resolve()
    root.mkdir()
    reports = [fixture(root / f"run-{index}", index, rpi) for index, rpi in enumerate((8000, 1000, 1000, 1000))]
    options = ["--link-path", "loopback", "--loopback-seconds", "1", "--expected", f"0:12:1:{SERIAL}"]
    command = [sys.executable, HERE / "review_gui_qualification.py", *options]

    def audit(paths, status):
        cli = subprocess.run([*command, *paths], capture_output=True, text=True, timeout=8,
                             creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        assert cli.returncode == status, (cli.stdout, cli.stderr)
        return json.loads(cli.stdout)

    report = audit(reports, 0)
    assert report["source"] == "saved_gui_reports" and report["link_path"] == "loopback"
    assert report["outcome"] == "passed"
    baseline, first, _, _ = report["runs"]
    assert not any(run["errors"] for run in report["runs"])
    assert baseline["timing"]["csv"]["rows"] == 125 and first["timing"]["csv"]["rows"] == 1000
    assert first["timing"]["socket"]["accepted"] == 1005
    assert first["timing"]["socket"]["skipped_deadlines"] == 2
    assert first["timing"]["socket"]["receive_spacing"]["p99_us"] == 1050
    assert first["timing"]["csv"]["generations"][0]["spacing"]["p99_us"] == 1000

    repeated = audit([*reports[:3], reports[2]], 1)
    assert any("already used" in error for error in repeated["runs"][3]["errors"])
    candidate = reports[1].read_text(encoding="utf-8")
    reports[1].write_text(
        candidate.replace("actual_to_us = 1000", "actual_to_us = 1500")
        .replace("timeouts = 0", "timeouts = 1")
        .replace(f"serial = {SERIAL}", "serial = 1"),
        encoding="utf-8",
    )
    failed = audit(reports, 1)
    errors = failed["runs"][1]["errors"]
    assert any("Forward_Open" in error for error in errors)
    assert any("timeouts" in error for error in errors)
    assert any("identity differs" in error for error in errors)
    assert not failed["runs"][2]["errors"]

    missing_part = reports[2].parent / "capture-000002.csv"
    incomplete = missing_part.with_suffix(".fixture")
    missing_part.rename(incomplete)
    failed = audit(reports, 1)
    assert any("CSV parts" in error for error in failed["runs"][2]["errors"])
    assert not failed["runs"][3]["errors"]
    print("GUI qualification review verified saved timing, missing parts, identity, RPI, timeout and repeated evidence.")


if __name__ == "__main__":
    main()
