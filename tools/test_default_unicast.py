"""Verify default unicast UDP endpoints and Forward_Open CPF on Windows loopback."""
import argparse
from pathlib import Path
import subprocess
import sys
import tomllib

from test_cli import NO_WINDOW, PROFILE, check_capture_window, parts_for, rows, with_adapter
from test_multicast import exclusive_udp

SCANNER = "127.0.0.1"
ADAPTER = "127.0.0.2"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("exe", type=Path)
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("This fixture checks Windows exclusive binding")
    exe, root = args.exe.resolve(strict=True), args.directory.resolve()
    root.mkdir(parents=True)
    source = "default_unicast_port = true\n" + PROFILE.read_text(encoding="utf-8")
    source = source.replace('device_ip = "127.0.0.1"', f'device_ip = "{ADAPTER}"').replace("= 1000", "= 8000")
    profile = root / "default-unicast.toml"
    profile.write_text(source, encoding="utf-8")
    large = root / "routed-large-default-unicast.toml"
    route = "1/2/2/127.0.0.3/1/0"
    large.write_text(f'io_route = "{route}"\nlarge_forward_open = true\n'
                     + source.replace("input_size = 4", "input_size = 600").replace("offset = 0", "offset = 596")
                     .replace("local_udp_port = 0", "local_udp_port = 2222")
                     + "\n[electronic_key]\nvendor = 1\ndevice_type = 12\nproduct = 1\nmajor = 1\nminor = 0\n",
                     encoding="utf-8")

    for invalid, content in (
        ("multicast", "input_multicast = true\n" + source),
        ("different-port", source.replace("local_udp_port = 0", "local_udp_port = 33333")),
    ):
        path = root / f"invalid-{invalid}.toml"
        path.write_text(content, encoding="utf-8")
        result = subprocess.run([exe, "validate", path], capture_output=True, text=True, timeout=5,
                                creationflags=NO_WINDOW)
        report = tomllib.loads(result.stdout)
        assert result.returncode == 1 and not report["connection_valid"]
        assert any("Default unicast UDP 2222" in message for message in report["connection_errors"])

    for mode, settings, vendor in (
        ("default-unicast", profile, 0), ("routed-io-large-default-unicast", large, 1),
    ):
        offline = subprocess.run([exe, "validate", settings], capture_output=True, text=True, check=True,
                                 timeout=5, creationflags=NO_WINDOW)
        checked = tomllib.loads(offline.stdout)
        assert checked["connection_valid"] and checked["default_unicast_port"]
        assert checked["local_udp_port"] == 2222
        result, peer = with_adapter(
            root, mode, [exe, "capture", settings, root / mode, "1", f"{vendor}:12:1:{0x12345679}"],
            mode, fixture_args=("--address", ADAPTER),
        )
        report = tomllib.loads(result.stdout)
        assert result.returncode == 0, (report, result.stderr)
        assert report["default_unicast_port"] and report["duration_reached"]
        assert report["actual_ot_us"] == report["actual_to_us"] == 8000
        assert report["forward_close"] == {"outcome": "confirmed"} and peer["forward_close"]
        assert peer["input_sockaddr_omitted"] and peer["output_sockaddr_omitted"] == (vendor == 0)
        endpoints = report["cyclic_io"]
        assert endpoints == peer["cyclic_io"] and endpoints["local_udp"] == f"{SCANNER}:2222"
        assert endpoints["output_destination"].startswith(ADAPTER + ":")
        assert endpoints["output_destination"].endswith(":2222") == (vendor == 0)
        assert peer["ot_received"] > 0 and peer["ot_invalid"] == 0
        assert report["invalid"] == report["sequence_gaps"] == report["buffer_overruns"] == 0
        samples = list(rows(parts_for(Path(report["csv_prefix"]))))
        check_capture_window(report, samples)
        assert samples
        if vendor:
            routed_path = "010212093132372e302e302e33000100340401000c0001000100200424012cfe2c65"
            assert report["io_route"] == route and peer["open_service"] == 0x5B
            assert peer["open_path_hex"] == peer["close_path_hex"] == routed_path
            assert all(int(row[-1]) == 0xA1B2C3D4 for row in samples)
        else:
            assert all(int(row[-1]) == peer["value_base"] + int(row[4]) // 16 for row in samples)
        print(f"{mode}: {len(samples)} rows at 8 ms, exact endpoints and confirmed close.")

    with exclusive_udp() as occupied:
        occupied.bind((SCANNER, 2222))
        result, peer = with_adapter(
            root, "port-occupied",
            [exe, "capture", profile, root / "port-occupied", "1", f"0:12:1:{0x12345679}"],
            "default-unicast", fixture_args=("--address", ADAPTER),
        )
        report = tomllib.loads(result.stdout)
        assert result.returncode == 1 and "Bind cyclic I/O on 127.0.0.1:2222" in report["network_error"]
        assert report["sent"] == report["written"] == 0 and "actual_to_us" not in report
        assert "input_sockaddr_omitted" not in peer and not peer["forward_close"]
    print("Occupied UDP 2222 was reported before Forward_Open.")


if __name__ == "__main__":
    main()
