"""Verify direct and routed Class 1 multicast input and unicast heartbeat on Windows loopback."""

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

    def capture(case, port, settings, mode):
        config = tomllib.loads(settings.read_text(encoding="utf-8"))
        bridge = config["device_ip"]
        vendor = 1 if config.get("large_forward_open") else 0
        serial = 0x12345679 if "io_route" in config else 0x12345678
        command = [exe, "capture", settings, root / case, "1", f"{vendor}:12:1:{serial}"]
        result, peer = with_adapter(root, case, command, mode,
                                    fixture_args=("--address", bridge, "--multicast-port", str(port)))
        report = tomllib.loads(result.stdout)
        assert report["input_multicast"]
        assert report["forward_close"] == {"outcome": "confirmed"} and peer["forward_close"]
        assert report["actual_ot_us"] == report["actual_to_us"] == 8000
        if route := config.get("io_route"):
            assert report["io_route"] == route and report["identity"]["serial"] == serial
            assert report["identity"]["name"] == "Loopback Routed Target"
            common_path = "200424012cfe2c65"
            if vendor:
                route_key = "010212093132372e302e302e33000100340401000c0001000100"
                assert peer["routed_identity_attributes"] == ["all", *range(1, 8)]
                assert peer["open_service"] == 0x5B
            else:
                route_key = "0103"
                assert peer["routed_identity_attributes"] == ["all"]
                assert peer["open_service"] == 0x54
            assert peer["open_path_hex"] == peer["close_path_hex"] == route_key + common_path
        return result.returncode, report, peer

    def check_capture(mode, settings, same_port):
        with exclusive_udp() as reserved:
            reserved.bind(("127.0.0.1", 0))
            port = reserved.getsockname()[1]
        case = f"{mode}-{'same-port' if same_port else 'separate-port'}"
        if same_port:
            fixed = root / f"{case}.toml"
            fixed.write_text(settings.read_text(encoding="utf-8").replace(
                "local_udp_port = 0", f"local_udp_port = {port}"), encoding="utf-8")
            settings = fixed
        code, report, peer = capture(case, port, settings, mode)
        assert code == 0, report
        endpoints = report["cyclic_io"]
        assert endpoints == peer["cyclic_io"]
        assert endpoints["input_multicast_destination"] == f"{MULTICAST_GROUP}:{port}"
        assert endpoints["input_local_udp"] == f"127.0.0.1:{port}"
        assert (endpoints["local_udp"] == endpoints["input_local_udp"]) == same_port
        assert peer["requested_input_port"] == (port if same_port else 2222)
        assert endpoints["to_connection_id"] == 0xBCDE8765
        assert peer["ot_received"] > 0 and peer["ot_invalid"] == 0
        expected_invalid = 2 if mode.startswith("routed-") else 1
        assert report["invalid"] == expected_invalid and report["sequence_gaps"] == 0, report
        samples = list(rows(parts_for(Path(report["csv_prefix"]))))
        check_capture_window(report, samples)
        assert samples
        if mode.endswith("-large"):
            assert all(int(row[-1]) == 0xA1B2C3D4 for row in samples)
        else:
            assert all(int(row[-1]) == peer["value_base"] + int(row[4]) // 16 for row in samples)
        with exclusive_udp() as probe:
            probe.bind(("127.0.0.1", port))
        print(f"{case}: {len(samples)} rows at 8 ms, rejected datagrams: {expected_invalid}, confirmed close.")

    routed_profile = root / "routed-multicast.toml"
    bridge_source = source.replace('device_ip = "127.0.0.1"', 'device_ip = "127.0.0.2"')
    routed_profile.write_text('io_route = "1/3"\n' + bridge_source, encoding="utf-8")
    large = root / "keyed-large-routed-multicast.toml"
    large.write_text('io_route = "1/2/2/127.0.0.3/1/0"\nlarge_forward_open = true\n'
                     + bridge_source.replace("input_size = 4", "input_size = 600")
                     .replace("offset = 0", "offset = 596")
                     + "\n[electronic_key]\nvendor = 1\ndevice_type = 12\nproduct = 1\nmajor = 1\nminor = 0\n",
                     encoding="utf-8")
    for mode, settings in (("multicast", profile), ("routed-io-multicast", routed_profile)):
        for same_port in (False, True):
            check_capture(mode, settings, same_port)
        for suffix, detail in (
            ("missing", "omitted the T->O multicast socket address"),
            ("unicast", "expected a multicast group and nonzero port"),
        ):
            failed_mode = f"{mode}-{suffix}"
            code, report, _ = capture(failed_mode, 2222, settings, failed_mode)
            assert code == 1 and detail in report["network_error"], report
            assert report["sent"] == report["written"] == 0

        with exclusive_udp() as occupied:
            occupied.bind(("127.0.0.1", 0))
            port = occupied.getsockname()[1]
            code, report, peer = capture(f"{mode}-port-in-use", port, settings, mode)
            assert code == 1 and f"Bind multicast input on 127.0.0.1:{port}" in report["network_error"], report
            assert report["sent"] == report["written"] == peer["ot_received"] == 0
    check_capture("routed-io-multicast-large", large, False)
    print("Direct and routed multicast input, unicast heartbeat, socket ownership, and cleanup passed.")


if __name__ == "__main__":
    main()
