"""Check ListIdentity, explicit reads and keyed Large Forward_Open on loopback."""

import argparse
import csv
import json
from pathlib import Path
import socket
import struct
import subprocess
import tomllib

from loopback_adapter import sockaddr
from test_cli import NO_WINDOW, PROFILE, with_adapter

LOCAL = "127.0.0.1"


def discover(exe):
    # The fixture sends independently encoded ListIdentity items and bad datagrams.
    with socket.socket(type=socket.SOCK_DGRAM) as server:
        server.bind((LOCAL, 44818))
        server.settimeout(4)
        with subprocess.Popen([exe, "discover", LOCAL, LOCAL], stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, text=True, creationflags=NO_WINDOW) as client:
            try:
                request, address = server.recvfrom(128)
                command, length, session, status, context, options = struct.unpack("<HHII8sI", request)
                assert address[0] == LOCAL and command == 0x63 and length == session == status == options == 0
                name = b"Loopback Adapter"
                item = struct.pack("<H", 1) + sockaddr(LOCAL, 44818)
                item += struct.pack("<HHHBBHIB", 0, 12, 1, 1, 0, 0x60, 0x12345678, len(name))
                item += name + b"\x03"
                payload = struct.pack("<HHH", 1, 0x0C, len(item)) + item
                def frame(ctx):
                    return struct.pack("<HHII8sI", 0x63, len(payload), 0, 0, ctx, 0) + payload
                valid = frame(context)
                server.sendto(valid[:-1], address)
                server.sendto(frame(bytes([context[0] ^ 1]) + context[1:]), address)
                server.sendto(valid, address)
                server.sendto(valid, address)
                stdout, stderr = client.communicate(timeout=5)
                assert client.returncode == 0, stderr
            finally:
                if client.poll() is None:
                    client.kill()
                    client.wait()
    report = json.loads(stdout)
    assert report["invalid_replies"] == 2 and not report["limit_reached"]
    device, = report["devices"]
    assert device["source"] == device["advertised_endpoint"] == f"{LOCAL}:44818"
    assert device["identity"]["serial"] == 0x12345678 and device["identity"]["name"] == name.decode()
    assert device["state"] == 3 and device["protocol_version"] == 1
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("exe", type=Path)
    parser.add_argument("results", type=Path)
    args = parser.parse_args()
    exe, root = args.exe.resolve(strict=True), args.results.resolve()
    root.mkdir(parents=True, exist_ok=False)
    (root / "discovery.json").write_text(json.dumps(discover(exe), indent=2), encoding="utf-8")

    eds = root / "utf16.eds"
    eds.write_text(
        PROFILE.with_suffix(".eds").read_text(encoding="utf-8")
        .replace("Param1,,Assem101", "Param1,600,Assem101")
        .replace("20 04 24 01 2C FE 2C 65", "34 04 01 00 0C 00 01 00 01 00 21 00 04 00 24 01 2C FE 2C 65"),
        encoding="utf-16",
    )
    imported = subprocess.run([exe, "eds", eds], capture_output=True, text=True,
                              timeout=5, creationflags=NO_WINDOW, check=True)
    eds_profile, = tomllib.loads(imported.stdout)["profiles"]
    assert eds_profile["supported"] and eds_profile["fields"]["Electronic key"] == "1:12:1 1.0"
    assert eds_profile["fields"]["Large Forward_Open"] == "Required by imported connection size"

    def run(case, *command, mode="explicit"):
        result, peer = with_adapter(root, case, [exe, *command], mode)
        assert result.returncode == 0, (result.stdout, result.stderr)
        return result.stdout, peer

    identity, _ = run("identity-fallback", "identity", LOCAL, LOCAL)
    assert tomllib.loads(identity)["identity"]["serial"] == 0x12345678
    read, _ = run("wide-object-path", "read", LOCAL, LOCAL, "0x1234", "0x12345678", "7")
    assert json.loads(read)["data_hex"] == "CA FE"
    all_attrs, _ = run("all-attributes", "read", LOCAL, LOCAL, "1", "1", "all", mode="normal")
    assert bytes.fromhex(json.loads(all_attrs)["data_hex"])[15:] == b"Loopback Adapter"
    rejected, _ = with_adapter(root, "rejected-read",
                               [exe, "read", LOCAL, LOCAL, "1", "1", "0x7f"], "explicit")
    assert rejected.returncode == 1
    failure = json.loads(rejected.stdout)
    assert failure["general_status"] == 0x14 and failure["extended_status"] == [0x0302]

    saved = root / "saved.toml"
    source = PROFILE.read_text(encoding="utf-8")
    saved.write_text(source + '\n[[app.cip_reads]]\nname = "Serial number"\n'
                     '[app.cip_reads.path]\nclass = 1\ninstance = 1\nattribute = 6\n', encoding="utf-8")
    read, peer = run("saved-read", "read-saved", saved, "Serial number")
    assert json.loads(read)["data_hex"] == "78 56 34 12" and not peer["forward_close"]
    assert json.loads(read)["decoded"] == "0x12345678"

    route_cmd = [exe, "read", LOCAL, LOCAL, "1", "1"]
    routed, peer = run("routed-single", *route_cmd[1:], "6", "--route", "1/3", mode="routed")
    document = json.loads(routed)
    assert document["decoded"] == "0x12345678" and document["reply_from"] == "target"
    assert document["route"] == "1/3" and document["bridge"] == LOCAL
    assert peer["route_hex"] == "0103" and not peer["forward_close"]
    routed, peer = run("routed-multi-hop", *route_cmd[1:], "all", "--route",
                       "1/2/2/192.0.2.100/1/0", mode="routed")
    assert bytes.fromhex(json.loads(routed)["data_hex"])[15:] == b"Loopback Adapter"
    assert bytes.fromhex(peer["route_hex"]) == b"\x01\x02\x12\x0B192.0.2.100\x00\x01\x00"
    for case, route, attr, layer, status, extended in (
        ("route-rejected", "1/7", "6", "router", 1, 0x0311),
        ("routed-target-rejected", "1/3", "0x7f", "target", 0x14, 0x0302),
    ):
        result, _ = with_adapter(root, case, [*route_cmd, attr, "--route", route], "routed")
        assert result.returncode == 1, (result.stdout, result.stderr)
        error = json.loads(result.stdout)
        assert error["reply_from"] == layer and error["general_status"] == status
        assert error["extended_status"] == [extended]
    malformed, _ = with_adapter(root, "route-no-target", [*route_cmd, "6", "--route", "1/5"], "routed")
    assert malformed.returncode == 1 and "omitted the target service" in malformed.stderr
    for route in ("1", "1/999", "1/3/", "15/1", "2/239.0.0.1", "1/1/" * 8 + "1/1"):
        result = subprocess.run([*route_cmd, "6", "--route", route], capture_output=True,
                                text=True, timeout=4, creationflags=NO_WINDOW)
        assert result.returncode == 1 and any(
            text in result.stderr for text in ("CIP route", "CIP links", "CIP port")
        ), (route, result.stdout, result.stderr)

    routed_profile = root / "routed.toml"
    saved_source = saved.read_text(encoding="utf-8")
    routed_profile.write_text(saved_source.replace("[app.cip_reads.path]", 'route = "1/3"\n[app.cip_reads.path]'),
                              encoding="utf-8")
    routed, _ = run("routed-saved", "read-saved", routed_profile, "Serial number", mode="routed")
    assert json.loads(routed)["route"] == "1/3" and json.loads(routed)["decoded"] == "0x12345678"

    profile = root / "keyed-large.toml"
    profile.write_text(
        ("large_forward_open = true\n" + source).replace("input_size = 4", "input_size = 600")
        .replace("= 1000", "= 8000").replace("offset = 0", "offset = 596")
        + "\n[electronic_key]\nvendor = 1\ndevice_type = 12\nproduct = 1\nmajor = 1\nminor = 0\n",
        encoding="utf-8",
    )
    capture, peer = run("keyed-large", "capture", profile, root / "capture", "1", "1:12:1",
                        mode="large-keyed")
    result = tomllib.loads(capture)
    assert result["forward_close"] == {"outcome": "confirmed"} and peer["forward_close"]
    assert result["actual_to_us"] == result["actual_ot_us"] == 8000 and peer["ot_invalid"] == 0
    assert result["buffer_overruns"] == 0 and result["sequence_gaps"] == 0
    files = list((root / "capture").glob("*.csv"))
    assert len(files) == 1
    with files[0].open(newline="", encoding="utf-8") as stream:
        rows = list(csv.DictReader(stream))
    assert len(rows) == result["written"] > 0
    assert all(int(row["value"]) == 0xA1B2C3D4 for row in rows)
    print(f"Discovery, direct and routed CIP reads, identity fallback and keyed Large Forward_Open passed ({len(rows)} CSV rows).")


if __name__ == "__main__":
    main()
