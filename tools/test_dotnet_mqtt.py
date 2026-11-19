"""Check the C# MQTT subscriber with the scanner and a loopback broker fixture."""

import argparse
from contextlib import ExitStack
import json
from pathlib import Path
import socket
import socketserver
import struct
import threading
import time
import tomllib

from test_dotnet_consumer import frame, verify_history
from test_integration import free_port, mqtt_packet
from test_recovery import PROFILE, adapter, fixture_report, process

TOPIC = "bench/dotnet/samples"
TOPIC_BYTES = TOPIC.encode()
TOPIC_FIELD = struct.pack(">H", len(TOPIC_BYTES)) + TOPIC_BYTES
SAMPLES = 50


def encode(header, body):
    size = len(body)
    packet = bytearray([header])
    while size >= 128:
        packet.append((size % 128) | 128)
        size //= 128
    packet.append(size)
    return bytes(packet) + body


class Connection(socketserver.BaseRequestHandler):
    def handle(self):
        self.request.settimeout(8)
        server = self.server
        try:
            header, body = mqtt_packet(self.request)
            assert header == 0x10 and body[:7] == b"\x00\x04MQTT\x04"
            name_length, = struct.unpack_from(">H", body, 10)
            name = body[12:12 + name_length]
            assert name.startswith(b"eipwb-")
            subscriber = name.startswith(b"eipwb-net-")
            self.request.sendall(b"\x20\x02\x00\x00")
            while True:
                header, body = mqtt_packet(self.request)
                if header == 0x82 and subscriber:
                    assert body[2:] == TOPIC_FIELD + b"\x00"
                    server.subscribe(self.request, body[:2])
                elif header == 0x30 and not subscriber:
                    assert body.startswith(TOPIC_FIELD)
                    server.forward(body)
                elif header == 0xC0:
                    self.request.sendall(b"\xD0\x00")
                elif header == 0xE0:
                    break
                else:
                    raise AssertionError(f"Unexpected MQTT packet {header:#x}")
        except (EOFError, ConnectionResetError, ConnectionAbortedError):
            pass
        except BrokenPipeError as error:
            if getattr(error, "winerror", None) != 10058 or self.request is not server.forced_reader:
                server.errors.append(error)
        except Exception as error:
            server.errors.append(error)
        finally:
            with server.lock:
                if server.reader is self.request:
                    server.reader = None


class Broker(socketserver.ThreadingTCPServer):
    def __init__(self):
        super().__init__(("127.0.0.1", 0), Connection)
        self.lock = threading.Lock()
        self.ready = threading.Event()
        self.reader = None
        self.subscriptions = 0
        self.first_forwarded = 0
        self.forced_reader = None
        self.errors = []

    def subscribe(self, reader, packet_id):
        with self.lock:
            self.subscriptions += 1
            reader.sendall(b"\x90\x03" + packet_id + b"\x00")
            now = time.time_ns() // 1000
            ignored = {
                "schema_version": 1, "stream_start_utc_us": now - 1000, "sample_index": 9001,
                "generation": 1, "elapsed_us": 0, "packet_sequence": 1, "cip_sequence": 1,
                "input_assembly": 101, "payload_hex": "00 00 00 00", "values": {"counter": "0"},
            }
            for header, sample in (
                (0x31, ignored),
                (0x30, ignored | {"stream_start_utc_us": now - 60_000_000}),
                (0x30, ignored | {"schema_version": 2}),
            ):
                reader.sendall(encode(header, TOPIC_FIELD + json.dumps(sample).encode()))
            self.reader = reader
            self.ready.set()

    def forward(self, payload):
        with self.lock:
            if self.reader is None:
                return
            try:
                self.reader.sendall(encode(0x30, payload))
            except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
                self.reader = None
                return
            if self.subscriptions == 1:
                self.first_forwarded += 1
                if self.first_forwarded == 3:
                    self.forced_reader = self.reader
                    self.reader = None
                    self.forced_reader.shutdown(socket.SHUT_RDWR)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("scanner", type=Path)
    parser.add_argument("mqtt_reader_dll", type=Path)
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    exe = args.scanner.resolve(strict=True)
    dll = args.mqtt_reader_dll.resolve(strict=True)
    root = args.directory.resolve()
    root.mkdir(parents=True)
    http_port = free_port()
    with Broker() as broker:
        worker = threading.Thread(target=broker.serve_forever, kwargs={"poll_interval": 0.05})
        worker.start()
        try:
            broker_port = broker.server_address[1]
            profile = root / "profile.toml"
            source = PROFILE.with_name("mapped-loopback.toml").read_text(encoding="utf-8")
            profile.write_text(source.replace("rpi_us = 1000", "rpi_us = 8000") + f"""

[app]
auto_reconnect = true
[app.integration]
http_port = {http_port}
[app.integration.mqtt]
broker = "127.0.0.1"
port = {broker_port}
topic = "{TOPIC}"
period_ms = 25
""", encoding="utf-8")
            with ExitStack() as stack:
                reader = stack.enter_context(process(["dotnet", dll, "127.0.0.1", str(broker_port), TOPIC, str(SAMPLES)]))
                assert broker.ready.wait(timeout=4), "MQTT subscriber did not become ready"
                fixture = adapter(stack, root, "source", "127.0.0.1", "recover")
                scanner = stack.enter_context(process([exe, "serve", profile, "0:12:1:305419896", "6"]))
                stdout, stderr = reader.communicate(timeout=8)
                assert reader.returncode == 0, (stdout, stderr, broker.errors)
                (root / "mqtt.jsonl").write_text(stdout, encoding="utf-8")
                (root / "subscriber.log").write_text(stderr, encoding="utf-8")
                indexes = {}
                previous = 0
                for record in map(json.loads, stdout.splitlines()):
                    sample = record["sample"]
                    index = sample["sample_index"]
                    assert index > previous and record["transport"] == "mqtt" and record["topic"] == TOPIC
                    assert 0 <= record["estimated_age_us"] <= 500000
                    assert record["skipped_export_indexes"] == (index - previous - 1 if previous else 0)
                    assert set(sample["values"]) == {"counter", "low_bit", "swapped_word"}
                    assert int(sample["values"]["counter"]) == int.from_bytes(bytes.fromhex(sample["payload_hex"]), "little")
                    indexes[index] = frame(sample)
                    previous = index
                assert len(indexes) == SAMPLES and {value[1] for value in indexes.values()} == {1, 2}
                assert len({value[0] for value in indexes.values()}) == 1
                verify_history(http_port, (indexes,))
                scanner_stdout, scanner_stderr = scanner.communicate(timeout=8)
                assert scanner.returncode == 0, (scanner_stdout, scanner_stderr)
                assert all(run["forward_close"] for run in fixture_report(fixture)["attempts"])
        finally:
            broker.shutdown()
            worker.join(timeout=2)
            assert not worker.is_alive()
        assert not broker.errors, broker.errors
        assert broker.forced_reader is not None and broker.subscriptions == 2

    (root / "serve.toml").write_text(scanner_stdout, encoding="utf-8")
    report = tomllib.loads(scanner_stdout)
    assert report["sequence_gaps"] == report["integration"]["queue_drops"] == 0
    assert report["integration"]["mqtt_errors"] == 0
    print(f"C# MQTT resubscribed after a broker disconnect and matched {SAMPLES} samples across two Class 1 generations.")
    print("Injected retained, stale and unsupported-schema messages were excluded.")


if __name__ == "__main__":
    main()
