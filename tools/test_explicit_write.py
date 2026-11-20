"""Verify reviewed explicit attribute writes on localhost."""

import argparse
import json
from pathlib import Path
import socket
import subprocess
import tomllib

from test_cli import NO_WINDOW, with_adapter

LOCAL = "127.0.0.1"
PIN = "0:12:1:305419896"
VALUE = "A5 16 2C 93"
CURRENT = "10 20 30 40"


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
        current_preview = subprocess.run([*command, "--expect-current", CURRENT], capture_output=True,
                                         text=True, timeout=6, creationflags=NO_WINDOW)
        invalid_current = subprocess.run([*command, "--expect-current", "ZZ", "--apply", PIN],
                                         capture_output=True, text=True, timeout=6, creationflags=NO_WINDOW)
        check_typed_previews(command[:-1], root)
        check_connected_preflight(command, root)
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
    assert current_preview.returncode == 0 and not current_preview.stderr
    assert json.loads(current_preview.stdout) == {**report, "route": None, "expected_current_hex": CURRENT}
    (root / "current-preview.json").write_text(current_preview.stdout, encoding="utf-8")
    assert invalid_current.returncode == 1 and "Expected current bytes" in invalid_current.stderr

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
        assert "current_value" not in outcome and "expected_current_hex" not in report
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
    check_current_value(command, root)
    check_connected_writes(command, root)
    check_write_drafts(command, root)


def check_write_drafts(command, root):
    def call(args):
        return subprocess.run(args, capture_output=True, text=True, timeout=6, creationflags=NO_WINDOW)

    exe = command[0]
    direct = root / "direct write draft.toml"
    routed = root / "routed-class3-draft.toml"
    bridge = root / "routed-unconnected-draft.toml"
    connected = root / "direct-class3-draft.toml"
    large_route = "1/2/2/192.0.2.100/1/0"
    options = ["--expect-current", CURRENT, "--readback"]
    typed = [*command[:-1], "747873558", "--type", "U32", "--endian", "big", "--swap", "word_order"]
    with socket.socket() as listener:
        listener.bind((LOCAL, 44818))
        listener.listen(1)
        for draft, save, additional_fields in (
            (direct, command, {}),
            (bridge, [*command, "--route", "1/3"], {"route": "1/3"}),
            (connected, [*command, "--connected"], {"class3_size": 504}),
            (routed, [*typed, "--route", large_route, "--connected", "--connection-size", "4002"],
             {"route": large_route, "class3_size": 4002}),
        ):
            args = [*save, *options, "--save-draft", draft]
            saved = call(args)
            assert saved.returncode == 0 and not saved.stderr, (saved.stdout, saved.stderr)
            report = json.loads(saved.stdout)
            assert Path(report.pop("draft_saved")) == draft and report["value_hex"] == VALUE
            report.pop("encoding", None)
            report.pop("typed_value", None)
            preview = call([exe, "write-draft", draft])
            assert preview.returncode == 0 and not preview.stderr
            assert json.loads(preview.stdout) == report and report["result"] == {"outcome": "preview"}
            baseline = draft.read_bytes()
            assert tomllib.loads(baseline.decode()) == {
                "version": 1, "adapter_or_bridge": LOCAL, "local_ip": LOCAL, "value_hex": VALUE,
                "expected_current_hex": CURRENT, "readback": True, "path": report["path"],
                **additional_fields,
            }
            exists = call(args)
            assert exists.returncode == 1 and "Save write draft" in exists.stderr
            assert draft.read_bytes() == baseline and not exists.stdout

        neither = root / "unused-draft.toml"
        conflict = call([*command, "--apply", PIN, "--save-draft", neither])
        assert conflict.returncode == 1 and "separate commands" in conflict.stderr and not neither.exists()

        empty_check = root / "empty-current-draft.toml"
        saved = call([*command, "--expect-current", "", "--save-draft", empty_check])
        assert saved.returncode == 0 and not saved.stderr
        preview = call([exe, "write-draft", empty_check])
        assert preview.returncode == 0 and json.loads(preview.stdout)["expected_current_hex"] == ""

        valid = routed.read_text(encoding="utf-8")
        invalid = root / "invalid-draft.toml"
        for text, error in (
            (valid.replace("version = 1", "version = 2"), "Unsupported write draft version 2"),
            (f'apply = "{PIN}"\n{valid}', "unknown field `apply`"),
            (valid.replace(VALUE, "GG"), "Value bytes"),
            (valid.replace(CURRENT, "GG"), "Expected current bytes"),
            (valid.replace("class3_size = 4002", "class3_size = 13"), "needs 12 CIP bytes"),
            ("#" * (1024 * 1024 + 1), "Write draft exceeds 1 MiB"),
        ):
            assert text != valid
            invalid.write_text(text, encoding="utf-8")
            rejected = call([exe, "write-draft", invalid, "--apply", PIN])
            assert rejected.returncode == 1 and error in rejected.stderr and not rejected.stdout, rejected.stderr
        listener.settimeout(0.1)
        try:
            unexpected, _ = listener.accept()
        except TimeoutError:
            pass
        else:
            unexpected.close()
            raise AssertionError("Saving, previewing or rejecting a draft opened a network session")

    for draft, mode in ((direct, "explicit-write-current"), (routed, "class3-routed-large-write")):
        verified, peer = with_adapter(root, f"{draft.stem}-verify", [exe, "write-draft", draft, "--verify"], mode)
        assert verified.returncode == 0 and not verified.stderr, (verified.stdout, verified.stderr)
        result = json.loads(verified.stdout)["result"]
        assert result["outcome"] == "identity_read" and result["observed_identity"]["serial"] == 305419896
        assert peer["registrations"] == 1 and "writes" not in peer
        if draft == routed:
            assert peer["forward_close"] and peer["connected_requests"] == ["010220012401"]
        applied, peer = with_adapter(root, f"{draft.stem}-apply", [exe, "write-draft", draft, "--apply", PIN], mode)
        assert applied.returncode == 0 and not applied.stderr, (applied.stdout, applied.stderr)
        result = json.loads(applied.stdout)["result"]
        assert result["outcome"] == "acknowledged" and result["current_value"]["outcome"] == "matched"
        assert result["readback"] == {"outcome": "matched", "data_hex": VALUE}
        assert peer["writes"] == ["10032004249a3003a5162c93"]
        if draft == routed:
            assert result["forward_close"]["outcome"] == "confirmed"
        (root / f"{draft.stem}-apply.json").write_text(applied.stdout, encoding="utf-8")

    read = {"outcome": "read", "data_hex": CURRENT, "bytes": 4}
    for label, draft, mode, current, close, exit_code in (
        ("direct", direct, "explicit-write-current", read, None, 0),
        ("routed-large", routed, "class3-routed-large-write", read, "confirmed", 0),
        ("target-rejected", direct, "explicit-write-current-rejected",
         {"outcome": "rejected", "general_status": 0x2c, "extended_status": [0x0302]}, None, 1),
        ("router-error", bridge, "explicit-write-current-router",
         {"outcome": "router_error", "general_status": 1, "extended_status": [0x0204]}, None, 1),
        ("close-rejected", connected, "class3-write-close-reject", read, "rejected", 1),
    ):
        label = f"inspect-{label}"
        before = draft.read_bytes()
        received, peer = with_adapter(root, label, [exe, "write-draft", draft, "--inspect"], mode)
        assert received.returncode == exit_code and not received.stderr, (received.stdout, received.stderr)
        result = json.loads(received.stdout)["result"]
        assert result["outcome"] == "inspection" and result["service"] == "Get_Attribute_Single"
        assert result["observed_identity"]["serial"] == 305419896 and result["current"] == current
        assert peer["registrations"] == 1 and peer["unregistered"] and "writes" not in peer
        assert peer["current_reads"] == ["0e032004249a3003"] and draft.read_bytes() == before
        if close:
            assert result["forward_close"]["outcome"] == close
            assert peer["connected_requests"] == ["010220012401", "0e032004249a3003"]
        else:
            assert "forward_close" not in result
        (root / f"{label}.json").write_text(received.stdout, encoding="utf-8")


def check_connected_preflight(command, root):
    result = subprocess.run([*command, "--connected", "--connection-size", "4002"],
                            capture_output=True, text=True, timeout=6, creationflags=NO_WINDOW)
    assert result.returncode == 0 and not result.stderr
    preview = json.loads(result.stdout)
    assert preview["class3_size"] == 4002 and preview["large_forward_open"]
    assert preview["result"] == {"outcome": "preview"}
    (root / "class3-preview.json").write_text(result.stdout, encoding="utf-8")
    for flags, error in (
        (["--connection-size", "504"], "requires --connected"),
        (["--connected", "--connection-size", "13"], "needs 12 CIP bytes"),
        (["--connected", "--connection-size", "4003"], "Class 3 connection size must be"),
    ):
        result = subprocess.run([*command, "--apply", PIN, *flags], capture_output=True,
                                text=True, timeout=6, creationflags=NO_WINDOW)
        assert result.returncode == 1 and error in result.stderr and not result.stdout


def check_connected_writes(command, root):
    identity = "010220012401"
    read = "0e032004249a3003"
    write = "10032004249a3003a5162c93"
    full_sequence = [identity, read, write, read]
    for case, mode, requests, outcome, close in (
        ("direct", "class3-write", full_sequence, "acknowledged", "confirmed"),
        ("routed-large", "class3-routed-large-write", full_sequence, "acknowledged", "confirmed"),
        ("close-rejected", "class3-write-close-reject", full_sequence, "acknowledged", "rejected"),
        ("wrong-sequence", "class3-write-bad-sequence", full_sequence[:-1], "unconfirmed", "confirmed"),
        ("wrong-identity", "class3-write", [identity], "not_sent", "confirmed"),
        ("wrong-current", "class3-write", [identity, read], "not_sent", "confirmed"),
        ("open-rejected", "class3-write-open-reject", [], "not_sent", None),
    ):
        label = f"class3-{case}"
        pin = "0:12:1:305419897" if case == "wrong-identity" else PIN
        expected = "10 20 30 41" if case == "wrong-current" else CURRENT
        args = [*command, "--connected", "--apply", pin, "--expect-current", expected, "--readback"]
        routed = case == "routed-large"
        if routed:
            args.extend(("--route", "1/2/2/192.0.2.100/1/0", "--connection-size", "4002"))
        result, peer = with_adapter(root, label, args, mode)
        assert result.returncode == int(outcome != "acknowledged" or close != "confirmed")
        assert not result.stderr
        report = json.loads(result.stdout)
        (root / f"{label}.json").write_text(result.stdout, encoding="utf-8")
        assert report["class3_size"] == (4002 if routed else 504) and report["large_forward_open"] == routed
        actual = report["result"]
        assert actual["outcome"] == outcome
        if close:
            assert actual["forward_close"]["outcome"] == close
        else:
            assert "forward_close" not in actual and "Class 3 Forward_Open" in actual["error"]
        if outcome == "unconfirmed":
            assert "data or sequence" in actual["error"]
        if close == "rejected":
            assert actual["forward_close"]["extended_status"] == [0x0107]
        assert actual["current_value"]["outcome"] == (
            "mismatch" if case == "wrong-current" else "matched" if read in requests else "skipped"
        )
        assert actual["readback"] == ({"outcome": "matched", "data_hex": VALUE}
                                     if outcome == "acknowledged" else {"outcome": "skipped"})
        assert peer["registrations"] == 1 and peer["unregistered"]
        assert peer.get("connected_requests", []) == requests
        assert peer["forward_close"] == bool(close)
        if routed:
            assert peer["open_path_hex"] == peer["close_path_hex"] == "0102120b3139322e302e322e31303000010020022401"


def check_current_value(command, root):
    for case, mode, route, pin, current in (
        ("direct", "explicit-write-current", None, PIN, {"outcome": "matched", "data_hex": CURRENT}),
        ("routed", "explicit-write-current", "1/3", PIN, {"outcome": "matched", "data_hex": CURRENT}),
        ("changed", "explicit-write-current-mismatch", None, PIN,
         {"outcome": "mismatch", "data_hex": "10 20 30 41"}),
        ("target-error", "explicit-write-current-rejected", None, PIN,
         {"outcome": "rejected", "general_status": 0x2C, "extended_status": [0x0302]}),
        ("router-error", "explicit-write-current-router", "1/3", PIN,
         {"outcome": "router_error", "general_status": 1, "extended_status": [0x0204]}),
        ("unavailable", "explicit-write-current-no-reply", None, PIN, {"outcome": "unavailable"}),
        ("identity-mismatch", "explicit-write-review", None, "0:12:1:305419897", {"outcome": "skipped"}),
    ):
        args = [*command, "--expect-current", CURRENT, "--apply", pin, "--readback"]
        if route:
            args.extend(("--route", route))
        result, peer = with_adapter(root, f"current-{case}", args, mode)
        matched = current["outcome"] == "matched"
        assert result.returncode == int(not matched) and not result.stderr, (result.stdout, result.stderr)
        report = json.loads(result.stdout)
        (root / f"current-{case}.json").write_text(result.stdout, encoding="utf-8")
        assert report["expected_current_hex"] == CURRENT and report["value_hex"] == VALUE
        assert report["result"]["outcome"] == ("acknowledged" if matched else "not_sent")
        check = report["result"]["current_value"]
        if current["outcome"] == "unavailable":
            assert "reply ended early" in check.pop("error")
        assert check == {**current, "expected_hex": CURRENT}
        assert report["result"]["readback"] == ({"outcome": "matched", "data_hex": VALUE}
                                                if matched else {"outcome": "skipped"})
        assert peer["registrations"] == 1 and peer["identity_route"] == ("0103" if route else "")
        assert peer.get("writes", []) == (["10032004249a3003a5162c93"] if matched else [])
        assert peer.get("readbacks", []) == (["0e032004249a3003"] if matched else [])
        assert peer.get("current_reads", []) == ([] if case == "identity-mismatch" else ["0e032004249a3003"])
        if case != "unavailable":
            assert peer["unregistered"]


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
    print("Unconnected and Class 3 write previews, identity, current-value, readback, close, and draft inspection passed.")
