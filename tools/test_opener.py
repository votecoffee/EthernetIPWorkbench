"""Build the pinned OpENer sample adapter and test Class 1 input-only capture on Windows."""
import argparse
from contextlib import contextmanager
from io import BytesIO
from ipaddress import IPv4Address
import json
from pathlib import Path
import socket
import struct
import subprocess
import sys
import threading
import time
import tomllib
import zipfile

from join_capture import METADATA, parts_for, rows
from loopback_adapter import io_packet, recv_exact, rr_items, rr_reply, sockaddr

HERE = Path(__file__).resolve().parent
UPSTREAM = "0e1ab7ee5c272950c0dc8daf7793008d7a518912"
ADAPTER = "127.0.0.2"
PROFILE = HERE.parent / "samples" / "opener-loopback.toml"
READY = "OpENer loopback ready"


def adapt_shared_io_for_windows_fixture(ports):
    """Keep the fixture's single shared UDP handle alive across per-path closes."""
    def replace_once(path, old, new):
        original = path.read_bytes()
        if b"\r\n" in original:
            old, new = (part.replace(b"\n", b"\r\n") for part in (old, new))
        if original.count(old) != 1:
            raise RuntimeError(f"Review the pinned Windows shared I/O fixture patch for {path.name}")
        path.write_bytes(original.replace(old, new))

    replace_once(ports / "networkhandler.h",
                 b"#define OPENER_SOCKET_WOULD_BLOCK EWOULDBLOCK",
                 b"#define OPENER_SOCKET_WOULD_BLOCK WSAEWOULDBLOCK")
    handler = ports / "generic_networkhandler.c"
    replace_once(handler, b"EipStatus NetworkHandlerInitialize(void) {",
                 b"EipStatus NetworkHandlerInitialize(void) {\n"
                 b"  g_network_status.udp_io_messaging = kEipInvalidSocket;")
    replace_once(handler, b"int CreateUdpSocket(void) {",
                 b"int CreateUdpSocket(void) {\n"
                 b"  if(g_network_status.udp_io_messaging != kEipInvalidSocket) {\n"
                 b"    return g_network_status.udp_io_messaging;\n  }")
    replace_once(handler, b"EipStatus NetworkHandlerFinish(void) {",
                 b"EipStatus NetworkHandlerFinish(void) {\n"
                 b"  CloseUdpSocket(g_network_status.udp_io_messaging);\n"
                 b"  g_network_status.udp_io_messaging = kEipInvalidSocket;")
    connection = ports.parent / "cip/cipioconnection.c"
    old = b"""  if(kEipInvalidSocket !=
     connection_object->socket[kUdpCommuncationDirectionConsuming]) {
    CloseUdpSocket(connection_object->socket[kUdpCommuncationDirectionConsuming]);
  }

  if(kEipInvalidSocket !=
     connection_object->socket[kUdpCommuncationDirectionProducing]) {
    CloseUdpSocket(connection_object->socket[kUdpCommuncationDirectionProducing]);
  }"""
    replace_once(connection, old, b"  /* The Windows loopback fixture owns the shared UDP socket. */")


def build_adapter(checkout, root, *, shared_io_fixture=False):
    archive = subprocess.check_output(
        ["git", "-C", checkout, "archive", "--format=zip", UPSTREAM],
        creationflags=subprocess.CREATE_NO_WINDOW,
    )
    source = root / "upstream"
    with zipfile.ZipFile(BytesIO(archive)) as bundle:
        bundle.extractall(source)
    ports = source / "source/src/ports"
    main = HERE / "fixtures/opener_loopback.c"
    (ports / "WIN32/main.c").write_bytes(main.read_bytes())

    handler = ports / "generic_networkhandler.c"
    data = handler.read_bytes()
    wildcard = b".sin_addr.s_addr = htonl(INADDR_ANY)"
    if data.count(wildcard) != 2:
        raise RuntimeError("Review OpENer socket bindings before running the adapter")
    handler.write_bytes(data.replace(wildcard, b".sin_addr.s_addr = g_network_status.ip_address"))
    if shared_io_fixture:
        adapt_shared_io_for_windows_fixture(ports)

    build = root / "build"
    log = root / "build.log"
    commands = [
        ["cmake", "-S", source / "source", "-B", build, "-G", "Visual Studio 17 2022", "-A", "x64",
         "-DOpENer_PLATFORM=WIN32", "-DCMAKE_C_STANDARD=11", "-DOpENer_TRACES=ON"],
        ["cmake", "--build", build, "--config", "Release", "--parallel", "8"],
    ]
    with log.open("xb") as stream:
        for command in commands:
            proc = subprocess.run(command, stdout=stream, stderr=subprocess.STDOUT,
                                  creationflags=subprocess.CREATE_NO_WINDOW, timeout=180)
            if proc.returncode != 0:
                raise RuntimeError(f"OpENer build failed. See {log}")
    return build / "src/ports/WIN32/Release/OpENer.exe", source / "data/opener_sample_app.eds"


@contextmanager
def run_adapter(exe, root):
    for kind, port in [(socket.SOCK_STREAM, 44818), (socket.SOCK_DGRAM, 44818), (socket.SOCK_DGRAM, 2222)]:
        with socket.socket(socket.AF_INET, kind) as check:
            check.setsockopt(socket.SOL_SOCKET, socket.SO_EXCLUSIVEADDRUSE, 1)
            check.bind((ADAPTER, port))
    log = root / "adapter.log"
    with log.open("xb") as stream, subprocess.Popen(
        [exe], cwd=root, stdin=subprocess.PIPE, stdout=stream, stderr=subprocess.STDOUT,
        creationflags=subprocess.CREATE_NO_WINDOW,
    ) as adapter:
        try:
            deadline = time.monotonic() + 5
            while READY not in log.read_text(encoding="utf-8", errors="replace"):
                if adapter.poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError(f"OpENer did not start. See {log}")
                time.sleep(0.05)
            yield log
        finally:
            if adapter.poll() is None:
                try:
                    adapter.communicate(b"\n", timeout=5)
                except subprocess.TimeoutExpired:
                    adapter.kill()
                    adapter.wait(timeout=5)
        if adapter.returncode != 0:
            raise RuntimeError(f"OpENer exited with code {adapter.returncode}. See {log}")


def call(cli, root, label, *args, timeout=10):
    result = subprocess.run([cli, *args], capture_output=True, timeout=timeout,
                            creationflags=subprocess.CREATE_NO_WINDOW)
    (root / f"{label}.toml").write_bytes(result.stdout)
    if result.returncode != 0:
        (root / f"{label}.stderr.txt").write_bytes(result.stderr)
        raise RuntimeError(f"{label} exited with code {result.returncode}. See {root}")
    return tomllib.loads(result.stdout.decode("utf-8"))


def check_capture(report, seconds):
    assert report["state"] == "Closed" and report["duration_reached"]
    assert report["forward_close"] == {"outcome": "confirmed"}
    assert report["input_assembly"] == 100 and report["input_size"] == 32
    assert report["last_input_hex"] == bytes(range(32)).hex(" ").upper()
    assert all(report[key] == 30000 for key in (
        "requested_ot_us", "requested_to_us", "actual_ot_us", "actual_to_us",
    ))
    assert not any(report[key] for key in ("timeouts", "invalid", "buffer_overruns"))
    assert "disk_error" not in report
    parts = parts_for(Path(report["csv_prefix"]))
    samples = list(rows(parts, METADATA + ["first_word_le", "last_word_be"]))
    assert len(samples) == report["written"] == report["seen"] >= seconds * 30
    after_deadline = report["after_capture_deadline"]
    assert report["accepted"] == report["seen"] + after_deadline
    assert seconds <= 10 or len(parts) > 1
    assert all((int(row[2]), int(row[3]), int(row[6]), int(row[7])) ==
               (index, 1, 0x03020100, 0x1C1D1E1F) for index, row in enumerate(samples))
    elapsed = [int(row[1]) for row in samples]
    assert elapsed[-1] - elapsed[0] <= seconds * 1_000_000
    gaps = sorted(later - earlier for earlier, later in zip(elapsed, elapsed[1:]))
    receive = report["receive_spacing"]
    assert receive["intervals"] == len(gaps) + after_deadline
    if after_deadline == 0:
        assert abs(receive["max_us"] - gaps[-1]) <= 1
        for pct in (50, 95, 99):
            expected = gaps[(len(gaps) * pct + 99) // 100 - 1]
            # CSV elapsed times are truncated separately. The coarse histogram rounds upward.
            assert expected - 1 <= receive[f"p{pct}_us"] <= expected + expected // 256 + 1
    print(f"{len(samples)} rows in {len(parts)} parts, {report['sequence_gaps']} sequence gaps, "
          f"{report['buffer_overruns']} overruns")
    print(f"Receive spacing (µs) {receive}")
    print(f"Send spacing (µs) {report['send_spacing']}")


def check_explore(cli, adapter, root, identity, seconds):
    root.mkdir()
    keyed = root / "keyed-large.toml"
    key = {
        "vendor": identity["vendor"], "device_type": identity["device_type"],
        "product": identity["code"], "major": identity["major"], "minor": identity["minor"],
    }
    keyed.write_text(
        "large_forward_open = true\n" + PROFILE.read_text(encoding="utf-8") +
        "\n[electronic_key]\n" + "".join(f"{field} = {value}\n" for field, value in key.items()),
        encoding="utf-8",
    )
    with run_adapter(adapter, root):
        for label, args in [
            ("discovery", ["discover", "127.0.0.1", ADAPTER]),
            ("read-product", ["read", ADAPTER, "127.0.0.1", "1", "1", "7"]),
        ]:
            result = subprocess.run([cli, *args], capture_output=True, check=True, timeout=6,
                                    creationflags=subprocess.CREATE_NO_WINDOW)
            (root / f"{label}.json").write_bytes(result.stdout)
        expected = ":".join(str(identity[field]) for field in ("vendor", "device_type", "code", "serial"))
        report = call(cli, root, "result", "capture", keyed, root / "capture",
                      str(seconds), expected, timeout=seconds + 15)
    discovery = json.loads((root / "discovery.json").read_bytes())
    device, = discovery["devices"]
    assert device["identity"]["serial"] == identity["serial"]
    assert json.loads((root / "read-product.json").read_bytes())["decoded"] == identity["name"]
    check_capture(report, seconds)
    print("OpENer discovery, explicit product-name read and keyed Large Forward_Open passed.")

    assert key["minor"] > 1
    key["minor"] -= 1
    older_key = PROFILE.read_text(encoding="utf-8") + "\n[electronic_key]\n" + "".join(
        f"{field} = {value}\n" for field, value in key.items())
    compatible = root / "compatible-older"
    compatible.mkdir()
    profile = compatible / "profile.toml"
    profile.write_text(older_key + "compatible = true\n", encoding="utf-8")
    with run_adapter(adapter, compatible):
        report = call(cli, compatible, "result", "capture", profile, compatible / "capture",
                      str(seconds), expected, timeout=seconds + 15)
    assert report["identity"] == identity
    check_capture(report, seconds)

    exact = root / "exact-older"
    exact.mkdir()
    profile = exact / "profile.toml"
    profile.write_text(older_key, encoding="utf-8")
    with run_adapter(adapter, exact):
        rejected = subprocess.run([cli, "capture", profile, exact / "capture", str(seconds), expected],
                                  capture_output=True, timeout=seconds + 15,
                                  creationflags=subprocess.CREATE_NO_WINDOW)
    (exact / "result.toml").write_bytes(rejected.stdout)
    document = tomllib.loads(rejected.stdout.decode("utf-8"))
    assert rejected.returncode == 1 and document["identity"] == identity
    assert document["cip_error"] == "CIP general 0x01, extended [0x0116]"
    assert document["written"] == document["sent"] == 0 and "cyclic_io" not in document
    print(f"OpENer {key['major']}.{identity['minor']} accepted compatible {key['major']}.{key['minor']} "
          "and rejected the same revision in exact mode.")


def check_multicast(cli, adapter, root, identity, seconds):
    root.mkdir()
    profile = root / "multicast.toml"
    profile.write_text("input_multicast = true\n" + PROFILE.read_text(encoding="utf-8"),
                       encoding="utf-8")
    expected = ":".join(str(identity[field]) for field in ("vendor", "device_type", "code", "serial"))
    with run_adapter(adapter, root):
        report = call(cli, root, "result", "capture", profile, root / "capture",
                      str(seconds), expected, timeout=seconds + 15)
    check_capture(report, seconds)
    endpoints = report["cyclic_io"]
    group, port = endpoints["input_multicast_destination"].split(":")
    assert IPv4Address(group).is_multicast and port == "2222"
    assert endpoints["input_local_udp"] == "127.0.0.1:2222"
    print(f"OpENer selected multicast group {group}:{port}.")


def check_connected_reads(cli, adapter, root, identity):
    root.mkdir()
    source = PROFILE.read_text(encoding="utf-8")
    for name, rejected in (("success", False), ("mixed", True)):
        case = root / name
        case.mkdir()
        profile = case / "profile.toml"
        requests = [("Product name", 7)]
        if rejected:
            requests.append(("Unsupported attribute", 127))
        requests.append(("Serial number", 6))
        profile.write_text(source + "".join(
            f'\n[[app.cip_reads]]\nname = "{label}"\npath = {{ class = 1, instance = 1, attribute = {attribute} }}\n'
            for label, attribute in requests
        ), encoding="utf-8")
        with run_adapter(adapter, case) as log:
            result = subprocess.run([cli, "read-saved-all", profile, "--connected"],
                                    capture_output=True, timeout=10, creationflags=subprocess.CREATE_NO_WINDOW)
        (case / "result.json").write_bytes(result.stdout)
        assert result.returncode == int(rejected), (result.stdout, result.stderr)
        report = json.loads(result.stdout)
        replies = report["results"]
        assert report["forward_close"] == {"outcome": "confirmed"}
        assert [reply["name"] for reply in replies] == [item[0] for item in requests]
        assert replies[0]["decoded"] == identity["name"]
        assert replies[-1]["decoded"] == f"0x{identity['serial']:08X}"
        assert all(reply["status"] == "success" for reply in (replies[0], replies[-1]))
        if rejected:
            assert replies[1]["status"] == "cip_rejected" and replies[1]["general_status"] == 0x14
        trace = log.read_text(encoding="utf-8", errors="replace")
        assert trace.count("Class 3 sequence number:") == len(requests)
        assert "ForwardClose: ConnSerNo" in trace
    print("OpENer Class 3 direct reads, partial rejection, sequence tracking and Forward_Close passed.")


@contextmanager
def controlling_heartbeat():
    """Independent loopback originator. Its input stream is observed by the listener."""
    local, config, out, inp = "127.0.0.1", 151, 152, 100
    serial, vendor, originator = 0x45CE, 0, 0x6D6B7212
    path = bytes((0x20, 4, 0x24, config, 0x2C, out, 0x2C, inp))
    timeout = b"\x0a\x0a"
    header = struct.Struct("<HHII8sI")
    with socket.socket() as tcp, socket.socket(type=socket.SOCK_DGRAM) as udp:
        tcp.bind((local, 0))
        tcp.settimeout(2)
        tcp.connect((ADAPTER, 44818))
        udp.bind((local, 0))
        handle, sequence = 0, 0

        def exchange(command, payload):
            nonlocal sequence
            sequence += 1
            context = sequence.to_bytes(8, "little")
            tcp.sendall(header.pack(command, len(payload), handle, 0, context, 0) + payload)
            cmd, size, session, status, ctx, options = header.unpack(recv_exact(tcp, header.size))
            assert cmd == command and ctx == context and status == options == 0
            assert not handle or session == handle
            return session, recv_exact(tcp, size)

        def cip(service, payload, *items):
            request = bytes((service, 2, 0x20, 6, 0x24, 1)) + payload
            _, response = exchange(0x6F, rr_reply(request, *items))
            parsed = rr_items(response)
            assert parsed[0xB2][:4] == bytes((service | 0x80, 0, 0, 0)), parsed
            return parsed

        register = b"\x01\x00\x00\x00"
        handle, data = exchange(0x65, register)
        assert handle and data == register
        body = timeout + struct.pack("<IIHHIB3xIHIHBB", 0, 0x7AB89F01, serial, vendor,
                                     originator, 3, 30000, 0x4802, 30000, 0x2822, 1, len(path) // 2)
        opened = cip(0x54, body + path, (0x8001, sockaddr("0.0.0.0", 2222)))
        ot, to, cs, v, osn, ot_api, to_api = struct.unpack_from("<IIHHIII", opened[0xB2], 4)
        _, output_port = struct.unpack_from(">HH", opened[0x8000])
        _, group_port, group_addr = struct.unpack(">HH4s8x", opened[0x8001])
        assert (cs, v, osn, ot_api, to_api) == (serial, vendor, originator, 30000, 30000)
        stop = threading.Event()
        errors = []

        def send():
            try:
                seq = 1
                while not stop.is_set():
                    udp.sendto(io_packet(ot, seq, b""), (ADAPTER, output_port))
                    seq += 1
                    stop.wait(0.030)
            except OSError as error:
                errors.append(error)

        worker = threading.Thread(target=send, daemon=True)
        worker.start()
        try:
            yield {"ot": ot, "to": to, "group": f"{socket.inet_ntoa(group_addr)}:{group_port}"}
        finally:
            stop.set()
            worker.join(timeout=2)
            assert not worker.is_alive() and not errors, errors
            close = timeout + struct.pack("<HHIBB", serial, vendor, originator, len(path) // 2, 0)
            closed = cip(0x4E, close + path)
            assert closed[0xB2][4:12] == close[2:10]
            tcp.sendall(header.pack(0x66, 0, handle, 0, bytes(8), 0))


def check_listen_only(cli, adapter, root, eds, identity, seconds):
    root.mkdir()
    source = "input_multicast = true\n" + PROFILE.read_text(encoding="utf-8")
    listener_profile = root / "listener.toml"
    listener_profile.write_text(
        "listen_only = true\n" + source.replace("output_instance = 152", "output_instance = 153")
        .replace("[app]\n", "[app]\nauto_reconnect = true\n"),
        encoding="utf-8",
    )
    validated = call(cli, root, "listener-preflight", "validate", listener_profile, eds, "Connection3")
    assert validated["connection_valid"] and validated["listen_only"] and validated["input_multicast"]
    assert not validated["requests_output_ownership"] and not validated["asserts_run"]
    assert validated["configuration_bytes"] == 0 and validated["output"]["payload_bytes"] == 0
    assert {row["field"]: row["state"] for row in validated["origins"]}["Ownership"] == "eds"

    expected = ":".join(str(identity[field]) for field in ("vendor", "device_type", "code", "serial"))
    for suffix, old, new in (
        ("unicast", "input_multicast = true", "input_multicast = false"),
        ("owner", "input_only = true", "input_only = false"),
        ("run-header", "output_run_idle = false", "output_run_idle = true"),
        ("output-bytes", 'output_size = 0\nconfig_instance', 'output_size = 2\nconfig_instance'),
        ("config-bytes", 'config_size = 0\nconfig_hex = ""', 'config_size = 2\nconfig_hex = "AA BB"'),
    ):
        path = root / f"invalid-{suffix}.toml"
        original = listener_profile.read_text(encoding="utf-8")
        assert old in original
        path.write_text(original.replace(old, new), encoding="utf-8")
        invalid = subprocess.run([cli, "capture", path, root / suffix, "1", expected],
                                 capture_output=True, timeout=5, creationflags=subprocess.CREATE_NO_WINDOW)
        assert invalid.returncode == 1 and b"Listen-only requires" in invalid.stderr, invalid.stderr
        assert not (root / suffix).exists()

    qualify = subprocess.run(
        [cli, "qualify", listener_profile, root / "qualification", "loopback", expected, "1"],
        capture_output=True, timeout=5, creationflags=subprocess.CREATE_NO_WINDOW,
    )
    assert qualify.returncode == 1 and b"use capture for listen-only" in qualify.stderr
    assert not (root / "qualification").exists()

    with run_adapter(adapter, root):
        orphan = subprocess.run([cli, "capture", listener_profile, root / "no-controller", "1", expected],
                                capture_output=True, timeout=10, creationflags=subprocess.CREATE_NO_WINDOW)
        (root / "no-controller.toml").write_bytes(orphan.stdout)
        rejected = tomllib.loads(orphan.stdout.decode("utf-8"))
        assert orphan.returncode == 1 and rejected["listen_only"]
        assert "0x0119" in rejected["cip_error"] and "no controlling connection" in rejected["cip_error"]
        assert rejected["sent"] == rejected["written"] == 0
        assert all(key not in rejected for key in ("attempts", "cyclic_io", "forward_close"))

        with controlling_heartbeat() as producer:
            # A second listener must still see this producer after the first closes.
            for name in ("listener", "next-listener"):
                listener = call(cli, root, f"{name}-result", "capture", listener_profile,
                                root / name, str(seconds), expected, timeout=seconds + 10)
                check_capture(listener, seconds)
                assert listener["listen_only"] and "attempts" not in listener
                assert listener["sent"] > 0
                endpoints = listener["cyclic_io"]
                assert endpoints["input_multicast_destination"] == producer["group"]
                assert endpoints["to_connection_id"] == producer["to"]
                assert endpoints["ot_connection_id"] != producer["ot"]
                assert endpoints["input_local_udp"] == "127.0.0.1:2222"
    print("OpENer rejected the orphan listener, then shared the producer's group and T->O ID. "
          "A second listener read the same producer after the first closed.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("exe", type=Path, help="Path to eipmaster-cli.exe")
    parser.add_argument("checkout", type=Path, help="OpENer Git checkout containing the pinned commit")
    parser.add_argument("directory", type=Path, help="New directory for the adapter build and capture")
    parser.add_argument("--seconds", type=int, default=60)
    parser.add_argument("--explore", action="store_true", help="Also test discovery, explicit reads and keyed Large Forward_Open")
    parser.add_argument("--multicast", action="store_true", help="Also capture OpENer multicast T->O")
    parser.add_argument("--listen-only", action="store_true", help="Also attach a listener to a controlling multicast session")
    args = parser.parse_args()
    if sys.platform != "win32" or not 2 <= args.seconds <= 300:
        parser.error("This fixture requires Windows and a duration from 2 to 300 seconds")
    root = args.directory.resolve()
    root.mkdir(parents=True)
    cli = args.exe.resolve()
    adapter, eds = build_adapter(args.checkout.resolve(), root, shared_io_fixture=args.listen_only)
    imported = call(cli, root, "eds", "eds", eds)
    profile = next(p for p in imported["profiles"] if p["id"] == "Connection2")
    assert profile["supported"] and profile["fields"]["Ownership"] == "Input-only"
    checked = call(cli, root, "preflight", "validate", PROFILE, eds, "Connection2")
    assert checked["connection_valid"] and checked["capture_config_valid"]
    assert not checked["requests_output_ownership"] and checked["configuration_bytes"] == 0
    origins = {item["field"]: item["state"] for item in checked["origins"]}
    assert origins.pop("Timeout multiplier code") == "manual"
    assert set(origins.values()) == {"eds"}
    with run_adapter(adapter, root) as log:
        identity = call(cli, root, "identity", "identity", ADAPTER, "127.0.0.1")["identity"]
        assert (identity["vendor"], identity["device_type"], identity["code"], identity["serial"]) == (
            1, 12, 65001, 123456789,
        )
        report = call(cli, root, "result", "capture", PROFILE, root / "capture", str(args.seconds),
                      "1:12:65001", timeout=args.seconds + 15)
    assert report["identity"] == identity
    assert "ForwardClose: ConnSerNo" in log.read_text(encoding="utf-8", errors="replace")
    check_capture(report, args.seconds)
    if args.explore:
        check_explore(cli, adapter, root / "extended", identity, args.seconds)
        check_connected_reads(cli, adapter, root / "connected-reads", identity)
    if args.multicast:
        check_multicast(cli, adapter, root / "multicast", identity, args.seconds)
    if args.listen_only:
        check_listen_only(cli, adapter, root / "listen-only", eds, identity, args.seconds)


if __name__ == "__main__":
    main()
