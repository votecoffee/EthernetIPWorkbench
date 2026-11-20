"""Independent, loopback-only EtherNet/IP Class 1 test fixture."""
import argparse
from contextlib import nullcontext
from ipaddress import IPv4Address
import json
import math
from pathlib import Path
import socket
import struct
import threading
import time

IP = "127.0.0.1"
SESSION = 0xABCD0123
OT_ID = 0x1234ABCD
VARIABLE_INPUT_MAX = {"variable-input": 12, "large-variable-input": 600}
VARIABLE_OUTPUT_MAX = {"variable-output": 12, "large-variable-output": 600}
MULTICAST_GROUP = "239.255.92.44"


def recv_exact(sock, length):
    data = bytearray()
    while len(data) < length:
        chunk = sock.recv(length - len(data))
        if not chunk:
            raise EOFError
        data.extend(chunk)
    return data


def rr_items(raw):
    assert raw[:4] == bytes(4)
    count = struct.unpack_from("<H", raw, 6)[0]
    items = {}
    offset = 8
    for _ in range(count):
        code, length = struct.unpack_from("<HH", raw, offset)
        offset += 4
        items[code] = raw[offset:offset + length]
        offset += length
    assert offset == len(raw)
    return items


def rr_reply(cip, *extra):
    items = [(0, b""), (0xB2, cip), *extra]
    return struct.pack("<IHH", 0, 0, len(items)) + b"".join(
        struct.pack("<HH", code, len(data)) + data for code, data in items
    )


def sockaddr(address, port):
    return struct.pack(">HH4s8x", 2, port, socket.inet_aton(address))


def io_packet(connection, seq, data):
    return struct.pack("<HHHIIHHH", 2, 0x8002, 8, connection, seq,
                       0xB1, 2 + len(data), seq & 0xFFFF) + data


def spacing(stamps):
    gaps = sorted((later - earlier) // 1000 for earlier, later in zip(stamps, stamps[1:]))
    if not gaps:
        return {}
    return {f"p{p}_us": gaps[math.ceil(p / 100 * len(gaps)) - 1] for p in (50, 95, 99)} | {
        "max_us": gaps[-1], "intervals": len(gaps),
    }


def udp_loop(udp, source, endpoint, connection, api, mode, value_base, stop, report, multicast=None):
    sent, received = [], []
    output_values = set()
    output_max = VARIABLE_OUTPUT_MAX.get(mode)
    owner = mode == "owner" or output_max is not None
    output_header = owner and mode != "large-variable-output"
    output_start = 24 if output_header else 20
    output_lengths = range(2, (output_max or 2) + 1) if owner else range(1)
    length_transitions = []
    variable_max = VARIABLE_INPUT_MAX.get(mode)
    prior_sequence = last_sent_sequence = None
    missed = invalid = skipped = 0
    seq = 0
    period = api * 1000 * (2 if mode == "slow-input" else 1)
    deadline = None
    udp.setblocking(False)
    while not stop.is_set():
        while True:
            try:
                packet, peer = udp.recvfrom(2048)
                stamp = time.perf_counter_ns()
            except BlockingIOError:
                break
            if multicast and not endpoint[1] and peer[0] == endpoint[0]:
                endpoint = peer
                report["cyclic_io"]["local_udp"] = f"{peer[0]}:{peer[1]}"
            output_length = len(packet) - output_start
            if peer != endpoint or output_length not in output_lengths:
                invalid += 1
                continue
            count, addr_type, addr_size, cid, sequence, dtype, size, cip = struct.unpack_from(
                "<HHHIIHHH", packet
            )
            if (count, addr_type, addr_size, cid, dtype, size, cip) != (
                2, 0x8002, 8, OT_ID, 0xB1, len(packet) - 18, sequence & 0xFFFF
            ):
                invalid += 1
                continue
            if output_header:
                assert packet[20:24] == b"\x01\x00\x00\x00"
            if owner:
                output_values.add(packet[output_start:].hex())
            if output_max is not None and (not length_transitions or output_length != length_transitions[-1]):
                length_transitions.append(output_length)
            if prior_sequence is not None:
                missed += (sequence - prior_sequence - 1) & 0xFFFFFFFF
            prior_sequence = sequence
            received.append(stamp)
            if deadline is None:
                deadline = stamp + (80_000_000 if mode == "delayed-start" else 0)
        now = time.perf_counter_ns()
        if deadline is not None and now >= deadline:
            slots = (now - deadline) // period + 1
            skipped += slots - 1
            deadline += slots * period
            seq = (seq + 1) & 0xFFFFFFFF
            drop = mode == "no-input" or (mode == "stall" and seq > 75) or (mode == "faults" and seq == 60)
            if not drop:
                value = struct.pack("<I", value_base + seq // 16)
                if mode in ("large-keyed", "routed-io-large"):
                    value += bytes(592) + struct.pack("<I", 0xA1B2C3D4)
                if variable_max is not None:
                    length = (4, 7, variable_max)[(seq // 8) % 3]
                    if seq == 10:
                        length = 3
                    elif seq == 20:
                        length = variable_max + 1
                    value = (value + b"\xA5" * length)[:length]
                if mode == "invalid-input":
                    value = value[:-1]
                packet = io_packet(connection, seq,
                                   (bytes((1, 0, 0, 0)) if owner or variable_max is not None else b"") + value)
                if multicast and seq == 5:
                    source.sendto(io_packet(connection ^ 1, seq, b"\xFF" * 4), multicast)
                source.sendto(packet, multicast or endpoint)
                sent.append(time.perf_counter_ns())
                if len(sent) == 1 and mode != "invalid-input":
                    source_ip, source_port = source.getsockname()
                    report["cyclic_io"]["first_input_source"] = f"{source_ip}:{source_port}"
                last_sent_sequence = seq
                if mode == "pause-different-api" and seq == 75:
                    deadline += 60_000_000
                    report["pause_after_sequence"] = seq
                if mode == "faults" and seq == 80:
                    source.sendto(packet, endpoint)
                    source.sendto(io_packet(connection, 60, value), endpoint)
                    source.sendto(io_packet(connection, seq, value[:-1]), endpoint)
        if deadline is None:
            time.sleep(0.00015)
        elif deadline - now > 250_000:
            time.sleep(min((deadline - now - 160_000) / 1e9, 0.0004))
    report.update({
        "ot_received": len(received), "ot_sequence_gaps": missed, "ot_invalid": invalid,
        "ot_received_after_last_to": sum(stamp >= sent[-1] for stamp in received) if sent else len(received),
        "ot_receive_spacing": spacing(received), "to_sent": len(sent),
        "to_last_sequence": last_sent_sequence,
        "to_last_gap_us": (sent[-1] - sent[-2]) // 1000 if len(sent) >= 2 else None,
        "to_send_spacing": spacing(sent), "to_skipped_deadline_slots": skipped, "output_values": sorted(output_values),
    })
    if output_max is not None:
        report["output_length_transitions"] = length_transitions


def routed_request(cip):
    assert cip[:6] == bytes.fromhex("52 02 20 06 24 01")
    priority_tick, ticks, length = struct.unpack_from("<BBH", cip, 6)
    assert 0 < (1 << (priority_tick & 0xF)) * ticks < 2000
    end = 10 + length
    message = cip[10:end]
    assert len(message) == length
    if length % 2:
        assert cip[end] == 0
        end += 1
    words, reserved = cip[end:end + 2]
    route = cip[end + 2:]
    assert reserved == 0 and len(route) == words * 2
    return route, message


def routed_reply(cip, attrs, report):
    route, message = routed_request(cip)
    report.update(route_hex=route.hex(), embedded_hex=message.hex())
    if route == b"\x01\x07":
        assert message == bytes.fromhex("0E 03 20 01 24 01 30 06")
        return bytes.fromhex("D2 00 01 01 11 03")
    if route == b"\x01\x05":
        assert message == bytes.fromhex("0E 03 20 01 24 01 30 06")
        return bytes.fromhex("D2 00 00 00")
    if route == b"\x01\x02\x12\x0B192.0.2.100\x00\x01\x00":
        assert message == bytes.fromhex("01 02 20 01 24 01")
        return bytes.fromhex("81 00 00 00") + b"".join(attrs)
    assert route == b"\x01\x03"
    if message == bytes.fromhex("0E 03 20 01 24 01 30 7F"):
        return bytes.fromhex("8E 00 14 01 02 03")
    assert message == bytes.fromhex("0E 03 20 01 24 01 30 06")
    return bytes.fromhex("8E 00 00 00") + attrs[6]


def multiple_service_reply(cip, attrs):
    assert cip[:6] == bytes.fromhex("0A 02 20 02 24 01")
    data = cip[6:]
    count, = struct.unpack_from("<H", data)
    offsets = struct.unpack_from(f"<{count}H", data, 2) + (len(data),)
    assert offsets[0] == 2 + count * 2
    replies = []
    rejected = False
    for start, end in zip(offsets, offsets[1:]):
        request = data[start:end]
        if request == bytes.fromhex("01 02 20 01 24 01"):
            replies.append(bytes.fromhex("81 00 00 00") + b"".join(attrs))
        else:
            assert len(request) == 8 and request[:7] == bytes.fromhex("0E 03 20 01 24 01 30")
            attribute = request[7]
            if attribute == 0x7F:
                replies.append(bytes.fromhex("8E 00 14 01 02 03"))
                rejected = True
            else:
                assert 1 <= attribute <= 7
                replies.append(bytes.fromhex("8E 00 00 00") + attrs[attribute])
    offsets = bytearray()
    offset = 2 + len(replies) * 2
    for reply in replies:
        offsets += struct.pack("<H", offset)
        offset += len(reply)
    return bytes((0x8A, 0, 0x1E if rejected else 0, 0)) + struct.pack("<H", count) + offsets + b"".join(replies)


def routed_identity_reply(cip, attrs, route, mode, report):
    received_route, message = routed_request(cip)
    assert received_route == route
    requested = report.setdefault("routed_identity_attributes", [])
    if message == bytes.fromhex("01 02 20 01 24 01"):
        requested.append("all")
        if mode == "routed-io-rejected-route":
            return bytes.fromhex("D2 00 01 01 11 03")
        if mode == "routed-io-large":
            return bytes.fromhex("81 00 08 00")
        return bytes.fromhex("81 00 00 00") + b"".join(attrs)
    assert mode == "routed-io-large"
    assert message[:7] == bytes.fromhex("0E 03 20 01 24 01 30") and len(message) == 8
    attribute = message[7]
    assert 1 <= attribute <= 7
    requested.append(attribute)
    return bytes.fromhex("8E 00 00 00") + attrs[attribute]


def serve(tcp, udp, source, mode, report, serial_delta=0, multicast_port=None):
    stop = threading.Event()
    io = None
    owner = mode == "owner" or mode in VARIABLE_OUTPUT_MAX
    multicast = mode.startswith("multicast")
    udp_mode = "stall" if mode == "stall-close-reject" else mode
    adapter_ip = tcp.getsockname()[0]
    scanner_ip = tcp.getpeername()[0]
    adapter_index = int(IPv4Address(adapter_ip)) - int(IPv4Address(IP))
    serial_number = 0x12345678 + adapter_index + serial_delta
    value_base = (adapter_index % 1000) * 100_000
    report.update(serial_number=serial_number, value_base=value_base)
    path = bytes.fromhex("20 04 24 01 2C 64 2C 65" if owner else "20 04 24 01 2C FE 2C 65")
    keyed = mode in ("large-keyed", "routed-io-large")
    if keyed:
        path = bytes.fromhex("34 04 01 00 0C 00 01 00 01 00") + path
    route = b""
    if mode.startswith("routed-io"):
        route = b"\x01\x02\x12\x09127.0.0.3\x00\x01\x00" if keyed else b"\x01\x03"
    path = route + path
    name = b"Loopback Routed Target" if route else b"Loopback Adapter"
    attrs = [b"", struct.pack("<H", 1 if keyed else 0),
             struct.pack("<H", 12), struct.pack("<H", 1), b"\x01\x00",
             struct.pack("<H", 0x0060), struct.pack("<I", serial_number),
             bytes([len(name)]) + name]
    try:
        while True:
            cmd, length, handle, status, ctx, options = struct.unpack(
                "<HHII8sI", recv_exact(tcp, 24)
            )
            request = recv_exact(tcp, length)
            assert status == options == 0
            if cmd == 0x65:
                assert handle == 0 and request == b"\x01\x00\x00\x00"
                if mode in ("saved-reads", "multiple-service"):
                    report["registrations"] = report.get("registrations", 0) + 1
                response = request
            elif cmd == 0x66:
                assert handle == SESSION
                if mode in ("saved-reads", "multiple-service"):
                    report["unregistered"] = True
                break
            elif cmd == 0x6F:
                assert handle == SESSION
                items = rr_items(request)
                cip = items[0xB2]
                service, words = cip[:2]
                if mode in ("saved-reads", "multiple-service"):
                    report.setdefault("explicit_requests", []).append(cip.hex())
                if mode == "multiple-service":
                    assert set(items) == {0, 0xB2} and not items[0]
                    response = rr_reply(multiple_service_reply(cip, attrs))
                    tcp.sendall(struct.pack("<HHII8sI", cmd, len(response), SESSION, 0, ctx, 0) + response)
                    continue
                if route and service == 0x52:
                    assert set(items) == {0, 0xB2} and not items[0]
                    result = routed_identity_reply(cip, attrs, route, mode, report)
                    response = rr_reply(result)
                    tcp.sendall(struct.pack("<HHII8sI", cmd, len(response), SESSION, 0, ctx, 0) + response)
                    continue
                if mode == "routed" or (mode == "saved-reads" and service == 0x52):
                    assert set(items) == {0, 0xB2} and not items[0]
                    response = rr_reply(routed_reply(cip, attrs, report))
                    tcp.sendall(struct.pack("<HHII8sI", cmd, len(response), SESSION, 0, ctx, 0) + response)
                    continue
                if mode in ("explicit", "saved-reads") and service == 0x0e:
                    assert len(cip) == 2 + words * 2
                    path_bytes = cip[2:]
                    if mode == "saved-reads" and path_bytes == bytes.fromhex("20 01 24 01 30 7E"):
                        break
                    if len(path_bytes) == 6 and path_bytes[:5] == b"\x20\x01\x24\x01\x30" and 1 <= path_bytes[5] <= 7:
                        attr = path_bytes[5]
                        result = bytes.fromhex("8E 00 00 00") + attrs[attr]
                    elif path_bytes == bytes.fromhex("21 00 34 12 26 00 78 56 34 12 30 07"):
                        result = bytes.fromhex("8E 00 00 00 CA FE")
                    else:
                        assert path_bytes == bytes.fromhex("20 01 24 01 30 7F")
                        result = bytes.fromhex("8E 00 14 01 02 03")
                    response = rr_reply(result)
                    tcp.sendall(struct.pack("<HHII8sI", cmd, len(response), SESSION, 0, ctx, 0) + response)
                    continue
                _, _, cls, object_class, inst, instance = cip[:6]
                assert (words, cls, inst, instance) == (2, 0x20, 0x24, 1)
                body = cip[6:]
                if service == 1 and object_class == 1:
                    assert not route, "Read the routed target identity before opening I/O"
                    response = rr_reply(bytes.fromhex("81 00 08 00") if mode == "explicit"
                                        else b"\x81\0\0\0" + b"".join(attrs))
                elif service in (0x54, 0x5B) and object_class == 6:
                    assert io is None
                    if route:
                        report["open_path_hex"] = path.hex()
                        assert report["routed_identity_attributes"] == (["all", *range(1, 8)] if keyed else ["all"])
                    _, to, serial, vendor, origin = struct.unpack_from("<IIHHI", body, 2)
                    if keyed or mode in ("large-variable-input", "large-variable-output"):
                        assert service == 0x5B
                        ot_rpi, ot_params, to_rpi, to_params, transport, path_words = struct.unpack_from(
                            "<IIIIBB", body, 22
                        )
                        expected_input = {
                            "large-variable-input": 0x4600025E,
                            "large-variable-output": 0x4400000A,
                            "large-keyed": 0x4400025A,
                            "routed-io-large": 0x4400025A,
                        }[mode]
                        expected_output = 0x4600025A if mode == "large-variable-output" else 0x44000002
                        assert ot_params == expected_output and to_params == expected_input
                        path_offset = 40
                    else:
                        assert service == 0x54
                        ot_rpi, ot_params, to_rpi, to_params, transport, path_words = struct.unpack_from(
                            "<IHIHBB", body, 22
                        )
                        expected_output = 0x4212 if mode == "variable-output" else 0x4008 if owner else 0x4002
                        assert ot_params & 0xE3FF == expected_output
                        expected_input = (0x2006 if multicast else 0x4212 if mode == "variable-input"
                                          else 0x400A if owner else 0x4006)
                        assert to_params & 0xE3FF == expected_input
                        path_offset = 36
                    assert transport == 1
                    assert len(body[path_offset:]) == path_words * 2
                    assert body[path_offset:] == path + (bytes.fromhex("80 02 AA BB CC 00") if owner else b"")
                    family, port, address = struct.unpack(">HH4s8x", items[0x8001])
                    assert family == 2 and port != 0
                    assert socket.inet_ntoa(address) == ("0.0.0.0" if multicast else scanner_ip)
                    assert body[18] <= 7 and body[19:22] == bytes(3)
                    report.update(ot_requested_us=ot_rpi, to_requested_us=to_rpi,
                                  timeout_multiplier_code=body[18])
                    if mode == "reject":
                        response = rr_reply(bytes.fromhex("D4 00 01 01 09 01"))
                    else:
                        ot_api, to_api = (1250, 1500) if mode in (
                            "different-api", "pause-different-api",
                        ) else (ot_rpi, to_rpi)
                        if mode == "slower-api":
                            ot_api, to_api = 10000, 12000
                        reply = struct.pack("<IIHHIIIBB", OT_ID, to, serial, vendor, origin,
                                            ot_api, to_api, 0, 0)
                        adapter_port = udp.getsockname()[1]
                        output_ip = "127.0.0.3" if mode == "routed-io-unexpected-peer" else adapter_ip
                        extras = [(0x8000, sockaddr(output_ip, adapter_port))]
                        group = None
                        if multicast:
                            # Supply a different T->O ID to model an existing producer.
                            to = 0xBCDE8765
                            reply = struct.pack("<IIHHIIIBB", OT_ID, to, serial, vendor, origin,
                                                ot_api, to_api, 0, 0)
                            report["requested_input_port"] = port
                            if mode != "multicast-missing":
                                group = (MULTICAST_GROUP if mode == "multicast" else adapter_ip, multicast_port)
                                extras.append((0x8001, sockaddr(*group)))
                        response = rr_reply(bytes([service | 0x80, 0, 0, 0]) + reply, *extras)
                        report["cyclic_io"] = {
                            "local_udp": f"{scanner_ip}:{port}",
                            "output_destination": f"{adapter_ip}:{adapter_port}",
                            "ot_connection_id": OT_ID,
                            "to_connection_id": to,
                        }
                        if mode == "delayed-start":
                            time.sleep(0.05)
                        if (not multicast or mode == "multicast") and mode != "routed-io-unexpected-peer":
                            if multicast:
                                # The fixture never emits multicast beyond localhost.
                                source.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_IF,
                                                  socket.inet_aton(adapter_ip))
                                source.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_TTL, 0)
                                source.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_LOOP, 1)
                                report["cyclic_io"]["input_multicast_destination"] = f"{group[0]}:{group[1]}"
                                report["cyclic_io"]["input_local_udp"] = f"{scanner_ip}:{group[1]}"
                            io = threading.Thread(target=udp_loop,
                                                  args=(udp, source, (scanner_ip, 0 if multicast else port),
                                                        to, to_api, udp_mode, value_base, stop, report, group))
                            io.start()
                elif service == 0x4E and object_class == 6:
                    assert struct.unpack_from("<HHI", body, 2) == (serial, vendor, origin)
                    assert body[12:] == path
                    report["forward_close"] = True
                    if route:
                        report["close_path_hex"] = body[12:].hex()
                    stop.set()
                    if mode == "close-eof":
                        break
                    if mode in ("close-reject", "stall-close-reject"):
                        response = rr_reply(bytes.fromhex("CE 00 01 01 07 01") + body[2:10] + b"\0\0")
                    elif mode == "close-mismatch":
                        wrong_ids = struct.pack("<H", serial ^ 1) + body[4:10]
                        response = rr_reply(b"\xCE\0\0\0" + wrong_ids + b"\0\0")
                    elif mode == "close-short":
                        response = rr_reply(b"\xCE\0\0\0")
                    else:
                        response = rr_reply(b"\xCE\0\0\0" + body[2:10] + b"\0\0")
                else:
                    raise AssertionError(f"Unexpected CIP request {service:#x}/{object_class:#x}")
            else:
                raise AssertionError(f"Unexpected command {cmd:#x}")
            tcp.sendall(struct.pack("<HHII8sI", cmd, len(response), SESSION, 0, ctx, 0) + response)
    finally:
        stop.set()
        if io is not None:
            io.join()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path)
    parser.add_argument("--mode", default="normal", choices=(
        "normal", "faults", "stall", "no-input", "invalid-input", "reject", "different-api", "owner",
        "reconnect", "recover", "recover-slower-api", "replacement", "qualify", "qualify-different-api",
        "qualify-replacement", "close-reject", "close-mismatch", "close-short", "close-eof",
        "stall-close-reject", "qualify-close-reject",
        "slow-input", "pause-different-api",
        "identity-then-owner",
        "explicit", "saved-reads", "multiple-service", "large-keyed", "routed", "variable-input", "large-variable-input",
        "routed-io", "routed-io-large", "routed-io-rejected-route", "routed-io-unexpected-peer",
        "variable-output", "large-variable-output",
        "multicast", "multicast-missing", "multicast-unicast",
        "delayed-start", "split-udp",
    ))
    parser.add_argument("--address", type=IPv4Address, default=IPv4Address(IP))
    parser.add_argument("--multicast-port", type=int, default=2222)
    parser.add_argument("--tcp-timeout", type=int, help="Override the fixture's TCP inactivity timeout in seconds")
    args = parser.parse_args()
    if not args.address.is_loopback:
        parser.error("--address must be an IPv4 loopback address")
    if args.tcp_timeout is not None and args.tcp_timeout <= 0:
        parser.error("--tcp-timeout must be positive")
    address = str(args.address)
    report = {"mode": args.mode, "forward_close": False}
    with socket.socket() as listener:
        listener.bind((address, 44818))
        listener.listen(1)
        listener.settimeout(30)
        print("Ready on loopback", flush=True)
        sequences = {
            "reconnect": ("normal", "normal"),
            "recover": ("stall", "normal"),
            "recover-slower-api": ("stall", "slower-api"),
            "replacement": ("stall", "normal"),
            "qualify": ("normal", "normal", "normal", "normal"),
            "qualify-replacement": ("normal", "normal"),
            "qualify-different-api": ("normal", "different-api", "normal", "normal"),
            "identity-then-owner": ("normal", "owner"),
            "qualify-close-reject": ("normal", "close-reject"),
        }
        modes = sequences.get(args.mode, (args.mode,))
        record_attempts = len(modes) > 1 and args.mode != "reconnect"
        if record_attempts:
            report["attempts"] = []
        for index, mode in enumerate(modes):
            conn, _ = listener.accept()
            with conn, socket.socket(type=socket.SOCK_DGRAM) as udp:
                conn.settimeout(args.tcp_timeout or (75 if args.mode.startswith("qualify") else 20))
                udp.bind((address, 0))
                attempt = {"forward_close": False} if record_attempts else report
                replacement = args.mode in ("replacement", "qualify-replacement") and index == 1
                input_socket = socket.socket(type=socket.SOCK_DGRAM) if mode == "split-udp" else nullcontext(udp)
                with input_socket as source:
                    if source is not udp:
                        source.bind((address, 0))
                    serve(conn, udp, source, mode, attempt, serial_delta=int(replacement),
                          multicast_port=args.multicast_port)
                if record_attempts:
                    report["attempts"].append(attempt)
    args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
