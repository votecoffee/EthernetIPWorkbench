"""Check discovery, explicit reads, and direct and routed Class 1 I/O on loopback."""

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


def check_routed_input(exe, root, source):
    bridge = "127.0.0.2"
    bridge_args = ("--address", bridge)
    serial = 0x12345679
    standard = root / "routed-input.toml"
    large = root / "routed-large-input.toml"
    setup = source.replace('device_ip = "127.0.0.1"', f'device_ip = "{bridge}"').replace("= 1000", "= 8000")
    standard.write_text('io_route = "1/3"\n' + setup, encoding="utf-8")
    multi_hop = "1/2/2/127.0.0.3/1/0"
    large.write_text(f'io_route = "{multi_hop}"\nlarge_forward_open = true\n'
                     + setup.replace("input_size = 4", "input_size = 600").replace("offset = 0", "offset = 596")
                     + "\n[electronic_key]\nvendor = 1\ndevice_type = 12\nproduct = 1\nmajor = 1\nminor = 0\n",
                     encoding="utf-8")
    identity, peer = with_adapter(root, "routed-identity", [exe, "identity", bridge, LOCAL, "--route", "1/3"],
                                  "routed-io", fixture_args=bridge_args)
    identity = tomllib.loads(identity.stdout)
    assert identity["io_route"] == "1/3" and identity["identity"]["serial"] == serial
    assert identity["identity"]["name"] == "Loopback Routed Target"
    assert peer["routed_identity_attributes"] == ["all"] and "open_path_hex" not in peer

    common_path = "200424012cfe2c65"
    for mode, profile, route, expected_path, vendor in (
        ("routed-io", standard, "1/3", "0103" + common_path, 0),
        ("routed-io-large", large, multi_hop, (b"\x01\x02\x12\x09127.0.0.3\x00\x01\x00".hex()
                                               + "340401000c0001000100" + common_path), 1),
    ):
        output = root / mode
        result, peer = with_adapter(root, mode, [exe, "capture", profile, output, "1", f"{vendor}:12:1:{serial}"],
                                    mode, fixture_args=bridge_args)
        assert result.returncode == 0, (result.stdout, result.stderr)
        capture = tomllib.loads(result.stdout)
        assert capture["io_route"] == route and capture["identity"]["serial"] == serial
        assert capture["duration_reached"] and capture["forward_close"] == {"outcome": "confirmed"}
        assert capture["actual_ot_us"] == capture["actual_to_us"] == 8000
        assert capture["sequence_gaps"] == capture["buffer_overruns"] == peer["ot_invalid"] == 0
        assert capture["cyclic_io"]["output_destination"].startswith(bridge + ":")
        assert capture["cyclic_io"]["first_input_source"].startswith(bridge + ":")
        assert peer["open_path_hex"] == peer["close_path_hex"] == expected_path
        csv_path, = output.glob("*.csv")
        with csv_path.open(newline="", encoding="utf-8") as stream:
            values = [int(row["value"]) for row in csv.DictReader(stream)]
        assert len(values) == capture["written"] > 0
        if vendor == 1:
            assert set(values) == {0xA1B2C3D4}
        else:
            assert values[0] == peer["value_base"] and values[-1] > values[0]
        print(f"{mode}: {len(values)} samples at 8 ms, matching open and close paths, no sequence gaps")

    for case, mode, expected, message in (
        ("routed-replacement", "routed-io", f"0:12:1:{serial + 1}", "Identity mismatch"),
        ("routed-no-target", "routed-io-rejected-route", f"0:12:1:{serial}", "CIP router rejected"),
        ("routed-unexpected-peer", "routed-io-unexpected-peer", f"0:12:1:{serial}", "configured cyclic peer"),
    ):
        failed, peer = with_adapter(root, case, [exe, "capture", standard, root / case, "1", expected],
                                    mode, fixture_args=bridge_args)
        assert failed.returncode == 1
        report = tomllib.loads(failed.stdout)
        assert report["sent"] == report["written"] == 0 and "cyclic_io" not in report
        if mode == "routed-io-unexpected-peer":
            assert report["forward_close"] == {"outcome": "confirmed"}
            assert message in report["network_error"] and peer["forward_close"]
        else:
            assert message in report["identity_error"]
            assert "open_path_hex" not in peer and not peer["forward_close"]

    for name, invalid in (
        ("invalid-route", standard.read_text(encoding="utf-8").replace("1/3", "1/")),
        ("routed-multicast-owner-listen-only", "listen_only = true\ninput_multicast = true\n"
         + standard.read_text(encoding="utf-8").replace("input_only = true", "input_only = false")),
        ("routed-unicast-listener", "listen_only = true\n" + standard.read_text(encoding="utf-8")),
        ("routed-input-run-idle", standard.read_text(encoding="utf-8").replace(
            "output_run_idle = false", "output_run_idle = true")),
    ):
        profile = root / f"{name}.toml"
        profile.write_text(invalid, encoding="utf-8")
        result = subprocess.run([exe, "validate", profile], capture_output=True, text=True, timeout=4,
                                creationflags=NO_WINDOW)
        assert result.returncode == 1 and not tomllib.loads(result.stdout)["connection_valid"]


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


def check_class_info(exe, root):
    for case, mode, route, exit_code in (
        ("direct", "class-info", None, 0),
        ("routed", "class-info", "1/3", 0),
        ("router-rejected", "class-info", "1/7", 1),
        ("bad-list", "class-info-malformed", None, 1),
    ):
        command = [exe, "class-info", LOCAL, LOCAL, "0x01"]
        if route:
            command.extend(("--route", route))
        result, peer = with_adapter(root, f"class-info-{case}", command, mode)
        assert result.returncode == exit_code and not result.stderr, (result.stdout, result.stderr)
        report = json.loads(result.stdout)
        (root / f"class-info-{case}.json").write_text(result.stdout, encoding="utf-8")
        assert report["class"] == 1 and report.get("route") == route
        replies = report["results"]
        assert [row["path"] for row in replies] == [
            {"class": 1, "instance": 0, "attribute": idx} for idx in range(1, 8)
        ]
        assert peer["class_attributes"] == list(range(1, 8))
        assert peer["registrations"] == 1 and peer["unregistered"] and not peer["forward_close"]
        if case == "router-rejected":
            assert peer["route_hex"] == "0107"
            assert all(row["reply_from"] == "router" and row["extended_status"] == [0x0311]
                       and row["status"] == "cip_rejected" for row in replies)
            continue
        assert [replies[i]["decoded"] for i in (0, 1, 2, 5, 6)] == ["2", "1", "1", "7", "256"]
        if case == "bad-list":
            assert replies[3]["status"] == "success"
            assert replies[3]["decoded"] is None and replies[3]["data_hex"] == "02 00 03 00"
            assert "declares 2 IDs" in replies[3]["decode_error"]
        else:
            assert replies[3]["decoded"] == "[0x0003, 0x0100]"
        if route:
            assert peer["route_hex"] == "0103"
            assert all(row["bridge"] == LOCAL and row["reply_from"] == "target" for row in replies)
            assert replies[4]["status"] == "cip_rejected" and replies[4]["general_status"] == 0x14
            assert replies[4]["extended_status"] == [0x0302]
        else:
            assert replies[4]["decoded"] == "[0x0001, 0x000E]"

    result = subprocess.run([exe, "class-info", LOCAL, LOCAL, "0"], capture_output=True,
                            text=True, timeout=4, creationflags=NO_WINDOW)
    assert result.returncode == 1 and "CIP class and attribute IDs must be nonzero" in result.stderr


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
    eds_report = tomllib.loads(imported.stdout)
    assert eds_report["device"] == {
        "vendor": 0, "device_type": 12, "product_code": 1,
        "major": 1, "minor": 0, "product_name": "Loopback Adapter",
    }
    eds_profile, = eds_report["profiles"]
    assert eds_profile["supported"] and eds_profile["fields"]["Electronic key"] == "1:12:1 1.0"
    assert eds_profile["fields"]["Large Forward_Open"] == "Required by imported connection size"
    assert "input_layout" not in eds_profile

    compatible_eds = root / "compatible-key.eds"
    compatible_eds.write_text(eds.read_text(encoding="utf-16").replace(
        "34 04 01 00 0C 00 01 00 01 00", "34 04 01 00 0C 00 01 00 81 01"), encoding="utf-16")
    imported = subprocess.run([exe, "eds", compatible_eds], capture_output=True, text=True,
                              timeout=5, creationflags=NO_WINDOW, check=True)
    [compatible] = tomllib.loads(imported.stdout)["profiles"]
    assert compatible["supported"] and compatible["fields"]["Electronic key"] == "1:12:1 1.1 (compatible mode)"
    compatible_profile = root / "compatible-key.toml"
    profile_text = PROFILE.read_text(encoding="utf-8") + (
        '\n[electronic_key]\nvendor = 1\ndevice_type = 12\nproduct = 1\nmajor = 1\nminor = 1\ncompatible = true\n')
    compatible_profile.write_text(profile_text, encoding="utf-8")
    result = subprocess.run([exe, "validate", compatible_profile, compatible_eds, "Connection1"],
                            capture_output=True, text=True, timeout=5, creationflags=NO_WINDOW, check=True)
    checked = tomllib.loads(result.stdout)
    assert checked["electronic_key"] == {
        "vendor": 1, "device_type": 12, "product": 1, "major": 1, "minor": 1, "compatible": True,
    }
    assert next(item for item in checked["origins"] if item["field"] == "Electronic key")["state"] == "eds"
    invalid_key = root / "invalid-compatible-key.toml"
    invalid_key.write_text(profile_text.replace("minor = 1", "minor = 0"), encoding="utf-8")
    result = subprocess.run([exe, "validate", invalid_key], capture_output=True, text=True,
                            timeout=5, creationflags=NO_WINDOW)
    assert result.returncode == 1
    assert "nonzero device type, product code and minor revision" in tomllib.loads(result.stdout)["connection_errors"][0]

    typed_eds = root / "typed-input.eds"
    typed_eds.write_text(
        PROFILE.with_suffix(".eds").read_text(encoding="utf-8")
        .replace("[Assembly]", 'Param2 = 0,,,0,0xC3,2,"Net weight";\n'
                              'Param3 = 0,,,0,0xC1,,"Ready";\n[Assembly]')
        .replace("4, 0x0000,,,32,;", ", 0x0000,,,16,Assem103,,Param2;")
        .replace("[Connection Manager]", 'Assem103 = "Status", "", ,0,,,3,,1,Param3,4,;\n'
                                         '[Connection Manager]'),
        encoding="utf-8",
    )
    imported = subprocess.run([exe, "eds", typed_eds], capture_output=True, text=True,
                              timeout=5, creationflags=NO_WINDOW, check=True)
    typed_profile, = tomllib.loads(imported.stdout)["profiles"]
    assert "output_layout" not in typed_profile
    assert typed_profile["fields"]["T->O size"] == "4 bytes (Assem101 member widths)"
    assert typed_profile["assembly_definition_sizes"] == ["T->O size"]
    layout = typed_profile["input_layout"]
    assert layout["format"] == "Assem101" and layout["skipped_members"] == 0
    assert layout["fields"] == [
        {"name": "Ready", "selection": {"offset": 0, "kind": "Bool", "bit": 3, "big_endian": False}},
        {"name": "Net_weight", "selection": {"offset": 2, "kind": "I16", "big_endian": False}},
    ]
    preflight = subprocess.run([exe, "validate", PROFILE, typed_eds, typed_profile["id"]],
                               capture_output=True, text=True, timeout=5, creationflags=NO_WINDOW,
                               check=True)
    verified = tomllib.loads(preflight.stdout)
    assert verified["connection_valid"] and verified["input"]["payload_bytes"] == 4
    size = next(row for row in verified["origins"] if row["field"] == "T->O size")
    assert size["state"] == "assembly_definition" and size["eds_size_is_layout"]

    mapped = root / "typed-input.toml"
    source = PROFILE.read_text(encoding="utf-8")
    for field in layout["fields"]:
        source += f'\n[[input_fields]]\nname = "{field["name"]}"\n[input_fields.selection]\n'
        for key in ("kind", "offset", "bit"):
            if key in field["selection"]:
                source += f'{key} = {json.dumps(field["selection"][key])}\n'
    mapped.write_text(source.replace("= 1000", "= 8000"), encoding="utf-8")
    for payload, expected in [("08 00 D4 FE", "true"), ("F7 FF D4 FE", "false")]:
        preview = subprocess.run([exe, "decode-input", mapped, payload], capture_output=True, text=True,
                                 timeout=5, creationflags=NO_WINDOW, check=True)
        assert [field["value"] for field in tomllib.loads(preview.stdout)["fields"]] == [expected, "-300"]

    owner_eds = root / "typed-owner.eds"
    owner_eds.write_text(
        typed_eds.read_text(encoding="utf-8")
        .replace("[Connection Manager]", 'Assem102 = "Outputs", "", ,0x0001,,,16,Assem103,,Param2;\n'
                                         '[Connection Manager]')
        .replace("0x02010002", "0x04010002")
        .replace("0x22440305", "0x22440405")
        .replace("Param1,0,,", "Param1,,Assem102,")
        .replace("2C FE", "2C 66"),
        encoding="utf-8",
    )
    imported = subprocess.run([exe, "eds", owner_eds], capture_output=True, text=True,
                              timeout=5, creationflags=NO_WINDOW, check=True)
    owner_profile, = tomllib.loads(imported.stdout)["profiles"]
    assert owner_profile["supported"]
    assert owner_profile["fields"]["O->T header"] == "32-bit Run/Idle"
    assert owner_profile["output_layout"] == {
        "format": "Assem102", "fields": layout["fields"], "skipped_members": 0,
    }
    assert owner_profile["input_layout"] == layout

    configuration_eds = root / "typed-configuration.eds"
    configuration_eds.write_text(
        typed_eds.read_text(encoding="utf-8")
        .replace("[Assembly]", 'Param4 = 0,,,0x0010,0xC6,1,"Read only";\n[Assembly]')
        .replace("[Connection Manager]", 'Assem201 = "Configuration", "", 4,0,,,8,Param4,8,Assem103,,Param2;\n'
                                         '[Connection Manager]')
        .replace("    0,,", "    ,Assem201,"),
        encoding="utf-8",
    )
    imported = subprocess.run([exe, "eds", configuration_eds], capture_output=True, text=True,
                              timeout=5, creationflags=NO_WINDOW, check=True)
    configured_profile, = tomllib.loads(imported.stdout)["profiles"]
    assert configured_profile["fields"]["Config size"] == "4"
    configuration_layout = configured_profile["configuration_layout"]
    assert configuration_layout["format"] == "Assem201" and configuration_layout["skipped_members"] == 1
    assert configuration_layout["fields"] == [
        {"name": "Ready", "selection": {"offset": 1, "kind": "Bool", "bit": 3, "big_endian": False}},
        layout["fields"][1],
    ]
    configuration_toml = root / "staged-configuration.toml"
    configuration_toml.write_text(
        PROFILE.read_text(encoding="utf-8").replace("config_size = 0", "config_size = 4")
        .replace('config_hex = ""', 'config_hex = "A5 08 D4 FE"'),
        encoding="utf-8",
    )
    preflight = subprocess.run([exe, "validate", configuration_toml, configuration_eds, configured_profile["id"]],
                               capture_output=True, text=True, timeout=5, creationflags=NO_WINDOW, check=True)
    assert tomllib.loads(preflight.stdout)["configuration_bytes"] == 4

    def run(case, *command, mode="explicit"):
        result, peer = with_adapter(root, case, [exe, *command], mode)
        assert result.returncode == 0, (result.stdout, result.stderr)
        return result.stdout, peer

    capture_dir = root / "packed-bool-csv"
    receipt, peer = run("packed-bool", "capture", mapped, capture_dir, "2", "0:12:1", mode="normal")
    report = tomllib.loads(receipt)
    assert report["forward_close"] == {"outcome": "confirmed"} and peer["forward_close"]
    part, = capture_dir.glob("*.csv")
    with part.open(newline="", encoding="utf-8") as stream:
        rows = list(csv.DictReader(stream))
    assert len(rows) == report["written"] and report["buffer_overruns"] == 0
    assert {row["Ready"] for row in rows} == {"true", "false"}
    for row in rows:
        fixture_value = peer["value_base"] + int(row["packet_sequence"]) // 16
        assert row["Ready"] == ("true" if fixture_value & 8 else "false")

    identity, _ = run("identity-fallback", "identity", LOCAL, LOCAL)
    readback = tomllib.loads(identity)
    assert readback["identity"]["serial"] == 0x12345678 and "eds_comparison" not in readback
    identity, peer = run("identity-matching-eds", "identity", LOCAL, LOCAL, "--eds", typed_eds)
    assert tomllib.loads(identity)["eds_comparison"] == {"compared_fields": 5, "differences": []}
    assert not peer["forward_close"]
    mismatch_eds = root / "identity-only.eds"
    mismatch_eds.write_text("[Device]\nVendCode = 0x00;\nProdCode = 7;\nMinRev = 2;\n",
                            encoding="utf-16")
    route = "1/2/2/192.0.2.100/1/0"
    identity, peer = run("identity-routed-eds", "identity", LOCAL, LOCAL,
                         "--eds", mismatch_eds, "--route", route, mode="routed")
    readback = tomllib.loads(identity)
    assert readback["io_route"] == route and readback["identity"]["code"] == 1
    assert readback["eds_comparison"] == {"compared_fields": 3, "differences": [
        {"field": "Product code", "eds": 7, "adapter": 1},
        {"field": "Minor revision", "eds": 2, "adapter": 0},
    ]}
    assert not peer["forward_close"]
    read, _ = run("wide-object-path", "read", LOCAL, LOCAL, "0x1234", "0x12345678", "7")
    assert json.loads(read)["data_hex"] == "CA FE"
    all_attrs, _ = run("all-attributes", "read", LOCAL, LOCAL, "1", "1", "all", mode="normal")
    assert bytes.fromhex(json.loads(all_attrs)["data_hex"])[15:] == b"Loopback Adapter"
    rejected, _ = with_adapter(root, "rejected-read",
                               [exe, "read", LOCAL, LOCAL, "1", "1", "0x7f"], "explicit")
    assert rejected.returncode == 1
    failure = json.loads(rejected.stdout)
    assert failure["general_status"] == 0x14 and failure["extended_status"] == [0x0302]
    check_class_info(exe, root)

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

    typed = root / "typed-read.toml"
    selection = 'selection = { offset = 1, kind = "U16", big_endian = true, swap = "bytes_in_words" }'
    typed_source = routed_profile.read_text(encoding="utf-8").replace(
        "[app.cip_reads.path]", f"{selection}\n[app.cip_reads.path]")
    typed.write_text(typed_source, encoding="utf-8")
    read, peer = run("routed-typed", "read-saved", typed, "Serial number", mode="routed")
    reply = json.loads(read)
    assert reply["data_hex"] == "78 56 34 12" and reply["reply_from"] == "target"
    assert reply["decoded"] == str(0x3456) and "decode_error" not in reply
    assert reply["selection"] == {"offset": 1, "kind": "U16", "big_endian": True, "bit": None,
                                  "swap": "bytes_in_words", "string_length": None}
    assert not peer["forward_close"]

    short = root / "short-typed-reply.toml"
    short.write_text(typed_source.replace(selection, 'selection = { offset = 3, kind = "U16" }')
                     .replace('route = "1/3"\n', ""), encoding="utf-8")
    read, _ = run("short-typed", "read-saved", short, "Serial number")
    reply = json.loads(read)
    assert reply["data_hex"] == "78 56 34 12" and reply["decoded"] is None
    assert reply["decode_error"] == "Selection at byte 3 needs 2 bytes. Reply has 4 bytes."

    def saved_request(name, attribute, options=""):
        return (f'\n[[app.cip_reads]]\nname = "{name}"\n{options}'
                f'path = {{ class = 1, instance = 1, attribute = {attribute} }}\n')

    short_field = saved_request("Short typed reply", 6, 'selection = { offset = 3, kind = "U16" }\n')
    product_name = saved_request("Product name", 7)
    all_ok = root / "all-saved-reads.toml"
    all_ok.write_text(typed_source + short_field + product_name, encoding="utf-8")
    read, peer = run("all-saved", "read-saved-all", all_ok, mode="saved-reads")
    results = json.loads(read)["results"]
    assert [item["status"] for item in results] == ["success"] * 3
    assert results[0]["decoded"] == str(0x3456) and results[0]["route"] == "1/3"
    assert results[1]["decoded"] is None and results[1]["data_hex"] == "78 56 34 12"
    assert "decode_error" in results[1] and results[2]["decoded"] == "Loopback Adapter"
    assert peer["registrations"] == 1 and peer["unregistered"]
    assert len(peer["explicit_requests"]) == 3 and not peer["forward_close"]

    mixed = root / "mixed-saved-reads.toml"
    mixed.write_text(
        typed_source + saved_request("Rejected route", 6, 'route = "1/7"\n')
        + saved_request("Rejected target", 127, 'route = "1/3"\n') + product_name
        + saved_request("Peer closes during read", 126) + saved_request("Unread", 4),
        encoding="utf-8",
    )
    read, peer = with_adapter(root, "mixed-saved", [exe, "read-saved-all", mixed], "saved-reads")
    assert read.returncode == 1, (read.stdout, read.stderr)
    report = json.loads(read.stdout)
    assert report["adapter_or_bridge"] == report["local_ip"] == LOCAL
    results = report["results"]
    assert [item["name"] for item in results] == [
        "Serial number", "Rejected route", "Rejected target", "Product name", "Peer closes during read", "Unread",
    ]
    assert [item["status"] for item in results] == [
        "success", "cip_rejected", "cip_rejected", "success", "exchange_failed", "not_attempted",
    ]
    assert results[1]["reply_from"] == "router" and results[1]["extended_status"] == [0x0311]
    assert results[2]["reply_from"] == "target" and results[2]["extended_status"] == [0x0302]
    assert results[3]["decoded"] == "Loopback Adapter" and results[4]["error"]
    assert peer["registrations"] == 1 and len(peer["explicit_requests"]) == 5

    serial = saved_request("Serial with selection", 6, selection + "\n")
    all_attributes = ('\n[[app.cip_reads]]\nname = "Identity object"\n'
                      'path = { class = 1, instance = 1 }\n')
    connected_args = ("--connected", "--multiple-service-packet")
    for mode, additional, wanted in (
        ("success", "", ["success"] * 3),
        ("partial", saved_request("Unsupported attribute", 127),
         ["success", "cip_rejected", "success", "success"]),
    ):
        packet = root / f"multiple-service-{mode}.toml"
        packet.write_text(source + product_name + additional + serial + all_attributes, encoding="utf-8")
        result, peer = with_adapter(
            root, f"multiple-service-{mode}",
            [exe, "read-saved-all", packet, "--multiple-service-packet"], "multiple-service",
        )
        assert result.returncode == (1 if additional else 0), (result.stdout, result.stderr)
        replies = json.loads(result.stdout)["results"]
        assert [item["status"] for item in replies] == wanted
        assert replies[0]["decoded"] == "Loopback Adapter"
        assert replies[-2]["decoded"] == str(0x3456) and replies[-2]["data_hex"] == "78 56 34 12"
        assert bytes.fromhex(replies[-1]["data_hex"])[15:] == b"Loopback Adapter"
        if additional:
            assert replies[1]["general_status"] == 0x14 and replies[1]["extended_status"] == [0x0302]
        assert peer["registrations"] == 1 and peer["unregistered"] and not peer["forward_close"]
        [request] = peer["explicit_requests"]
        assert bytes.fromhex(request)[:6] == bytes.fromhex("0A 02 20 02 24 01")
        route, route_hex = ("1/3", "0103") if additional else (
            "1/2/2/192.0.2.100/1/0", "0102120b3139322e302e322e313030000100",
        )
        routed_packet = root / f"routed-packet-{mode}.toml"
        routed_packet.write_text(packet.read_text(encoding="utf-8").replace(
            "[[app.cip_reads]]\n", f'[[app.cip_reads]]\nroute = "{route}"\n'), encoding="utf-8")
        result, peer = with_adapter(
            root, f"routed-packet-{mode}",
            [exe, "read-saved-all", routed_packet, "--multiple-service-packet"], "multiple-service",
        )
        assert result.returncode == (1 if additional else 0), (result.stdout, result.stderr)
        routed = json.loads(result.stdout)
        assert routed["results"] == [
            {**reply, "route": route, "bridge": LOCAL, "reply_from": "target"} for reply in replies
        ]
        assert peer["registrations"] == 1 and peer["unregistered"] and not peer["forward_close"]
        assert len(peer["explicit_requests"]) == 1 and peer["route_hex"] == route_hex
        assert peer["embedded_hex"] == request

        case = f"class3-packet-{mode}"
        result, peer = with_adapter(root, case, [exe, "read-saved-all", packet, *connected_args], "class3-packet")
        assert result.returncode == (1 if additional else 0), (result.stdout, result.stderr)
        connected = json.loads(result.stdout)
        assert connected["forward_close"] == {"outcome": "confirmed"}
        assert connected["results"] == replies and "packet_error" not in connected
        assert peer["connected_requests"] == [request]
        assert peer["registrations"] == 1 and peer["unregistered"] and peer["forward_close"]

    packet = root / "multiple-service-success.toml"
    result, peer = with_adapter(
        root, "class3-packet-close-reject", [exe, "read-saved-all", packet, "--multiple-service-packet", "--connected"],
        "class3-packet-close-reject",
    )
    assert result.returncode == 1, (result.stdout, result.stderr)
    connected = json.loads(result.stdout)
    assert connected["forward_close"] == {
        "outcome": "rejected", "general_status": 1, "extended_status": [0x0107],
    }
    assert [read["status"] for read in connected["results"]] == ["success"] * 3
    assert peer["forward_close"] and len(peer["connected_requests"]) == 1

    routed_sequential = root / "class3-routed-sequential.toml"
    routed_sequential.write_text(source + "".join(
        saved_request(name, attribute, 'route = "1/3"\n') for name, attribute in
        (("Product name", 7), ("Unsupported attribute", 127), ("Serial number", 6))
    ), encoding="utf-8")
    routed_connected_packet = root / "class3-routed-packet.toml"
    multi_hop = "1/2/2/192.0.2.100/1/0"
    routed_connected_packet.write_text((root / "routed-packet-partial.toml").read_text(encoding="utf-8")
                                       .replace('route = "1/3"', f'route = "{multi_hop}"'), encoding="utf-8")
    multi_hop_path = "0102120b3139322e302e322e31303000010020022401"
    for label, mode, profile, flags, route, path, request_count, exit_code, expected_statuses in (
        ("sequential", "class3-routed", routed_sequential, ["--connected"], "1/3",
         "010320022401", 3, 1, ["success", "cip_rejected", "success"]),
        ("success", "class3-routed-packet", root / "routed-packet-success.toml", connected_args, multi_hop,
         multi_hop_path, 1, 0, ["success"] * 3),
        ("mixed", "class3-routed-packet", routed_connected_packet, connected_args, multi_hop,
         multi_hop_path, 1, 1, ["success", "cip_rejected", "success", "success"]),
        ("unsupported", "class3-routed-packet-unsupported", routed_connected_packet, connected_args, multi_hop,
         multi_hop_path, 1, 1, []),
    ):
        result, peer = with_adapter(root, f"class3-routed-{label}", [exe, "read-saved-all", profile, *flags], mode)
        assert result.returncode == exit_code, (result.stdout, result.stderr)
        reply = json.loads(result.stdout)
        assert reply["connection_route"] == route and reply["forward_close"] == {"outcome": "confirmed"}
        assert [item["status"] for item in reply["results"]] == expected_statuses
        if expected_statuses:
            assert all(item["route"] == route and item["bridge"] == LOCAL and item["reply_from"] == "target"
                       for item in reply["results"])
            assert reply["results"][0]["decoded"] == "Loopback Routed Target"
            serial_index = 1
            if "cip_rejected" in expected_statuses:
                assert reply["results"][1]["extended_status"] == [0x0302]
                serial_index = 2
            assert reply["results"][serial_index]["data_hex"] == "78 56 34 12"
            assert "packet_error" not in reply
        else:
            assert reply["packet_error"] == {"general_status": 8, "extended_status": []}
        assert peer["open_path_hex"] == peer["close_path_hex"] == path
        assert peer["registrations"] == 1 and peer["unregistered"] and peer["forward_close"]
        assert len(peer["connected_requests"]) == request_count and "cyclic_io" not in peer

    result, peer = with_adapter(
        root, "class3-routed-open-reject", [exe, "read-saved-all", routed_sequential, "--connected"],
        "class3-routed-open-reject",
    )
    assert result.returncode == 1 and not result.stdout
    assert "Class 3 Forward_Open via 1/3" in result.stderr and "0x0311" in result.stderr
    assert peer["open_path_hex"] == "010320022401" and peer["unregistered"]
    assert "connected_requests" not in peer and not peer["forward_close"]

    oversized = root / "class3-packet-oversized.toml"
    oversized.write_text(source + "".join(saved_request(f"Field {i}", 7) for i in range(50)), encoding="utf-8")
    result = subprocess.run([exe, "read-saved-all", oversized, "--connected", "--multiple-service-packet"],
                            capture_output=True, text=True, timeout=4, creationflags=NO_WINDOW)
    assert result.returncode == 1 and "limit is 502 bytes" in result.stderr

    for label, route in (("direct", None), ("routed", multi_hop)):
        large = root / f"class3-large-{label}.toml"
        reads = "".join(saved_request(f"Field {i}", 7, f'route = "{route}"\n' if route else "")
                        for i in range(50))
        large.write_text(source + "\n[app]\ncip_class3_size = 4002\n" + reads, encoding="utf-8")
        mode = "class3-routed-large-packet" if route else "class3-large-packet"
        result, peer = with_adapter(root, mode, [exe, "read-saved-all", large, *connected_args], mode)
        assert result.returncode == 0, (result.stdout, result.stderr)
        reply = json.loads(result.stdout)
        assert reply["class3_size"] == 4002 and reply["large_forward_open"] is True
        assert reply.get("connection_route") == route and reply["forward_close"] == {"outcome": "confirmed"}
        assert len(reply["results"]) == 50 and all(item["status"] == "success" for item in reply["results"])
        assert peer["class3_size"] == 4002 and peer["open_service"] == 0x5B
        [request] = peer["connected_requests"]
        assert len(bytes.fromhex(request)) == 508 and min(peer["connected_reply_sizes"]) > 504
        assert peer["unregistered"] and peer["forward_close"] and "cyclic_io" not in peer
        if route:
            assert peer["open_path_hex"] == peer["close_path_hex"] == multi_hop_path

    standard = root / "class3-standard-511.toml"
    standard.write_text(source + "\n[app]\ncip_class3_size = 511\n" + product_name, encoding="utf-8")
    result, peer = with_adapter(root, "class3-sized", [exe, "read-saved-all", standard, "--connected"], "class3-sized")
    assert result.returncode == 0, (result.stdout, result.stderr)
    reply = json.loads(result.stdout)
    assert reply["class3_size"] == 511 and reply["large_forward_open"] is False
    assert peer["class3_size"] == 511 and peer["open_service"] == 0x54 and peer["forward_close"]

    result, peer = with_adapter(
        root, "class3-large-open-reject",
        [exe, "read-saved-all", root / "class3-large-direct.toml", *connected_args], "class3-large-open-reject",
    )
    assert result.returncode == 1 and not result.stdout
    assert "Class 3 Large Forward_Open" in result.stderr and "0x0126" in result.stderr and "0x01F8" in result.stderr
    assert peer["open_service"] == 0x5B and peer["registrations"] == 1 and peer["unregistered"]
    assert "connected_requests" not in peer and not peer["forward_close"]

    for size, message in ((8, "read needs 8 CIP bytes"), (4003, "Class 3 connection size must be")):
        invalid_size = root / f"class3-size-{size}.toml"
        invalid_size.write_text(source + f"\n[app]\ncip_class3_size = {size}\n" + product_name, encoding="utf-8")
        result = subprocess.run([exe, "read-saved-all", invalid_size, "--connected"],
                                capture_output=True, text=True, timeout=4, creationflags=NO_WINDOW)
        assert result.returncode == 1 and message in result.stderr, (result.stdout, result.stderr)

    for route, error, status in [
        ("1/7", "CIP router rejected Unconnected Send", "0x0311"),
        ("1/8", "CIP target rejected read", "0x08"),
    ]:
        profile = root / f"routed-packet-{route[-1]}-rejected.toml"
        profile.write_text((root / "routed-packet-partial.toml").read_text(encoding="utf-8")
                           .replace('route = "1/3"', f'route = "{route}"'), encoding="utf-8")
        result, peer = with_adapter(
            root, f"routed-packet-{route[-1]}-rejected",
            [exe, "read-saved-all", profile, "--multiple-service-packet"], "multiple-service",
        )
        assert result.returncode == 1 and not result.stdout, (result.stdout, result.stderr)
        assert f"Multiple Service Packet via {route}: {error}" in result.stderr and status in result.stderr
        assert len(peer["explicit_requests"]) == 1 and peer["unregistered"] and not peer["forward_close"]

    split_routes = root / "routed-packet-split.toml"
    split_routes.write_text((root / "routed-packet-partial.toml").read_text(encoding="utf-8")
                            .replace('route = "1/3"', 'route = "1/7"', 1), encoding="utf-8")
    for profile in (all_ok, split_routes):
        result = subprocess.run([exe, "read-saved-all", profile, "--multiple-service-packet"],
                                capture_output=True, text=True, timeout=4, creationflags=NO_WINDOW)
        assert result.returncode == 1 and "requires one shared route or all direct reads" in result.stderr
    for profile, flags in ((all_ok, ["--connected"]), (split_routes, connected_args)):
        result = subprocess.run([exe, "read-saved-all", profile, *flags],
                                capture_output=True, text=True, timeout=4, creationflags=NO_WINDOW)
        assert result.returncode == 1 and "requires one shared route or all direct reads" in result.stderr

    empty = subprocess.run([exe, "read-saved-all", PROFILE], capture_output=True, text=True,
                           timeout=4, creationflags=NO_WINDOW)
    assert empty.returncode == 1 and "no saved CIP reads" in empty.stderr

    invalid = root / "invalid-typed-read.toml"
    invalid.write_text(typed_source.replace(selection, 'selection = { offset = 0, kind = "Bool", bit = 8 }'),
                       encoding="utf-8")
    checked = subprocess.run([exe, "read-saved", invalid, "Serial number"], capture_output=True,
                             text=True, timeout=4, creationflags=NO_WINDOW)
    assert checked.returncode == 1 and "Bit index must be" in checked.stderr

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
    check_routed_input(exe, root, source)


if __name__ == "__main__":
    main()
