"""Verify reviewed explicit attribute writes on localhost."""

import argparse
import json
from pathlib import Path
import socket
import subprocess

from test_cli import NO_WINDOW, with_adapter

LOCAL = "127.0.0.1"
PIN = "0:12:1:305419896"
VALUE = "A5 16 2C 93"


def check_writes(exe, root):
    root.mkdir()
    command = [exe, "write-attribute", LOCAL, LOCAL, "4", "154", "3", VALUE]
    with socket.socket() as listener:
        listener.bind((LOCAL, 44818))
        listener.listen(1)
        preview = subprocess.run([*command, "--route", "1/3"], capture_output=True,
                                 text=True, timeout=6, creationflags=NO_WINDOW)
        preview_readback = subprocess.run([*command, "--route", "1/3", "--readback"], capture_output=True,
                                          text=True, timeout=6, creationflags=NO_WINDOW)
        duplicate = subprocess.run([*command, "--readback", "--readback", "--apply", PIN], capture_output=True,
                                    text=True, timeout=6, creationflags=NO_WINDOW)
        incomplete = subprocess.run([*command, "--apply", "0:12:1"], capture_output=True,
                                    text=True, timeout=6, creationflags=NO_WINDOW)
        check_typed_previews(command[:-1], root)
        listener.setblocking(False)
        try:
            unexpected, _ = listener.accept()
        except BlockingIOError:
            pass
        else:
            unexpected.close()
            raise AssertionError("Offline preview or invalid apply opened a session")
    assert preview.returncode == 0 and not preview.stderr
    report = json.loads(preview.stdout)
    assert report == {
        "adapter_or_bridge": LOCAL, "local_ip": LOCAL, "route": "1/3",
        "path": {"class": 4, "instance": 154, "attribute": 3},
        "service": "Set_Attribute_Single", "value_hex": VALUE, "bytes": 4,
        "result": {"outcome": "preview"},
    }
    (root / "preview.json").write_text(preview.stdout, encoding="utf-8")
    assert incomplete.returncode == 1 and "requires decimal vendor:type:product:serial" in incomplete.stderr
    assert preview_readback.returncode == 0 and not preview_readback.stderr
    assert json.loads(preview_readback.stdout) == {**report, "readback_requested": True}
    (root / "readback-preview.json").write_text(preview_readback.stdout, encoding="utf-8")
    assert duplicate.returncode == 1 and not duplicate.stdout

    for case, mode, route, pin, expected in (
        ("direct", "explicit-write", None, PIN, "acknowledged"),
        ("routed", "explicit-write", "1/3", PIN, "acknowledged"),
        ("mismatch", "explicit-write-review", None, "0:12:1:305419897", "not_sent"),
        ("rejected", "explicit-write-rejected", None, PIN, "rejected"),
        ("no-reply", "explicit-write-no-reply", None, PIN, "unconfirmed"),
        ("router-timeout", "explicit-write-router-timeout", "1/3", PIN, "router_error"),
    ):
        args = [*command, "--apply", pin]
        if route:
            args.extend(("--route", route))
        result, peer = with_adapter(root, case, args, mode)
        assert result.returncode == (0 if expected == "acknowledged" else 1), (result.stdout, result.stderr)
        report = json.loads(result.stdout)
        (root / f"{case}.json").write_text(result.stdout, encoding="utf-8")
        assert report["value_hex"] == VALUE and report["route"] == route
        outcome = report["result"]
        assert outcome["outcome"] == expected
        assert "readback" not in outcome and "readback_requested" not in report
        assert outcome["observed_identity"]["serial"] == 305419896
        assert outcome["expected_identity"]["serial"] == int(pin.rsplit(":", 1)[1])
        assert peer["registrations"] == 1 and peer["identity_route"] == ("0103" if route else "")
        assert not peer["forward_close"] and "cyclic_io" not in peer
        if case == "mismatch":
            assert "writes" not in peer and "did not match" in outcome["error"]
        else:
            assert peer["writes"] == ["10032004249a3003a5162c93"]
        if case != "no-reply":
            assert peer["unregistered"]
        if case == "rejected":
            assert outcome["general_status"] == 0x0E and outcome["extended_status"] == [0x0302]
        if case == "router-timeout":
            assert outcome["general_status"] == 1 and outcome["extended_status"] == [0x0204]
        if case == "no-reply":
            assert "reply ended early" in outcome["error"]
            assert "read exchange" not in outcome["error"]

    check_readbacks(command, root)
    check_typed_apply(command[:-1], root)


def check_typed_previews(address, root):
    for name, input_value, options, payload in (
        ("bool", "true", ["--type", "Bool"], "01"),
        ("signed", "-2", ["--type", "I16"], "FE FF"),
        ("float", "1.5", ["--type", "F32"], "00 00 C0 3F"),
        ("swapped", "747873558",
         ["--type", "U32", "--endian", "big", "--swap", "word_order"], VALUE),
        ("utf8", "\u00e9", ["--type", "Utf8", "--text-bytes", "4"], "C3 A9 00 00"),
    ):
        result = subprocess.run([*address, input_value, *options], capture_output=True, text=True,
                                encoding="utf-8", timeout=6, creationflags=NO_WINDOW)
        assert result.returncode == 0 and not result.stderr, (result.stdout, result.stderr)
        report = json.loads(result.stdout)
        assert report["value_hex"] == payload and report["bytes"] == len(bytes.fromhex(payload))
        assert report["typed_value"] == input_value and report["encoding"]["kind"] == options[1]
        assert report["result"] == {"outcome": "preview"}
        (root / f"typed-{name}.json").write_text(result.stdout, encoding="utf-8")

    for value, options, message in (
        ("256", ["--type", "U8"], "valid U8"),
        ("\u00e9", ["--type", "Utf8", "--text-bytes", "1"], "fit the fixed byte length"),
        ("FF", ["--endian", "big"], "require --type"),
        ("7", ["--type", "U16", "--swap", "word_order"], "This swap needs"),
        ("7", ["--type", "U16", "--text-bytes", "4"], "fixed text length requires Utf8"),
    ):
        result = subprocess.run([*address, value, *options, "--apply", PIN],
                                capture_output=True, text=True, timeout=6, creationflags=NO_WINDOW)
        assert result.returncode == 1 and not result.stdout and message in result.stderr, result


def check_typed_apply(address, root):
    args = [*address, "747873558", "--type", "U32", "--endian", "big",
            "--swap", "word_order", "--apply", PIN, "--readback"]
    result, peer = with_adapter(root, "typed-apply", args, "explicit-write-readback")
    assert result.returncode == 0 and not result.stderr, (result.stdout, result.stderr)
    report = json.loads(result.stdout)
    (root / "typed-apply.json").write_text(result.stdout, encoding="utf-8")
    assert report["value_hex"] == VALUE and report["typed_value"] == "747873558"
    assert report["encoding"]["big_endian"] and report["encoding"]["swap"] == "word_order"
    assert report["result"]["outcome"] == "acknowledged"
    assert report["result"]["readback"] == {"outcome": "matched", "data_hex": VALUE}
    assert peer["registrations"] == 1 and peer["unregistered"]
    assert peer["writes"] == ["10032004249a3003a5162c93"] and peer["readbacks"] == ["0e032004249a3003"]


def check_readbacks(command, root):
    for case, mode, route, pin, write, readback in (
        ("direct", "explicit-write-readback", None, PIN, "acknowledged",
         {"outcome": "matched", "data_hex": VALUE}),
        ("routed", "explicit-write-readback", "1/3", PIN, "acknowledged",
         {"outcome": "matched", "data_hex": VALUE}),
        ("mismatch", "explicit-write-readback-mismatch", None, PIN, "acknowledged",
         {"outcome": "mismatch", "data_hex": "A5 16 2C 92"}),
        ("target-error", "explicit-write-readback-rejected", None, PIN, "acknowledged",
         {"outcome": "rejected", "general_status": 0x2C, "extended_status": [0x0302]}),
        ("router-error", "explicit-write-readback-router", "1/3", PIN, "acknowledged",
         {"outcome": "router_error", "general_status": 1, "extended_status": [0x0204]}),
        ("unavailable", "explicit-write-readback-no-reply", None, PIN, "acknowledged",
         {"outcome": "unavailable"}),
        ("blocked-write", "explicit-write-review", None, "0:12:1:305419897", "not_sent",
         {"outcome": "skipped"}),
        ("uncertain-write", "explicit-write-no-reply", None, PIN, "unconfirmed",
         {"outcome": "skipped"}),
        ("rejected-write", "explicit-write-rejected", None, PIN, "rejected",
         {"outcome": "skipped"}),
    ):
        args = [*command, "--readback", "--apply", pin]
        if route:
            args.extend(("--route", route))
        result, peer = with_adapter(root, f"readback-{case}", args, mode)
        assert result.returncode == int(readback["outcome"] != "matched"), (result.stdout, result.stderr)
        report = json.loads(result.stdout)
        (root / f"readback-{case}.json").write_text(result.stdout, encoding="utf-8")
        assert report["readback_requested"] is True and report["value_hex"] == VALUE
        assert report["result"]["outcome"] == write
        actual = report["result"]["readback"]
        if readback["outcome"] == "unavailable":
            assert "reply ended early" in actual["error"]
            actual = {key: value for key, value in actual.items() if key != "error"}
        assert actual == readback
        assert peer["registrations"] == 1 and not peer["forward_close"]
        assert peer.get("writes", []) == ([] if write == "not_sent" else ["10032004249a3003a5162c93"])
        assert peer.get("readbacks", []) == (["0e032004249a3003"] if write == "acknowledged" else [])


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("exe", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    check_writes(args.exe.resolve(strict=True), args.output.resolve())
    print("Explicit write preview, typed encoding, identity pin, direct and routed writes, and readback outcomes passed.")
