"""Verify all read-only exports against independent localhost clients on Windows."""

import argparse
from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager
import ctypes
from ctypes import wintypes
import http.client
import json
from pathlib import Path
import socket
import struct
import subprocess
import sys
import threading
import time
import tomllib
from uuid import uuid4

from test_cli import FIXTURE, NO_WINDOW, PROFILE, check_capture_window, parts_for, rows


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def get(port, path, method="GET", host=None):
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=1)
    try:
        connection.request(method, path, headers={"Host": host} if host else {})
        response = connection.getresponse()
        return response.status, json.loads(response.read())
    finally:
        connection.close()


def measure_reader_overlap(port):
    def request_diagnostics():
        started = time.perf_counter_ns()
        status, response = get(port, "/v1/connection")
        elapsed_ms = (time.perf_counter_ns() - started) / 1e6
        assert status == 200 and response["connection"]["state"] == "Receiving"
        return elapsed_ms

    with ThreadPoolExecutor(max_workers=1) as clients:
        with socket.create_connection(("127.0.0.1", port), timeout=1) as slow:
            slow.sendall(f"GET /v1/status HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n".encode())
            time.sleep(0.04)
            fast = clients.submit(request_diagnostics)
            time.sleep(0.14)
            completed_during_slow_request = fast.done()
            slow.sendall(b"\r\n")
            response = http.client.HTTPResponse(slow)
            response.begin()
            assert response.status == 200
            response.read()
            response.close()
        return completed_during_slow_request, fast.result(timeout=1)


def wait_for_connection(port, generation, actual):
    deadline = time.monotonic() + 2
    while time.monotonic() < deadline:
        status, response = get(port, "/v1/connection")
        assert status == 200 and response["schema_version"] == 1
        current = response["connection"]
        if (response["active_generation"] == generation and current["generation"] == generation
                and current["state"] == "Receiving" and current["receive_spacing"]["intervals"] > 0):
            assert response["adapter_ip"] == response["local_ip"] == "127.0.0.1"
            assert response["io_route"] is None and current["identity"]["serial"] == 305419896
            assert current["requested_ot_us"] == current["requested_to_us"] == 8000
            assert (current["actual_ot_us"], current["actual_to_us"]) == actual
            assert current["input_age_us"] >= 0 and current["accepted"] > 1
            assert current["connection_timeout"]["receive_watchdog_us"] == actual[1] * 32
            assert current["receive_timeout"] is None and current["forward_close"] is None
            assert current["network_error"] is None and current["cip_error"] is None
            assert current["cyclic_io"]["first_input_source"].startswith("127.0.0.1:")
            return response
        time.sleep(0.02)
    raise AssertionError(f"No live diagnostics for generation {generation}")


def read_exact(sock, size):
    data = bytearray()
    while len(data) < size:
        chunk = sock.recv(size - len(data))
        if not chunk:
            raise EOFError
        data.extend(chunk)
    return bytes(data)


def mqtt_packet(sock):
    header = read_exact(sock, 1)[0]
    length = 0
    for shift in range(0, 28, 7):
        digit = read_exact(sock, 1)[0]
        length |= (digit & 127) << shift
        if digit < 128:
            return header, read_exact(sock, length)
    raise ValueError("Invalid MQTT remaining length")


def receive_mqtt(listener, received, errors, disconnect_once=True):
    """Minimal independent broker fixture for MQTT 3.1.1 CONNECT and QoS 0."""
    try:
        for attempt in range(2 if disconnect_once else 1):
            sock, _ = listener.accept()
            with sock:
                sock.settimeout(7)
                header, body = mqtt_packet(sock)
                assert header == 0x10 and body[:7] == b"\x00\x04MQTT\x04"
                sock.sendall(b"\x20\x02\x00\x00")
                count = 0
                while True:
                    try:
                        header, body = mqtt_packet(sock)
                    except EOFError:
                        break
                    if header == 0xC0:
                        sock.sendall(b"\xD0\x00")
                        continue
                    assert header == 0x30
                    size, = struct.unpack_from(">H", body)
                    assert body[2:2 + size] == b"bench/test/samples"
                    received.append(json.loads(body[2 + size:]))
                    count += 1
                    # Force the publishing client to reconnect while scanning continues.
                    if disconnect_once and attempt == 0 and count == 3:
                        break
    except Exception as error:
        errors.append(error)


def windows_api():
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    for name, result, args in [
        ("OpenFileMappingW", wintypes.HANDLE, [wintypes.DWORD, wintypes.BOOL, wintypes.LPCWSTR]),
        ("OpenMutexW", wintypes.HANDLE, [wintypes.DWORD, wintypes.BOOL, wintypes.LPCWSTR]),
        ("MapViewOfFile", ctypes.c_void_p, [wintypes.HANDLE, wintypes.DWORD, wintypes.DWORD,
                                          wintypes.DWORD, ctypes.c_size_t]),
        ("UnmapViewOfFile", wintypes.BOOL, [ctypes.c_void_p]),
        ("WaitForSingleObject", wintypes.DWORD, [wintypes.HANDLE, wintypes.DWORD]),
        ("ReleaseMutex", wintypes.BOOL, [wintypes.HANDLE]),
        ("CloseHandle", wintypes.BOOL, [wintypes.HANDLE]),
    ]:
        function = getattr(kernel, name)
        function.restype = result
        function.argtypes = args
    return kernel


@contextmanager
def shared_memory(name):
    kernel = windows_api()
    prefix = rf"Local\EthernetIPWorkbench.{name}"
    file = lock = view = None
    try:
        file = kernel.OpenFileMappingW(4, False, prefix + ".data")
        lock = kernel.OpenMutexW(0x100001, False, prefix + ".lock")
        if not file or not lock:
            raise ctypes.WinError(ctypes.get_last_error())
        view = kernel.MapViewOfFile(file, 4, 0, 0, 573)
        if not view:
            raise ctypes.WinError(ctypes.get_last_error())
        yield kernel, lock, view
    finally:
        if view:
            kernel.UnmapViewOfFile(view)
        for handle in (file, lock):
            if handle:
                kernel.CloseHandle(handle)


def read_memory(kernel, lock, view):
    assert kernel.WaitForSingleObject(lock, 1000) == 0
    try:
        data = ctypes.string_at(view, 573)
    finally:
        assert kernel.ReleaseMutex(lock)
    magic, started, index, generation, active, elapsed, packet, cip, assembly, size = struct.unpack_from(
        "<8sqQQQQIHHH", data
    )
    assert magic == b"EIPWB001" and 0 <= size <= 509
    return {
        "stream_start_utc_us": started, "sample_index": index, "generation": generation,
        "active_generation": active, "elapsed_us": elapsed, "packet_sequence": packet,
        "cip_sequence": cip, "input_assembly": assembly, "payload_hex": data[64:64 + size].hex(" ").upper(),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("exe", type=Path)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--serve", action="store_true", help="Exercise the feed-only CLI")
    args = parser.parse_args()
    root = args.directory.resolve()
    root.mkdir(parents=True)
    http_port = free_port()
    name = f"test-{uuid4().hex}"
    profile = root / "profile.toml"
    source = PROFILE.read_text(encoding="utf-8").replace("rpi_us = 1000", "rpi_us = 8000")
    cli = args.exe.resolve()
    if args.serve:
        source = "log_fields = []\n" + source
        disabled = root / "no-feeds.toml"
        disabled.write_text(source, encoding="utf-8")
        result = subprocess.run([cli, "serve", disabled, "0:12:1", "1"], capture_output=True,
                                encoding="utf-8", timeout=5, creationflags=NO_WINDOW)
        assert result.returncode == 1 and "requires at least one" in result.stderr
    with socket.socket() as broker:
        broker.bind(("127.0.0.1", 0))
        broker.listen()
        broker.settimeout(7)
        profile.write_text(source + f"""
[app]
auto_reconnect = true
[app.integration]
http_port = {http_port}
shared_memory = "{name}"
[app.integration.mqtt]
broker = "127.0.0.1"
port = {broker.getsockname()[1]}
topic = "bench/test/samples"
period_ms = 25
""", encoding="utf-8")
        invalid = root / "invalid-profile.toml"
        invalid.write_text(profile.read_text(encoding="utf-8").replace("period_ms = 25", "period_ms = 0"),
                           encoding="utf-8")
        rejected = subprocess.run(
            [cli, "validate", invalid], capture_output=True, encoding="utf-8",
            timeout=5, creationflags=NO_WINDOW,
        )
        assert rejected.returncode == 1 and "MQTT publish period" in rejected.stdout
        received, errors = [], []
        worker = threading.Thread(target=receive_mqtt, args=(broker, received, errors), daemon=True)
        worker.start()
        mode = "recover-slower-api" if args.serve else "recover"
        fixture = subprocess.Popen([sys.executable, FIXTURE, root / "adapter.json", "--mode", mode],
                                   stdout=subprocess.PIPE, encoding="utf-8", creationflags=NO_WINDOW)
        capture = None
        try:
            assert fixture.stdout.readline().strip() == "Ready on loopback"
            expected = "0:12:1:305419896"
            command = ([cli, "serve", profile, expected, "5"] if args.serve else
                       [cli, "capture", profile, root / "capture", "5", expected])
            capture = subprocess.Popen(command, cwd=root, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                       encoding="utf-8", creationflags=NO_WINDOW)
            deadline = time.monotonic() + 2
            while True:
                try:
                    status, latest = get(http_port, "/v1/latest")
                    if status == 200 and latest["sample"]:
                        break
                except (OSError, http.client.HTTPException):
                    pass
                assert time.monotonic() < deadline
                time.sleep(0.02)
            assert get(http_port, "/v1/latest", "POST")[0] == 405
            assert get(http_port, "/v1/latest", host="other.invalid")[0] == 400
            assert get(http_port, "/v1/connection", "POST")[0] == 405
            assert get(http_port, "/v1/connection", host="other.invalid")[0] == 400
            assert set(get(http_port, "/v1/status")[1]) == {
                "schema_version", "stream_start_utc_us", "active_generation", "counters",
            }
            first_connection = wait_for_connection(http_port, 1, (8000, 8000))
            assert first_connection["previous_connection"] is None
            (root / "connection-1.json").write_text(json.dumps(first_connection, indent=2), encoding="utf-8")
            with shared_memory(name) as memory:
                before = read_memory(*memory)
                assert before["generation"] == 1

                samples = []
                cursor = 0
                deadline = time.monotonic() + 3
                while time.monotonic() < deadline:
                    status, batch = get(http_port, f"/v1/samples?after={cursor}")
                    assert status == 200
                    samples.extend(batch["samples"])
                    if samples:
                        cursor = samples[-1]["sample_index"]
                        if samples[-1]["generation"] == 2:
                            break
                    time.sleep(0.02)
                assert {sample["generation"] for sample in samples} == {1, 2}
                assert [sample["sample_index"] for sample in samples] == list(range(1, cursor + 1))
                by_index = {sample["sample_index"]: sample for sample in samples}
                for key, value in before.items():
                    if key != "active_generation":
                        assert by_index[before["sample_index"]][key] == value, key
                assert all(int(sample["values"]["value"]) == int.from_bytes(
                    bytes.fromhex(sample["payload_hex"]), "little"
                ) for sample in samples)
                second_connection = wait_for_connection(http_port, 2, (10000, 12000) if args.serve else (8000, 8000))
                previous = second_connection["previous_connection"]
                assert previous["generation"] == 1 and previous["state"] == "Timed out"
                assert previous["timeouts"] == 1 and previous["receive_timeout"]["limit_us"] == 256000
                assert previous["forward_close"] == {"outcome": "confirmed"}
                assert previous["accepted"] >= first_connection["connection"]["accepted"]
                assert first_connection["stream_start_utc_us"] == second_connection["stream_start_utc_us"]
                (root / "connection-2.json").write_text(json.dumps(second_connection, indent=2), encoding="utf-8")
                fast_completed, latency_ms = measure_reader_overlap(http_port)
                assert fast_completed, f"Slow request blocked a second HTTP reader for {latency_ms:.1f} ms"
                print(f"Second HTTP reader completed in {latency_ms:.1f} ms while a request was still open.")
                kernel, lock, _ = memory
                assert kernel.WaitForSingleObject(lock, 1000) == 0
                try:
                    time.sleep(0.08)
                    status, later = get(http_port, "/v1/latest")
                    assert status == 200 and later["sample"]["sample_index"] > cursor
                finally:
                    assert kernel.ReleaseMutex(lock)
                with socket.create_connection(("127.0.0.1", http_port), timeout=1) as slow_http:
                    slow_http.sendall(b"GET /v1/connection")
                    time.sleep(0.12)
                    assert read_memory(*memory)["sample_index"] > later["sample"]["sample_index"]
                stdout, stderr = capture.communicate(timeout=8)
                assert capture.returncode == 0, (stdout, stderr)
                assert read_memory(*memory)["active_generation"] == 0
            assert fixture.wait(timeout=5) == 0
            worker.join(timeout=3)
            assert not worker.is_alive() and not errors, errors
        finally:
            for process in (capture, fixture):
                if process is not None and process.poll() is None:
                    process.terminate()
                    process.wait(timeout=5)

    report = tomllib.loads(stdout)
    stats = report["integration"]
    assert stats["received"] == report["accepted"]
    assert stats["queue_drops"] == 0
    if args.serve:
        (root / "serve.toml").write_text(stdout, encoding="utf-8")
        assert report["mode"] == "serve" and report["state"] == "Closed" and report["duration_reached"]
        assert report["identity"]["serial"] == 305419896
        assert report["requested_ot_us"] == report["requested_to_us"] == 8000
        assert [run["generation"] for run in report["attempts"]] == [1, 2]
        assert all(run["forward_close"]["outcome"] == "confirmed" for run in report["attempts"])
        assert report["attempts"][0]["state"] == "Timed out" and report["attempts"][1]["state"] == "Closed"
        assert not (root / "captures").exists() and not list(root.rglob("*.csv"))
        print(f"Served {report['accepted']} valid packets without a CSV writer. Export counters {stats}.")
    else:
        check_capture_window(report, list(rows(parts_for(Path(report["csv_prefix"])))))
        assert report["buffer_overruns"] == 0
        print(f"CSV wrote {report['written']} rows. Export counters {stats}.")
    assert stats["shared_memory_busy"] > 0 and stats["mqtt_errors"] > 0
    assert {message["generation"] for message in received} == {1, 2}
    assert all(message["stream_start_utc_us"] == before["stream_start_utc_us"] for message in received)
    print("HTTP connection diagnostics preserved the previous timeout and Forward_Close.")
    print(f"HTTP and shared memory verified across reconnect. MQTT published {len(received)} samples.")


if __name__ == "__main__":
    main()
