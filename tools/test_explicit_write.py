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
        incomplete = subprocess.run([*command, "--apply", "0:12:1"], capture_output=True,
                                    text=True, timeout=6, creationflags=NO_WINDOW)
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


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("exe", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    check_writes(args.exe.resolve(strict=True), args.output.resolve())
    print("Explicit write preview, identity pin, direct and routed writes, and uncertain outcomes passed.")
