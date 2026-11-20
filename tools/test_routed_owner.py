"""Verify Exclusive Owner lifecycle with a loopback CIP bridge."""
import argparse
from pathlib import Path

from test_cli import with_adapter


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("test_exe", type=Path, help="Compiled assembly-scanner Rust test executable")
    parser.add_argument("directory", type=Path, help="New directory for the bridge reports")
    args = parser.parse_args()
    root = args.directory.resolve()
    root.mkdir(parents=True)
    cases = {
        "routed-owner": "routed_owner_sends_configuration_and_updates_fixed_outputs",
        "routed-large-owner": "routed_owner_keyed_large_open_changes_variable_output_length",
        "routed-owner-replacement": "routed_owner_blocks_a_replaced_target",
        "routed-owner-unexpected-peer": "routed_owner_closes_when_the_bridge_advertises_another_udp_peer",
    }
    config_segment = "8002aabbcc00"
    standard_path = "0103200424012c642c65"
    large_path = "010212093132372e302e302e33000100340401000c0001000100200424012c642c65"
    for case, name in cases.items():
        result, report = with_adapter(
            root, case, [args.test_exe.resolve(), name, "--ignored"], case,
            fixture_args=("--address", "127.0.0.2"),
        )
        if result.returncode != 0 or "1 passed" not in result.stdout:
            raise RuntimeError(f"{case} failed:\n{result.stdout}\n{result.stderr}")
        preview, opened = report["attempts"]
        assert "open_path_hex" not in preview and not preview["forward_close"]
        large = case == "routed-large-owner"
        requested_attributes = ["all", *range(1, 8)] if large else ["all"]
        assert preview["routed_identity_attributes"] == opened["routed_identity_attributes"] == requested_attributes
        if case == "routed-owner-replacement":
            assert opened["serial_number"] == preview["serial_number"] + 1
            assert "open_path_hex" not in opened and not opened["forward_close"]
            continue

        path = large_path if large else standard_path
        assert opened["open_service"] == (0x5B if large else 0x54)
        assert opened["open_path_hex"] == path + config_segment
        assert opened["forward_close"] and opened["close_path_hex"] == path
        assert opened["ot_requested_us"] == opened["to_requested_us"] == 8000
        if case == "routed-owner-unexpected-peer":
            assert opened["cyclic_io"]["output_destination"].startswith("127.0.0.3:")
            continue

        assert opened["cyclic_io"]["output_destination"].startswith("127.0.0.2:")
        assert opened["cyclic_io"]["first_input_source"].startswith("127.0.0.2:")
        assert opened["to_sent"] >= 16
        assert opened["ot_invalid"] == 0
        if large:
            assert opened["output_length_transitions"] == [2, 600, 5, 2]
            assert opened["output_values"] == ["1234", "5b" * 5, "a5" * 600, "cafe"]
        else:
            assert opened["output_values"] == ["1234", "abcd"]
    print("Validated routed Exclusive Owner open, cyclic output, identity checks, and close.")


if __name__ == "__main__":
    main()
