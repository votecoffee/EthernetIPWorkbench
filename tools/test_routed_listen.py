"""Test routed Class 1 listen-only against an OpENer multicast producer."""
import argparse
from contextlib import contextmanager
import json
from pathlib import Path
import socket
from socketserver import BaseRequestHandler, ThreadingTCPServer
import struct
import subprocess
import sys
import threading
import tomllib

from loopback_adapter import recv_exact, routed_request, rr_items, rr_reply
from test_opener import ADAPTER, PROFILE, build_adapter, call, check_capture, controlling_heartbeat, run_adapter


BRIDGE_ROUTE = bytes.fromhex("01 03")
BACKEND_PORT = 44819
HEADER = struct.Struct("<HHII8sI")
APPLICATION_PATHS = {bytes.fromhex(f"20 04 24 97 2C {assembly:02x} 2C 64"): role
                     for assembly, role in [(152, "controller"), (153, "listener")]}


class BridgeRequest(BaseRequestHandler):
    def handle(self):
        # OpENer shares the bridge's loopback IP for UDP and uses a separate TCP port.
        with socket.create_connection((ADAPTER, BACKEND_PORT), timeout=5,
                                      source_address=("127.0.0.1", 0)) as adapter:
            self.request.settimeout(8)
            while True:
                try:
                    command, length, session, status, context, options = HEADER.unpack(
                        recv_exact(self.request, HEADER.size))
                except EOFError:
                    return
                body = recv_exact(self.request, length)
                event = None
                if command == 0x6F:
                    body, event = self.translate_cip(body)
                else:
                    assert command in (0x65, 0x66)
                adapter.sendall(HEADER.pack(command, len(body), session, status, context, options) + body)
                if command == 0x66:
                    return
                reply = recv_exact(adapter, HEADER.size)
                payload = recv_exact(adapter, HEADER.unpack(reply)[1])
                if event is not None:
                    # OpENer can leave bytes past the declared CPF items in a rejected open.
                    null, null_size, item, size = struct.unpack_from("<HHHH", payload, 8)
                    assert (null, null_size, item) == (0, 0, 0xB2)
                    cip = payload[16:16 + size]
                    assert len(cip) == size and size >= 4
                    event["general_status"] = cip[2]
                    event["extended_status"] = list(struct.unpack_from(f"<{cip[3]}H", cip, 4))
                    self.server.events.append(event)
                self.request.sendall(reply + payload)

    def translate_cip(self, body):
        items = rr_items(body)
        cip = items.pop(0xB2)
        assert items.pop(0) == b""
        service = cip[0]
        if service == 0x52:
            route, message = routed_request(cip)
            assert route == BRIDGE_ROUTE and message[0] in (1, 0x0E)
            assert message[2:6] == bytes.fromhex("20 01 24 01")
            return rr_reply(message), {"service": service, "route": route.hex()}
        assert service in (0x54, 0x5B, 0x4E)
        assert cip[1:6] == bytes.fromhex("02 20 06 24 01")
        size_at, path_at = {0x54: (41, 42), 0x5B: (45, 46), 0x4E: (16, 18)}[service]
        path = cip[path_at:]
        assert len(path) == cip[size_at] * 2 and path.startswith(BRIDGE_ROUTE)
        target_path = bytes(path[len(BRIDGE_ROUTE):])
        role = APPLICATION_PATHS[target_path]
        if service == 0x4E:
            assert not items
        else:
            assert set(items) == {0x8001}
            family, port, address = struct.unpack(">HH4s8x", items[0x8001])
            assert (family, port, address) == (2, 2222, bytes(4))
            # Fixed-size unicast heartbeat, multicast payload, scheduled priority, no Run/Idle header.
            if service == 0x54:
                assert struct.unpack_from("<IHIH", cip, 28) == (30000, 0x4802, 30000, 0x2822)
            else:
                assert struct.unpack_from("<IIII", cip, 28) == (30000, 0x48000002, 30000, 0x28000022)
        header = bytearray(cip[:path_at])
        header[size_at] = len(target_path) // 2
        return rr_reply(header + target_path, *items.items()), {
            "service": service, "role": role, "path": path.hex(),
        }


@contextmanager
def route_bridge():
    with ThreadingTCPServer((ADAPTER, 44818), BridgeRequest) as bridge:
        bridge.events = []
        worker = threading.Thread(target=bridge.serve_forever, kwargs={"poll_interval": 0.05})
        worker.start()
        try:
            yield bridge.events
        finally:
            bridge.shutdown()
            worker.join(timeout=5)
            assert not worker.is_alive()


def check_routed_listener(cli, adapter, eds, root, seconds):
    root.mkdir(parents=True)
    standard, large = root / "listener.toml", root / "large-listener.toml"
    source = ('io_route = "1/3"\nlisten_only = true\ninput_multicast = true\n'
              + PROFILE.read_text(encoding="utf-8").replace("output_instance = 152", "output_instance = 153")
              .replace("[app]\n", "[app]\nauto_reconnect = true\n"))
    standard.write_text(source, encoding="utf-8")
    large.write_text("large_forward_open = true\n" + source, encoding="utf-8")
    identity = "1:12:65001:123456789"
    preflight = call(cli, root, "preflight", "validate", standard, eds, "Connection3")
    assert preflight["connection_valid"] and preflight["listen_only"] and preflight["input_multicast"]
    assert preflight["io_route"] == "1/3" and preflight["output"]["payload_bytes"] == 0
    assert not preflight["requests_output_ownership"] and not preflight["asserts_run"]
    assert preflight["configuration_bytes"] == 0

    def expect_no_controller(name):
        result = subprocess.run([cli, "capture", standard, root / name, "1", identity],
                                capture_output=True, timeout=10, creationflags=subprocess.CREATE_NO_WINDOW)
        (root / f"{name}.toml").write_bytes(result.stdout)
        report = tomllib.loads(result.stdout.decode("utf-8"))
        assert result.returncode == 1 and report["io_route"] == "1/3"
        assert "0x0119" in report["cip_error"] and "no controlling connection" in report["cip_error"]
        assert report["sent"] == report["written"] == 0
        assert all(key not in report for key in ("attempts", "cyclic_io", "forward_close"))

    with run_adapter(adapter, root, tcp_port=BACKEND_PORT), route_bridge() as trace:
        expect_no_controller("before-controller")
        with controlling_heartbeat(BRIDGE_ROUTE) as producer:
            last_sequence = None
            for label, profile in [("standard", standard), ("large", large)]:
                receipt = call(cli, root, f"{label}-result", "capture", profile, root / label,
                               str(seconds), identity, timeout=seconds + 12)
                samples = check_capture(receipt, seconds)
                if last_sequence is not None:
                    assert int(samples[0][4]) > last_sequence
                last_sequence = int(samples[-1][4])
                assert receipt["io_route"] == "1/3" and receipt["identity"]["serial"] == 123456789
                assert receipt["listen_only"] and "attempts" not in receipt
                assert receipt["sent"] > 0
                endpoints = receipt["cyclic_io"]
                assert endpoints["input_local_udp"] == "127.0.0.1:2222"
                assert endpoints["input_multicast_destination"] == producer["group"]
                assert endpoints["to_connection_id"] == producer["to"]
                assert endpoints["ot_connection_id"] != producer["ot"]
                assert endpoints["first_input_source"].startswith(ADAPTER + ":")
        expect_no_controller("after-controller")

    (root / "bridge.json").write_text(json.dumps(trace, indent=2) + "\n", encoding="utf-8")
    connection_requests = [entry for entry in trace if entry["service"] != 0x52]
    assert [(e["service"], e["role"], e["general_status"], e["extended_status"])
            for e in connection_requests] == [
        (0x54, "listener", 1, [0x0119]),
        (0x54, "controller", 0, []),
        (0x54, "listener", 0, []),
        (0x4E, "listener", 0, []),
        (0x5B, "listener", 0, []),
        (0x4E, "listener", 0, []),
        (0x4E, "controller", 0, []),
        (0x54, "listener", 1, [0x0119]),
    ]
    print("Routed standard and Large listen-only shared the same OpENer producer and confirmed close.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("exe", type=Path)
    parser.add_argument("checkout", type=Path, help="OpENer checkout with the pinned source")
    parser.add_argument("directory", type=Path, help="New directory for build and reports")
    parser.add_argument("--seconds", type=int, default=3)
    args = parser.parse_args()
    if sys.platform != "win32" or not 2 <= args.seconds <= 30:
        parser.error("This loopback fixture requires Windows and a duration from 2 to 30 seconds")
    root = args.directory.resolve()
    root.mkdir(parents=True)
    adapter, eds = build_adapter(args.checkout.resolve(), root, shared_io_fixture=True, tcp_port=BACKEND_PORT)
    check_routed_listener(args.exe.resolve(), adapter, eds, root / "routed-listen-only", args.seconds)


if __name__ == "__main__":
    main()
