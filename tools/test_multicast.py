"""Verify Class 1 multicast input and unicast heartbeat on Windows loopback."""

import argparse
from pathlib import Path
import socket
import subprocess
import sys
import tomllib

from loopback_adapter import MULTICAST_GROUP
from test_cli import NO_WINDOW, PROFILE, check_capture_window, parts_for, rows, with_adapter


def exclusive_udp():
    sock = socket.socket(type=socket.SOCK_DGRAM)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_EXCLUSIVEADDRUSE, 1)
    return sock


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("exe", type=Path)
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("This fixture checks Windows multicast socket binding")
    exe, root = args.exe.resolve(strict=True), args.directory.resolve()
    root.mkdir(parents=True, exist_ok=False)
    source = "input_multicast = true\n" + PROFILE.read_text(encoding="utf-8")
    source = source.replace("rpi_us = 1000", "rpi_us = 8000")
    profile = root / "multicast.toml"
    profile.write_text(source, encoding="utf-8")

    eds = root / "multicast.eds"
    eds.write_text(PROFILE.with_suffix(".eds").read_text(encoding="utf-8")
                   .replace("0x22440305", "0x22240305"), encoding="utf-8")
    checked = subprocess.run([exe, "validate", profile, eds, "Connection1"], capture_output=True,
                             encoding="utf-8", check=True, timeout=5, creationflags=NO_WINDOW)
    offline = tomllib.loads(checked.stdout)
    assert offline["connection_valid"] and offline["input_multicast"]
    origins = {entry["field"]: entry for entry in offline["origins"]}
    assert origins["T->O distribution"]["state"] == "eds"

    def capture(case, port, settings=profile, mode="multicast"):
        command = [exe, "capture", settings, root / case, "1", "0:12:1"]
        result, peer = with_adapter(root, case, command, mode,
                                    fixture_args=("--multicast-port", str(port)))
        report = tomllib.loads(result.stdout)
        assert report["input_multicast"]
        assert report["forward_close"] == {"outcome": "confirmed"} and peer["forward_close"]
        assert report["actual_ot_us"] == report["actual_to_us"] == 8000
        return result.returncode, report, peer

    for same_port in (False, True):
        with exclusive_udp() as reserved:
            reserved.bind(("127.0.0.1", 0))
            port = reserved.getsockname()[1]
        settings = profile
        if same_port:
            settings = root / "fixed-source.toml"
            settings.write_text(source.replace("local_udp_port = 0", f"local_udp_port = {port}"),
                                encoding="utf-8")
        code, report, peer = capture("same-port" if same_port else "separate-port", port, settings)
        assert code == 0, report
        endpoints = report["cyclic_io"]
        assert endpoints == peer["cyclic_io"]
        assert endpoints["input_multicast_destination"] == f"{MULTICAST_GROUP}:{port}"
        assert endpoints["input_local_udp"] == f"127.0.0.1:{port}"
        assert (endpoints["local_udp"] == endpoints["input_local_udp"]) == same_port
        assert peer["requested_input_port"] == (port if same_port else 2222)
        assert endpoints["to_connection_id"] == 0xBCDE8765
        assert peer["ot_received"] > 0 and peer["ot_invalid"] == 0
        assert report["invalid"] == 1 and report["sequence_gaps"] == 0
        samples = list(rows(parts_for(Path(report["csv_prefix"]))))
        check_capture_window(report, samples)
        assert all(int(row[-1]) == int(row[4]) // 16 for row in samples)
        with exclusive_udp() as probe:
            probe.bind(("127.0.0.1", port))

    for mode, detail in (
        ("multicast-missing", "omitted the T->O multicast socket address"),
        ("multicast-unicast", "expected a multicast group and nonzero port"),
    ):
        code, report, _ = capture(mode, 2222, mode=mode)
        assert code == 1 and detail in report["network_error"], report
        assert report["sent"] == report["written"] == 0

    with exclusive_udp() as occupied:
        occupied.bind(("127.0.0.1", 0))
        port = occupied.getsockname()[1]
        code, report, peer = capture("port-in-use", port)
        assert code == 1 and f"Bind multicast input on 127.0.0.1:{port}" in report["network_error"], report
        assert report["sent"] == report["written"] == peer["ot_received"] == 0
    print("Multicast receive, unicast heartbeat, same-port reuse and close on setup failures passed at 8 ms.")


if __name__ == "__main__":
    main()
