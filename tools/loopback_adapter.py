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
ROUTED_OWNER_CYCLIC_MODES = {
    "routed-io-owner": "owner",
    "routed-io-large-owner": "large-variable-output",
    "routed-io-owner-unexpected-peer": "owner",
}
ROUTED_KEYED_MODES = ("routed-io-large", "routed-io-large-owner", "routed-io-multicast-large",
                      "routed-io-large-default-unicast", "routed-io-large-extended-port")
LARGE_INPUT_MODES = ("large-keyed", "routed-io-large", "routed-io-multicast-large",
                     "routed-io-large-default-unicast", "routed-io-large-extended-port")
EXTENDED_ROUTE = (b"\x01\x02\x1f\x0b\x2c\x01192.0.2.100\x00"
                  b"\x1f\x03\x34\x12\x00\x80\xfe\x00\x0f\x0f\x00\x07")
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
                if mode in LARGE_INPUT_MODES:
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
                    source.sendto(io_packet(connection ^ 1, seq, b"\xFF" * len(value)), multicast)
                    if mode.startswith("routed-io-"):
                        # A valid packet from a different sender must be ignored.
                        with socket.socket(type=socket.SOCK_DGRAM) as other:
                            other.bind((endpoint[0], 0))
                            other.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_IF,
                                             socket.inet_aton(endpoint[0]))
                            other.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_TTL, 0)
                            other.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_LOOP, 1)
                            other.sendto(io_packet(connection, seq, b"\xFF" * len(value)), multicast)
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
    if route == EXTENDED_ROUTE:
        report["request_cip_hex"] = cip.hex()
    else:
        assert route == b"\x01\x03"
    if message == bytes.fromhex("0E 03 20 01 24 01 30 7F"):
        return bytes.fromhex("8E 00 14 01 02 03")
    assert message == bytes.fromhex("0E 03 20 01 24 01 30 06")
    return bytes.fromhex("8E 00 00 00") + attrs[6]


def identity_attribute_reply(request, attrs):
    assert len(request) == 8 and request[:7] == bytes.fromhex("0E 03 20 01 24 01 30")
    attribute = request[7]
    if attribute == 0x7F:
        return bytes.fromhex("8E 00 14 01 02 03")
    assert 1 <= attribute <= 7
    return bytes.fromhex("8E 00 00 00") + attrs[attribute]


def attribute_list_reply(cip, mode, report):
    route, message = routed_request(cip) if cip[0] == 0x52 else (b"", cip)
    report.setdefault("attribute_list_requests", []).append(message.hex())
    report["attribute_list_route"] = route.hex()
    assert message == bytes.fromhex("03 02 20 01 24 01 03 00 01 00 00 01 06 00")
    assert route in (b"", b"\x01\x03", b"\x01\x07")
    if route == b"\x01\x07":
        return bytes.fromhex("D2 00 01 01 11 03")
    if mode == "attribute-list-unsupported":
        return bytes.fromhex("83 00 08 00")
    missing = mode == "attribute-list-mixed"
    rows = struct.pack("<HHBBH", 3, 1, 0, 0, 123)
    rows += struct.pack("<HBB", 0x100, 0x14 if missing else 0, 0)
    if not missing:
        rows += b"\xC5"
    rows += struct.pack("<HBBI", 6, 0, 0, 0x12345678)
    if mode == "attribute-list-malformed":
        rows += b"\xFF"
    return bytes((0x83, 0, 0x0A if missing else 0, 0)) + rows


def class_info_reply(cip, mode, report):
    route, message = routed_request(cip) if cip[0] == 0x52 else (b"", cip)
    assert len(message) == 8 and message[:7] == bytes.fromhex("0E 03 20 01 24 00 30")
    attribute = message[7]
    assert 1 <= attribute <= 7 and route in (b"", b"\x01\x03", b"\x01\x07")
    report.setdefault("class_attributes", []).append(attribute)
    report["route_hex"] = route.hex()
    if route == b"\x01\x07":
        return bytes.fromhex("D2 00 01 01 11 03")
    if route and attribute == 5:
        return bytes.fromhex("8E 00 14 01 02 03")
    if mode == "class-info-malformed" and attribute == 4:
        return bytes.fromhex("8E 00 00 00 02 00 03 00")
    if mode == "class-info-empty" and attribute in (4, 5):
        return bytes.fromhex("8E 00 00 00 00 00")
    values = ("02 00", "01 00", "01 00", "02 00 03 00 00 01",
              "02 00 01 00 0E 00", "07 00", "00 01")
    return bytes.fromhex("8E 00 00 00 " + values[attribute - 1])


def port_info_reply(cip, mode, report):
    route, message = routed_request(cip) if cip[0] == 0x52 else (b"", cip)
    assert len(message) == 8 and message[:5] == bytes.fromhex("0E 03 20 F4 24")
    instance, attr_tag, attribute = message[5:]
    assert instance in (0, 1) and attr_tag == 0x30
    assert route in (b"", b"\x01\x03", b"\x01\x07")
    report.setdefault("port_reads", []).append([instance, attribute])
    report["route_hex"] = route.hex()
    if route == b"\x01\x07":
        return bytes.fromhex("D2 00 01 01 11 03")
    if mode == "port-info-optional" and attribute == 10:
        return bytes.fromhex("8E 00 14 01 02 03")
    directory = {2: "02 00", 3: "02 00", 8: "01 00",
                 9: "00 00 00 00 04 00 2C 01 01 00 01 00"}
    details = {1: "04 00", 2: "2C 01", 3: "02 00 20 F5 24 01",
               4: "06 55 70 6C 69 6E 6B", 7: "1F 03 2C 01 00 80 FE 00",
               10: "17 00 00 80"}
    if mode == "port-info-malformed":
        details[7] = "1F 03 2C 01 00 80 FE"
    values = directory if instance == 0 else details
    return bytes.fromhex("8E 00 00 00 " + values[attribute])


def tcpip_info_reply(cip, mode, report):
    route, message = routed_request(cip) if cip[0] == 0x52 else (b"", cip)
    assert len(message) == 8 and message[:7] == bytes.fromhex("0E 03 20 F5 24 01 30")
    attribute = message[7]
    diagnostics = mode.startswith("tcpip-info-diagnostics")
    allowed = (1, 2, 3, 4, 5, 6, 8, 9, 13) if diagnostics else range(1, 7)
    assert attribute in allowed and route in (b"", b"\x01\x03")
    report.setdefault("tcpip_attributes", []).append(attribute)
    report["route_hex"] = route.hex()
    if mode == "tcpip-info-optional" and attribute == 6:
        return bytes.fromhex("8E 00 14 01 02 03")
    if mode == "tcpip-info-diagnostics-optional" and attribute == 13:
        return bytes.fromhex("8E 00 14 01 02 03")
    addresses = "0A 02 00 C0 00 FF FF FF 01 02 00 C0 35 64 33 C6 07 71 00 CB"
    domain = "08 00 6C 61 62" if mode == "tcpip-info-malformed" else "03 00 6C 61 62 00"
    values = {
        1: "11 00 00 00" if diagnostics else "01 00 00 00",
        2: "15 00 00 00",
        3: "12 00 00 00",
        4: "02 00 20 F6 24 01",
        5: addresses + " " + domain,
        6: "05 00 62 65 6E 63 68",
        8: "03",
        9: "01 00 20 00 12 09 C1 EF",
        13: "0A 00",
    }
    if mode == "tcpip-info-diagnostics-malformed":
        values.update({8: "03 00", 9: "01 00 20 00 12 09 C1", 13: "00 00"})
    return bytes.fromhex("8E 00 00 00 " + values[attribute])


def acd_info_reply(cip, mode, report):
    route, message = routed_request(cip) if cip[0] == 0x52 else (b"", cip)
    assert len(message) == 8 and message[:7] == bytes.fromhex("0E 03 20 F5 24 01 30")
    attribute = message[7]
    assert attribute in (1, 2, 10, 11) and route in (b"", b"\x01\x03", b"\x01\x07")
    report.setdefault("acd_attributes", []).append(attribute)
    report["route_hex"] = route.hex()
    if route == b"\x01\x07":
        return bytes.fromhex("D2 00 01 01 11 03")
    if mode == "acd-info-unsupported" and attribute in (10, 11):
        return bytes.fromhex("8E 00 14 00")

    clear = mode in ("acd-info-clear", "acd-info-unsupported")
    activity = 0 if clear else 1 if mode == "acd-info-probe" else 2
    remote = bytes.fromhex("02 12 34 56 78 9A")
    arp = struct.pack(">HHBBH", 1, 0x800, 6, 4, 1 if activity == 1 else 2)
    sender = "0.0.0.0" if activity == 1 else "192.0.2.23"
    arp += remote + socket.inet_aton(sender) + bytes(6) + socket.inet_aton("198.51.100.37")
    if mode == "acd-info-unknown-arp":
        arp = b"\x00\x02" + arp[2:]
    values = {
        1: struct.pack("<I", 0x01 if clear else 0xC1),
        2: struct.pack("<I", 0x15 if mode == "acd-info-unsupported" else 0x95),
        10: bytes((0 if clear else 1,)),
        11: bytes((activity,)) + remote + arp,
    }
    if mode == "acd-info-malformed":
        values.update({10: b"\x02", 11: values[11][:-1]})
    return bytes.fromhex("8E 00 00 00") + values[attribute]


def link_info_reply(cip, mode, report):
    route, message = routed_request(cip) if cip[0] == 0x52 else (b"", cip)
    assert len(message) == 8 and message[:5] == bytes.fromhex("0E 03 20 F6 24")
    instance, tag, attribute = message[5:]
    assert (instance, route) in ((1, b""), (1, b"\x01\x03"), (2, b"\x01\x03"))
    details = mode.startswith("link-info-details")
    assert tag == 0x30 and 1 <= attribute <= (11 if details else 5)
    report.setdefault("link_reads", []).append([instance, attribute])
    report["route_hex"] = route.hex()
    if (mode == "link-info-optional" and attribute == 5
            or mode == "link-info-details-optional" and attribute == 9):
        return bytes.fromhex("8E 00 14 01 02 03")
    flags = 0x80000077 if instance == 1 else 0x02
    interface = struct.pack("<11I", 0xFFFFFFFF, *range(1, 10), 0x80000002)
    if mode == "link-info-malformed":
        interface = interface[:-1]
    values = (
        struct.pack("<I", 1000 if instance == 1 else 0), struct.pack("<I", flags),
        bytes.fromhex("02 AB CD 10 20 30"), interface, struct.pack("<12I", *range(20, 32)),
    )
    if details:
        caps = struct.pack("<IB", 0x8000000F, 3)
        caps += b"".join(struct.pack("<HB", speed, duplex) for speed, duplex in ((10, 0), (100, 1), (1000, 1)))
        port_label = b"\x06Port 1" if instance == 1 else b"\x05\x00Fiber\x00"
        if mode == "link-info-details-malformed":
            port_label, caps = b"\x06Port", caps[:-1]
        values += (
            struct.pack("<HH", 0x0003 if instance == 1 else 0x0002, 1000),
            bytes((2 if instance == 1 else 3,)),
            bytes((1 if instance == 1 else 2,)),
            b"\x01", port_label, caps,
        )
    return bytes.fromhex("8E 00 00 00") + values[attribute - 1]


def lldp_neighbor_reply(cip, mode, report):
    route, message = routed_request(cip) if cip[0] == 0x52 else (b"", cip)
    assert route in (b"", b"\x01\x03")
    report["route_hex"] = route.hex()
    if message[0] == 0x11:
        assert message == bytes.fromhex("11 03 21 00 0A 01 24 00 40")
        report["lldp_listed"] = True
        return bytes.fromhex("91 00 00 00 03 07 00 2C 01 00 00")
    assert len(message) == 12 and message[:-1] == bytes.fromhex("0E 05 21 00 0A 01 25 00 2C 01 30")
    attribute = message[-1]
    assert 1 <= attribute <= 9
    report.setdefault("lldp_reads", []).append(attribute)
    if mode == "lldp-neighbor-optional" and attribute == 6:
        return bytes.fromhex("8E 00 14 01 02 03")
    values = [
        struct.pack("<H", 2), bytes.fromhex("02 14 6B 34 12 04"), b"\x11Lab-Switch Port 4",
        struct.pack("<H", 120), struct.pack("<HH", 0x0094, 0x0004),
        bytes((2, 192, 0, 2, 41, 198, 51, 100, 5)),
        struct.pack("<HHHBBI", 0x1234, 12, 0x0190, 2, 7, 0x89ABCDEF),
        bytes((1, 1, 0, 2)), struct.pack("<I", 5001),
    ]
    if mode == "lldp-neighbor-empty":
        values[:3] = [bytes(2), bytes(6), bytes(1)]
        values[5:8] = [bytes(1), bytes(12), bytes(4)]
    if mode == "lldp-neighbor-malformed":
        values[2] = b"\x05Lab"
        values[5] = values[5][:-4]
        values[6] = values[6][:-1]
    return bytes.fromhex("8E 00 00 00") + values[attribute - 1]


def connection_data_reply(cip, mode, report):
    route, message = routed_request(cip) if cip[0] == 0x52 else (b"", cip)
    assert route in (b"", b"\x01\x03", b"\x01\x07")
    assert message in (
        bytes.fromhex("56 02 20 06 24 01 34 12"),
        bytes.fromhex("57 02 20 06 24 01 CD AB 01 00 78 56 34 12"),
    )
    report.setdefault("connection_data_requests", []).append(message.hex())
    report["route_hex"] = route.hex()
    if route == b"\x01\x07":
        return bytes.fromhex("D2 00 01 01 11 03")
    reply = bytes((message[0] | 0x80, 0))
    if mode == "connection-data-unsupported":
        return reply + bytes.fromhex("08 00")
    if mode == "connection-data-not-found":
        return reply + bytes.fromhex("05 00")
    serial = 0xABCC if mode == "connection-data-mismatch" else 0xABCD
    data = struct.pack("<6HI", 0x1234, 3, 37, 19, serial, 1, 0x12345678)
    data += struct.pack("<IIB3xII", 0x343A5678, 0xAB90EFEF, 3, 8000, 8100)
    data += struct.pack("<IIB3xII", 0xEE101010, 0x90919293, 4, 1000, 2000)
    if mode == "connection-data-malformed":
        data = data[:-1]
    return reply + b"\0\0" + data


def manager_info_reply(cip, mode, report):
    route, message = routed_request(cip) if cip[0] == 0x52 else (b"", cip)
    assert len(message) == 8 and message[:7] == bytes.fromhex("0E 03 20 06 24 01 30")
    attribute = message[7]
    assert 1 <= attribute <= 8 and route in (b"", b"\x01\x03", b"\x01\x07")
    report.setdefault("manager_attributes", []).append(attribute)
    report["route_hex"] = route.hex()
    if route == b"\x01\x07":
        return bytes.fromhex("D2 00 01 01 11 03")
    if mode == "manager-info-optional" and attribute == 3:
        return bytes.fromhex("8E 00 14 01 02 03")
    if mode == "manager-info-malformed" and attribute == 5:
        return bytes.fromhex("8E 00 00 00 2A 00 01 00")
    values = (0xFFFF, 0, 0x0201, 0x8005, 42, 2, 3, 0xA120)
    return bytes.fromhex("8E 00 00 00") + struct.pack("<H", values[attribute - 1])


def dlr_info_reply(cip, mode, report):
    route, message = routed_request(cip) if cip[0] == 0x52 else (b"", cip)
    assert len(message) == 8 and message[:5] == bytes.fromhex("0E 03 20 47 24")
    instance, tag, attribute = message[5:]
    assert tag == 0x30 and (instance, route) in ((1, b""), (1, b"\x01\x03"), (2, b"\x01\x03"))
    report.setdefault("dlr_attributes", []).append(attribute)
    report["dlr_instance"] = instance
    report["route_hex"] = route.hex()
    if mode == "dlr-info-optional" and attribute == 7:
        return bytes.fromhex("8E 00 14 01 02 03")
    values = {
        1: b"\x01", 2: b"\x01", 3: b"\x01", 5: bytes.fromhex("34 12"),
        6: bytes.fromhex("0A 02 00 C0 00 00 00 00 00 00"),
        7: bytes.fromhex("00 00 00 00 02 00 00 00 00 07"),
        8: bytes.fromhex("00 00"),
        10: bytes.fromhex("05 02 00 C0 02 11 22 33 44 55"),
        11: b"\xF0", 12: bytes.fromhex("A2 00 00 80"),
    }
    if mode == "dlr-info-malformed":
        values[2] = b"\x04\x00"
        values[10] = values[10][:-1]
    if mode == "dlr-info-empty":
        values.update({1: b"\x00", 2: b"\x05", 10: bytes(10)})
    return bytes.fromhex("8E 00 00 00") + values[attribute]


def dlr_participants_reply(cip, mode, report):
    route, message = routed_request(cip) if cip[0] == 0x52 else (b"", cip)
    assert len(message) == 8 and message[:5] == bytes.fromhex("0E 03 20 47 24")
    instance, tag, attribute = message[5:]
    assert tag == 0x30 and (instance, route) in ((1, b""), (1, b"\x01\x03"), (2, b"\x01\x03"))
    attributes = report.setdefault("dlr_participant_attributes", [])
    assert attribute == (8, 9, 8)[len(attributes)]
    attributes.append(attribute)
    report["dlr_instance"] = instance
    report["route_hex"] = route.hex()
    nodes = bytes.fromhex("46 64 33 C6 02 11 22 33 44 55 00 00 00 00 02 11 22 33 44 03 04 02 00 C0 00 00 00 00 00 00")
    if mode == "dlr-participants-large":
        nodes = b"".join(struct.pack("<I", 0xC0000200 + n) + bytes((2, 0, 0, 0, 0x20, n))
                         for n in range(1, 56))
    if mode == "dlr-participants-empty":
        nodes = b""
    if mode == "dlr-participants-overflow":
        nodes = nodes[:-10] + b"\xff" * 10
    before = after = len(nodes) // 10
    if mode == "dlr-participants-mismatch":
        before = after = 4
    if mode == "dlr-participants-changed":
        before, after = 2, 4
    if mode == "dlr-participants-malformed":
        nodes += b"\xAA"
    if mode == "dlr-participants-rejected" and attribute == 9:
        return bytes.fromhex("8E 00 11 01 02 03")
    if mode == "dlr-participants-no-count" and attribute == 8:
        return bytes.fromhex("8E 00 14 00")
    body = nodes if attribute == 9 else struct.pack("<H", before if len(attributes) == 1 else after)
    return bytes.fromhex("8E 00 00 00") + body


def dlr_paged_reply(cip, mode, report):
    route, message = routed_request(cip) if cip[0] == 0x52 else (b"", cip)
    assert route in (b"", b"\x01\x03")
    report["route_hex"] = route.hex()
    total = {"dlr-paged-empty": 0, "dlr-paged-limit": 1050}.get(mode, 70)
    if message[0] == 0x0E:
        assert message == bytes.fromhex("0E 03 20 47 24 01 30 08")
        count_reads = report.get("dlr_count_reads", 0) + 1
        assert count_reads <= 2
        report["dlr_count_reads"] = count_reads
        if mode == "dlr-paged-short" and count_reads == 2:
            total += 1
        return bytes.fromhex("8E 00 00 00") + struct.pack("<H", total)

    assert message[:10] == bytes.fromhex("18 05 20 47 24 01 30 09 29 00")
    assert len(message) == 15 and message[12] == 1
    flags = struct.unpack_from("<H", message, 10)[0]
    first = flags & 0x7FFF
    requested = struct.unpack_from("<H", message, 13)[0]
    assert flags & 0x8000 and 1 <= first <= total and requested == min(32, total - first + 1)
    pages = report.setdefault("dlr_page_requests", [])
    pages.append([first, requested])
    assert report.get("dlr_count_reads") == 1
    returned = 13 if first == 1 and mode == "dlr-paged-short" else requested
    reply_first = first
    if len(pages) == 2:
        if mode == "dlr-paged-malformed":
            reply_first += 1
        if mode == "dlr-paged-rejected":
            return bytes.fromhex("98 00 0C 01 04 03")
        if mode == "dlr-paged-exchange":
            return bytes.fromhex("8E 00 00 00")
        if mode == "dlr-paged-stalled":
            return bytes.fromhex("98 00 00 00 00 00")
        if mode == "dlr-paged-overflow":
            returned = 2
    nodes = b"".join(struct.pack("<I", 0xC0000200 + node % 254 + 1)
                     + bytes((2, 0, 0, 0x20, node >> 8, node & 0xFF))
                     for node in range(first, first + returned))
    if mode == "dlr-paged-overflow" and len(pages) == 2:
        nodes = nodes[:-10] + b"\xFF" * 10
    return bytes.fromhex("98 00 00 00") + struct.pack("<HH", returned, reply_first) + nodes


def time_sync_reply(cip, mode, report):
    route, message = routed_request(cip) if cip[0] == 0x52 else (b"", cip)
    assert route in (b"", b"\x01\x03")
    assert len(message) == 8 and message[:7] == bytes.fromhex("0E 03 20 43 24 01 30")
    attribute = message[-1]
    report.setdefault("time_sync_attributes", []).append(attribute)
    report["route_hex"] = route.hex()
    if mode == "time-sync-optional" and attribute == 6:
        return bytes.fromhex("8E 00 14 01 02 03")
    values = {
        1: b"\x01", 2: b"\x01", 5: struct.pack("<q", -123456789012),
        6: struct.pack("<Q", 0x8100000000000280), 7: struct.pack("<q", 43210),
        8: bytes.fromhex("02 11 22 FF FE 33 44 55") + struct.pack("<8H", 6, 0x22, 0x1234, 37, 0x800C, 0x20, 64, 128),
        12: struct.pack("<7H", 3, 1, 9, 2, 6, 257, 0x0400),
        18: b"\x18", 27: struct.pack("<H", 258),
    }
    if mode == "time-sync-empty":
        values.update({1: bytes(1), 2: bytes(1), 5: bytes(8), 8: bytes(24), 12: bytes(2),
                       18: bytes(1), 27: bytes(2)})
    if mode == "time-sync-malformed":
        values.update({2: b"\x02", 5: values[5][:-1], 8: values[8][:-1], 12: values[12][:-4]})
    return bytes.fromhex("8E 00 00 00") + values[attribute]


def qos_info_reply(cip, mode, report):
    route, message = routed_request(cip) if cip[0] == 0x52 else (b"", cip)
    assert len(message) == 8 and message[:7] == bytes.fromhex("0E 03 20 48 24 01 30")
    attribute = message[7]
    assert 1 <= attribute <= 8 and route in (b"", b"\x01\x03")
    report.setdefault("qos_attributes", []).append(attribute)
    report["route_hex"] = route.hex()
    if mode == "qos-info-optional" and attribute <= 3:
        return bytes.fromhex("8E 00 14 01 02 03")
    if mode == "qos-info-malformed" and attribute in (1, 4, 5):
        body = {1: "02", 4: "40", 5: "2E 00"}[attribute]
        return bytes.fromhex("8E 00 00 00 " + body)
    settings = (1, 59, 47, 63, 46, 43, 0, 27)
    return bytes.fromhex("8E 00 00 00") + bytes([settings[attribute - 1]])


def assembly_info_reply(cip, mode, report):
    route, message = routed_request(cip) if cip[0] == 0x52 else (b"", cip)
    assert len(message) == 8 and message[:7] == bytes.fromhex("0E 03 20 04 24 65 30")
    attribute = message[7]
    assert attribute in (3, 4) and route in (b"", b"\x01\x03")
    report.setdefault("assembly_attributes", []).append(attribute)
    report["route_hex"] = route.hex()
    if (mode, attribute) in (("assembly-info-size-unsupported", 4), ("assembly-info-data-unsupported", 3)):
        return bytes.fromhex("8E 00 14 01 02 03")
    if mode == "assembly-info-empty":
        body = b"" if attribute == 3 else b"\0\0"
    elif attribute == 3:
        body = bytes.fromhex("A5 16 2C")
    elif mode == "assembly-info-malformed":
        body = b"\x03"
    else:
        body = struct.pack("<H", 32 if route else 3)
    return bytes.fromhex("8E 00 00 00") + body


def profile_assemblies_reply(cip, mode, report):
    route, message = routed_request(cip) if cip[0] == 0x52 else (b"", cip)
    assert len(message) == 8 and message[:5] == bytes.fromhex("0E 03 20 04 24")
    instance = message[5]
    assert message[6:] == b"\x30\x04" and route in (b"", b"\x01\x03")
    report.setdefault("assembly_size_instances", []).append(instance)
    report["route_hex"] = route.hex()
    if route and instance == 1:
        return bytes.fromhex("8E 00 14 01 02 03")
    if mode == "profile-assemblies-malformed" and instance == 1:
        return bytes.fromhex("8E 00 00 00 00")
    sizes = {101: 5, 254: 4} if route else {101: 4, 254: 0, 1: 0}
    return bytes.fromhex("8E 00 00 00") + struct.pack("<H", sizes[instance])


def object_list_reply(cip, mode, report):
    route, message = routed_request(cip) if cip[0] == 0x52 else (b"", cip)
    assert message == bytes.fromhex("0E 03 20 02 24 01 30 01")
    assert route in (b"", b"\x01\x03", b"\x01\x07")
    report.setdefault("object_list_requests", []).append(message.hex())
    report["route_hex"] = route.hex()
    if route == b"\x01\x07":
        return bytes.fromhex("D2 00 01 01 11 03")
    if mode == "object-list-unsupported":
        return bytes.fromhex("8E 00 14 01 02 03")
    values = {
        "object-list": "04 00 34 12 04 00 F6 00 06 00",
        "object-list-empty": "00 00",
        "object-list-malformed": "04 00 34 12 04 00",
        "object-list-zero": "02 00 34 12 00 00",
    }
    return bytes.fromhex("8E 00 00 00 " + values[mode])


def instance_list_reply(cip, mode, report):
    route, message = routed_request(cip) if cip[0] == 0x52 else (b"", cip)
    request = {
        bytes.fromhex("11 03 21 00 34 12 24 00 40"): 0,
        bytes.fromhex("11 04 21 00 34 12 25 00 2C 01 40"): 300,
        bytes.fromhex("11 04 21 00 34 12 25 00 58 02 40"): 600,
    }
    after = request[bytes(message)]
    assert route in (b"", b"\x01\x03", b"\x01\x07")
    report.setdefault("instance_list_cursors", []).append(after)
    report["route_hex"] = route.hex()
    if route == b"\x01\x07":
        return bytes.fromhex("D2 00 01 01 11 03")
    if mode == "instance-list-unsupported":
        return bytes.fromhex("91 00 08 00")
    if mode == "instance-list-malformed":
        return bytes.fromhex("91 00 00 00 02 01 00")
    if mode == "instance-list-stalled":
        assert after == 300
        return bytes.fromhex("91 00 00 00 01 2C 01")
    payload = {0: "03 01 00 2C 00 2C 01", 300: "02 58 02 00 00", 600: "00"}[after]
    return bytes.fromhex("91 00 00 00 " + payload)


def explicit_write_reply(cip, mode, attrs, report):
    route, message = routed_request(cip) if cip[0] == 0x52 else (b"", cip)
    assert route in (b"", b"\x01\x03")
    if message == bytes.fromhex("01 02 20 01 24 01"):
        assert "identity_route" not in report and not report.get("writes")
        report["identity_route"] = route.hex()
        return bytes.fromhex("81 00 00 00") + b"".join(attrs)
    if message == bytes.fromhex("0E 03 20 04 24 9A 30 03"):
        current_check = mode.startswith("explicit-write-current")
        if current_check and not report.get("writes"):
            assert report["identity_route"] == route.hex() and "current_reads" not in report
            report["current_reads"] = [message.hex()]
            if mode.endswith("-no-reply"):
                return None
            if mode.endswith("-rejected"):
                return bytes.fromhex("8E 00 2C 01 02 03")
            if mode.endswith("-router"):
                assert route
                return bytes.fromhex("D2 00 01 01 04 02")
            if mode == "explicit-write-current-matches":
                return bytes.fromhex("8E 00 00 00 A5 16 2C 93")
            value = "10 20 30 41" if mode.endswith("-mismatch") else "10 20 30 40"
            return bytes.fromhex("8E 00 00 00 " + value)
        assert (current_check or mode.startswith("explicit-write-readback")) and report.get("writes")
        assert report["identity_route"] == route.hex() and "readbacks" not in report
        report["readbacks"] = [message.hex()]
        if mode.endswith("-no-reply"):
            return None
        if mode.endswith("-rejected"):
            return bytes.fromhex("8E 00 2C 01 02 03")
        if mode.endswith("-router"):
            assert route
            return bytes.fromhex("D2 00 01 01 04 02")
        value = "A5 16 2C 92" if mode.endswith("-mismatch") else "A5 16 2C 93"
        return bytes.fromhex("8E 00 00 00 " + value)
    assert mode != "explicit-write-review"
    if mode.startswith("explicit-write-current"):
        assert mode == "explicit-write-current" and report["current_reads"] == ["0e032004249a3003"]
    assert report["identity_route"] == route.hex() and "writes" not in report
    assert message[:8] == bytes.fromhex("10 03 20 04 24 9A 30 03")
    assert message[8:] == bytes.fromhex("A5 16 2C 93")
    report["writes"] = [message.hex()]
    if mode == "explicit-write-no-reply":
        return None
    if mode == "explicit-write-rejected":
        return bytes.fromhex("90 00 0E 01 02 03")
    if mode == "explicit-write-router-timeout":
        return bytes.fromhex("D2 00 01 01 04 02")
    return bytes.fromhex("90 00 00 00")


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
            reply = identity_attribute_reply(request, attrs)
            rejected |= reply[2] != 0
            replies.append(reply)
    offsets = bytearray()
    offset = 2 + len(replies) * 2
    for reply in replies:
        offsets += struct.pack("<H", offset)
        offset += len(reply)
    return bytes((0x8A, 0, 0x1E if rejected else 0, 0)) + struct.pack("<H", count) + offsets + b"".join(replies)


def routed_packet_reply(cip, attrs, report):
    route, message = routed_request(cip)
    report.update(route_hex=route.hex(), embedded_hex=message.hex())
    assert message[:6] == bytes.fromhex("0A 02 20 02 24 01")
    if route == b"\x01\x07":
        return bytes.fromhex("D2 00 01 01 11 03")
    if route == b"\x01\x08":
        return bytes.fromhex("8A 00 08 00")
    assert route in (b"\x01\x03", b"\x01\x02\x12\x0B192.0.2.100\x00\x01\x00")
    return multiple_service_reply(message, attrs)


def routed_identity_reply(cip, attrs, route, mode, report):
    received_route, message = routed_request(cip)
    assert received_route == route
    requested = report.setdefault("routed_identity_attributes", [])
    if message == bytes.fromhex("01 02 20 01 24 01"):
        requested.append("all")
        if mode == "routed-io-rejected-route":
            return bytes.fromhex("D2 00 01 01 11 03")
        if mode in ROUTED_KEYED_MODES:
            return bytes.fromhex("81 00 08 00")
        return bytes.fromhex("81 00 00 00") + b"".join(attrs)
    assert mode in ROUTED_KEYED_MODES
    assert message[:7] == bytes.fromhex("0E 03 20 01 24 01 30") and len(message) == 8
    attribute = message[7]
    assert 1 <= attribute <= 7
    requested.append(attribute)
    return bytes.fromhex("8E 00 00 00") + attrs[attribute]


def serve(tcp, udp, source, mode, report, serial_delta=0, multicast_port=None):
    stop = threading.Event()
    io = None
    cyclic_mode = ROUTED_OWNER_CYCLIC_MODES.get(mode, mode)
    owner = cyclic_mode == "owner" or cyclic_mode in VARIABLE_OUTPUT_MAX
    unexpected_peer = mode in ("routed-io-unexpected-peer", "routed-io-owner-unexpected-peer")
    connected = mode.startswith("class3-")
    connected_packet = connected and "packet" in mode
    connected_large = connected and "large" in mode
    connected_write = connected and "write" in mode
    connected_size = 4002 if connected_large else 511 if mode == "class3-sized" else 504
    explicit_write = mode.startswith("explicit-write")
    object_readers = {
        "attribute-list": attribute_list_reply,
        "class-info": class_info_reply,
        "port-info": port_info_reply,
        "tcpip-info": tcpip_info_reply,
        "acd-info": acd_info_reply,
        "link-info": link_info_reply,
        "lldp-neighbor": lldp_neighbor_reply,
        "dlr-info": dlr_info_reply,
        "dlr-participants": dlr_participants_reply,
        "dlr-paged": dlr_paged_reply,
        "time-sync": time_sync_reply,
        "manager-info": manager_info_reply,
        "connection-data": connection_data_reply,
        "qos-info": qos_info_reply,
        "assembly-info": assembly_info_reply,
        "profile-assemblies": profile_assemblies_reply,
        "object-list": object_list_reply,
        "instance-list": instance_list_reply,
    }
    object_reader = next((reader for name, reader in object_readers.items() if mode.startswith(name)), None)
    report_explicit = (
        connected or explicit_write or mode in ("saved-reads", "multiple-service")
        or object_reader is not None
    )
    multicast_mode = mode.removeprefix("routed-io-")
    multicast = multicast_mode.startswith("multicast")
    valid_multicast = multicast_mode in ("multicast", "multicast-large")
    default_unicast = mode.endswith("default-unicast")
    udp_mode = "stall" if mode == "stall-close-reject" else cyclic_mode
    adapter_ip = tcp.getsockname()[0]
    scanner_ip = tcp.getpeername()[0]
    adapter_index = int(IPv4Address(adapter_ip)) - int(IPv4Address(IP))
    serial_number = 0x12345678 + adapter_index + serial_delta
    value_base = (adapter_index % 1000) * 100_000
    report.update(serial_number=serial_number, value_base=value_base)
    path = bytes.fromhex("20 04 24 01 2C 64 2C 65" if owner else "20 04 24 01 2C FE 2C 65")
    if connected:
        path = bytes.fromhex("20 02 24 01")
    keyed = mode == "large-keyed" or mode in ROUTED_KEYED_MODES
    if keyed:
        path = bytes.fromhex("34 04 01 00 0C 00 01 00 01 00") + path
    route = b""
    if mode.startswith("routed-io"):
        route = b"\x01\x02\x12\x09127.0.0.3\x00\x01\x00" if keyed else b"\x01\x03"
    if mode.startswith("class3-routed"):
        route = (b"\x01\x02\x12\x0B192.0.2.100\x00\x01\x00"
                 if connected_packet or connected_write else b"\x01\x03")
    if mode.endswith("-extended-port"):
        route = EXTENDED_ROUTE
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
                if report_explicit:
                    report["registrations"] = report.get("registrations", 0) + 1
                response = request
            elif cmd == 0x66:
                assert handle == SESSION
                if report_explicit:
                    report["unregistered"] = True
                break
            elif cmd == 0x70 and connected:
                assert handle == SESSION and ctx == bytes(8) and request[4:6] == b"\0\0"
                assert "open_path_hex" in report and not report["forward_close"]
                assert not mode.endswith("open-reject")
                items = rr_items(request)
                assert set(items) == {0xA1, 0xB1} and items[0xA1] == struct.pack("<I", OT_ID)
                assert len(items[0xB1]) <= connected_size
                requests = report.setdefault("connected_requests", [])
                sequence, message = items[0xB1][:2], items[0xB1][2:]
                assert sequence == struct.pack("<H", len(requests) + 1)
                requests.append(message.hex())
                if connected_write:
                    write_mode = {
                        "class3-write-current-changed": "explicit-write-current-mismatch",
                        "class3-write-current-matches": "explicit-write-current-matches",
                        "class3-write-current-matches-close-reject": "explicit-write-current-matches",
                    }.get(mode, "explicit-write-current")
                    cip = explicit_write_reply(message, write_mode, attrs, report)
                    if message[0] == 0x10 and mode.endswith("-bad-sequence"):
                        sequence = struct.pack("<H", len(requests) + 1)
                elif connected_packet:
                    cip = multiple_service_reply(message, attrs)
                else:
                    cip = identity_attribute_reply(message, attrs)
                if mode == "class3-routed-packet-unsupported":
                    cip = bytes.fromhex("8A 00 08 00")
                report.setdefault("connected_reply_sizes", []).append(len(cip) + 2)
                response = (struct.pack("<IHHHHIHH", 0, 0, 2, 0xA1, 4, to, 0xB1, len(cip) + 2)
                            + sequence + cip)
            elif cmd == 0x6F:
                assert handle == SESSION
                items = rr_items(request)
                cip = items[0xB2]
                service, words = cip[:2]
                if explicit_write:
                    assert set(items) == {0, 0xB2} and not items[0]
                    result = explicit_write_reply(cip, mode, attrs, report)
                    if result is None:
                        break
                    response = rr_reply(result)
                    tcp.sendall(struct.pack("<HHII8sI", cmd, len(response), SESSION, 0, ctx, 0) + response)
                    continue
                if object_reader:
                    assert set(items) == {0, 0xB2} and not items[0]
                    response = rr_reply(object_reader(cip, mode, report))
                    tcp.sendall(struct.pack("<HHII8sI", cmd, len(response), SESSION, 0, ctx, 0) + response)
                    continue
                if mode in ("saved-reads", "multiple-service"):
                    report.setdefault("explicit_requests", []).append(cip.hex())
                if mode == "multiple-service":
                    assert set(items) == {0, 0xB2} and not items[0]
                    result = (routed_packet_reply(cip, attrs, report) if service == 0x52
                              else multiple_service_reply(cip, attrs))
                    response = rr_reply(result)
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
                if connected:
                    assert object_class == 6 and service in (0x54, 0x5B, 0x4E)
                body = cip[6:]
                if service == 1 and object_class == 1:
                    assert not route, "Read the routed target identity before opening I/O"
                    response = rr_reply(bytes.fromhex("81 00 08 00") if mode == "explicit"
                                        else b"\x81\0\0\0" + b"".join(attrs))
                elif connected and service in (0x54, 0x5B) and object_class == 6:
                    assert set(items) == {0, 0xB2} and not items[0]
                    _, to, serial, vendor, origin = struct.unpack_from("<IIHHI", body, 2)
                    assert service == (0x5B if connected_large else 0x54)
                    if connected_large:
                        fields, flags, path_offset = "<IIIIBB", 0x42000000, 40
                    else:
                        fields, flags, path_offset = "<IHIHBB", 0x4200, 36
                    ot, ot_params, to_rpi, to_params, transport, path_words = struct.unpack_from(fields, body, 22)
                    assert ot_params == to_params == flags | connected_size
                    assert transport == 0xA3 and path_words == len(path) // 2
                    assert body[path_offset:] == path and to and serial and origin
                    report.update(open_path_hex=body[path_offset:].hex(), open_service=service,
                                  class3_size=connected_size)
                    reply = struct.pack("<IIHHIIIBB", OT_ID, to, serial, vendor, origin, ot, to_rpi, 0, 0)
                    if mode in ("class3-routed-open-reject", "class3-write-open-reject"):
                        response = rr_reply(bytes.fromhex("D4 00 01 01 11 03"))
                    elif mode == "class3-large-open-reject":
                        response = rr_reply(bytes.fromhex("DB 00 01 02 26 01 F8 01"))
                    else:
                        response = rr_reply(bytes((service | 0x80, 0, 0, 0)) + reply)
                elif service in (0x54, 0x5B) and object_class == 6:
                    assert io is None
                    if route:
                        assert report["routed_identity_attributes"] == (["all", *range(1, 8)] if keyed else ["all"])
                    _, to, serial, vendor, origin = struct.unpack_from("<IIHHI", body, 2)
                    if keyed or cyclic_mode in ("large-variable-input", "large-variable-output"):
                        assert service == 0x5B
                        ot_rpi, ot_params, to_rpi, to_params, transport, path_words = struct.unpack_from(
                            "<IIIIBB", body, 22
                        )
                        expected_input = {
                            "large-variable-input": 0x4600025E,
                            "large-variable-output": 0x4400000A,
                            "large-keyed": 0x4400025A,
                            "routed-io-large": 0x4400025A,
                            "routed-io-multicast-large": 0x2400025A,
                            "routed-io-large-default-unicast": 0x4400025A,
                            "routed-io-large-extended-port": 0x4400025A,
                        }[cyclic_mode]
                        expected_output = 0x4600025A if cyclic_mode == "large-variable-output" else 0x44000002
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
                    if route:
                        report.update(open_path_hex=body[path_offset:].hex(), open_service=service)
                    if default_unicast:
                        assert set(items) == {0, 0xB2}
                        port = 2222
                        report["input_sockaddr_omitted"] = True
                    else:
                        family, port, address = struct.unpack(">HH4s8x", items[0x8001])
                        assert family == 2 and port != 0
                        assert socket.inet_ntoa(address) == ("0.0.0.0" if multicast else scanner_ip)
                    assert body[18] <= 7 and body[19:22] == bytes(3)
                    report.update(ot_requested_us=ot_rpi, to_requested_us=to_rpi,
                                  timeout_multiplier_code=body[18], open_request_triad={
                                      "connection_serial": serial,
                                      "originator_vendor": vendor,
                                      "originator_serial": origin,
                                  })
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
                        output_ip = "127.0.0.3" if unexpected_peer else adapter_ip
                        extras = [] if mode == "default-unicast" else [(0x8000, sockaddr(output_ip, adapter_port))]
                        if default_unicast:
                            report["output_sockaddr_omitted"] = not extras
                        group = None
                        if multicast:
                            # Supply a different T->O ID to model an existing producer.
                            to = 0xBCDE8765
                            reply = struct.pack("<IIHHIIIBB", OT_ID, to, serial, vendor, origin,
                                                ot_api, to_api, 0, 0)
                            report["requested_input_port"] = port
                            if multicast_mode != "multicast-missing":
                                group = (MULTICAST_GROUP if valid_multicast else adapter_ip, multicast_port)
                                extras.append((0x8001, sockaddr(*group)))
                        response = rr_reply(bytes([service | 0x80, 0, 0, 0]) + reply, *extras)
                        report["cyclic_io"] = {
                            "local_udp": f"{scanner_ip}:{port}",
                            "output_destination": f"{output_ip}:{adapter_port}",
                            "ot_connection_id": OT_ID,
                            "to_connection_id": to,
                        }
                        if mode == "delayed-start":
                            time.sleep(0.05)
                        if (not multicast or valid_multicast) and not unexpected_peer:
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
                    if mode in ("close-reject", "stall-close-reject",
                                "class3-packet-close-reject", "class3-write-close-reject",
                                "class3-write-current-matches-close-reject"):
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


def list_services_reply(request, source, mode, report):
    command, size, session, status, context, options = struct.unpack("<HHII8sI", request)
    assert (command, size, session, status, options) == (4, 0, 0, 0, 0)
    report["request_hex"] = request.hex()
    report["source_ip"] = source[0]
    flags = 0x0020 if mode == "list-services-tcp" else 0x8120
    communications = struct.pack("<HHHH16s", 0x0100, 20, 1, flags, b"Communications")
    unknown = bytes.fromhex("01 90 03 00 A5 16 2C")
    data = b"\x02\x00" + communications + unknown
    if mode == "list-services-malformed":
        data = data[:-1]
    if mode == "list-services-rejected":
        data, status = b"", 1
    if mode == "list-services-header":
        session = 0xA5162C31
    if mode == "list-services-stream-context":
        context = bytes([context[0] ^ 1]) + context[1:]
    return struct.pack("<HHII8sI", 4, len(data), session, status, context, 0) + data


def serve_list_services(address, mode, report):
    with socket.socket(type=socket.SOCK_DGRAM) as peer:
        peer.bind((address, 44818))
        peer.settimeout(10)
        print("Ready on loopback", flush=True)
        request, source = peer.recvfrom(256)
        reply = list_services_reply(request, source, mode, report)
        if mode == "list-services":
            context = request[12:20]
            unrelated = bytes([context[0] ^ 1]) + context[1:]
            peer.sendto(struct.pack("<HHII8sI", 4, 0, 0, 1, unrelated, 0), source)
        peer.sendto(reply, source)


def list_identity_stream_reply(request, source, mode, report):
    command, size, session, status, context, options = struct.unpack("<HHII8sI", request)
    assert (command, size, session, status, options) == (0x63, 0, 0, 0, 0)
    report["request_hex"] = request.hex()
    report["source_ip"] = source[0]
    name = b"TCP Loopback Adapter"
    name_length = len(name) + int(mode == "list-identity-stream-malformed")
    identity = struct.pack("<HHHBBHIB", 1, 12, 1, 2, 3, 0x60, 0x12345678, name_length)
    identity = struct.pack("<H", 1) + sockaddr("127.0.0.9", 44819) + identity + name + b"\x03"
    item = struct.pack("<HH", 0x0C, len(identity)) + identity
    payload = bytes.fromhex("03 00 01 90 01 00 A5") + item + item
    if mode == "list-identity-stream-context":
        context = bytes([context[0] ^ 1]) + context[1:]
    if mode == "list-identity-stream-rejected":
        payload, status = b"", 1
    return struct.pack("<HHII8sI", command, len(payload), session, status, context, options) + payload


def serve_encapsulation_stream(address, mode, report):
    with socket.socket() as listener:
        listener.bind((address, 44818))
        listener.listen(1)
        listener.settimeout(10)
        print("Ready on loopback", flush=True)
        connection, source = listener.accept()
        with connection:
            connection.settimeout(5)
            connection.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            request = recv_exact(connection, 24)
            make_reply = list_identity_stream_reply if mode.startswith("list-identity-stream") else list_services_reply
            reply = make_reply(request, source, mode, report)
            if mode == "list-services-stream-truncated":
                connection.sendall(reply[:-1])
                connection.shutdown(socket.SHUT_WR)
            else:
                for chunk in (reply[:9], reply[9:26], reply[26:]):
                    connection.sendall(chunk)
                    time.sleep(0.025)
            report["tcp_eof"] = connection.recv(1) == b""
            assert report["tcp_eof"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path)
    parser.add_argument("--mode", default="normal", choices=(
        "normal", "faults", "stall", "no-input", "invalid-input", "reject", "different-api", "owner",
        "reconnect", "recover", "recover-slower-api", "replacement", "qualify", "qualify-different-api",
        "qualify-replacement", "close-reject", "close-mismatch", "close-short", "close-eof",
        "stall-close-reject", "qualify-close-reject",
        "slow-input", "pause-different-api",
        "identity-then-owner", "routed-owner", "routed-large-owner",
        "routed-owner-replacement", "routed-owner-unexpected-peer",
        "explicit", "saved-reads", "multiple-service", "large-keyed", "routed", "variable-input", "large-variable-input",
        "attribute-list", "attribute-list-mixed", "attribute-list-malformed", "attribute-list-unsupported",
        "class-info", "class-info-malformed", "class-info-empty", "port-info", "port-info-optional", "port-info-malformed",
        "tcpip-info-diagnostics", "tcpip-info-diagnostics-optional", "tcpip-info-diagnostics-malformed",
        "acd-info", "acd-info-probe", "acd-info-clear", "acd-info-unsupported", "acd-info-malformed",
        "acd-info-unknown-arp",
        "tcpip-info", "tcpip-info-optional", "tcpip-info-malformed",
        "link-info", "link-info-optional", "link-info-malformed",
        "link-info-details", "link-info-details-optional", "link-info-details-malformed",
        "lldp-neighbor", "lldp-neighbor-empty", "lldp-neighbor-optional", "lldp-neighbor-malformed",
        "lldp-neighbor-gui",
        "dlr-info", "dlr-info-optional", "dlr-info-empty", "dlr-info-malformed",
        "dlr-participants", "dlr-participants-changed", "dlr-participants-mismatch", "dlr-participants-empty",
        "dlr-participants-malformed", "dlr-participants-rejected", "dlr-participants-no-count",
        "dlr-participants-large", "dlr-participants-overflow",
        "dlr-paged", "dlr-paged-empty", "dlr-paged-short", "dlr-paged-malformed",
        "dlr-paged-rejected", "dlr-paged-exchange", "dlr-paged-stalled",
        "dlr-paged-limit", "dlr-paged-overflow",
        "time-sync", "time-sync-optional", "time-sync-empty", "time-sync-malformed",
        "manager-info", "manager-info-optional", "manager-info-malformed",
        "connection-data", "connection-data-malformed", "connection-data-mismatch",
        "connection-data-unsupported", "connection-data-not-found",
        "qos-info", "qos-info-optional", "qos-info-malformed",
        "assembly-info", "assembly-info-size-unsupported", "assembly-info-data-unsupported",
        "assembly-info-malformed", "assembly-info-empty",
        "profile-assemblies", "profile-assemblies-malformed",
        "object-list", "object-list-empty", "object-list-malformed", "object-list-zero",
        "object-list-unsupported",
        "instance-list", "instance-list-unsupported", "instance-list-malformed", "instance-list-stalled",
        "instance-list-gui",
        "list-services", "list-services-tcp", "list-services-malformed",
        "list-services-rejected", "list-services-header",
        "list-services-stream", "list-services-stream-context", "list-services-stream-truncated",
        "list-identity-stream", "list-identity-stream-context", "list-identity-stream-rejected",
        "list-identity-stream-malformed",
        "explicit-write", "explicit-write-review", "explicit-write-gui", "explicit-write-rejected",
        "explicit-write-no-reply", "explicit-write-router-timeout",
        "explicit-write-readback", "explicit-write-readback-mismatch", "explicit-write-readback-rejected",
        "explicit-write-readback-no-reply", "explicit-write-readback-router",
        "explicit-write-current", "explicit-write-current-mismatch", "explicit-write-current-rejected",
        "explicit-write-current-no-reply", "explicit-write-current-router", "explicit-write-gui-current",
        "class3-packet", "class3-packet-close-reject",
        "class3-routed", "class3-routed-extended-port", "class3-routed-open-reject",
        "class3-routed-packet", "class3-routed-packet-unsupported",
        "class3-sized", "class3-large-packet", "class3-routed-large-packet", "class3-large-open-reject",
        "class3-write", "class3-routed-large-write", "class3-write-open-reject",
        "class3-write-close-reject", "class3-write-bad-sequence", "class3-write-gui",
        "class3-write-gui-unconfirmed",
        "class3-write-current-matches", "class3-write-current-matches-close-reject",
        "class3-write-inspect-gui",
        "routed-io", "routed-io-large", "routed-io-large-extended-port",
        "routed-io-rejected-route", "routed-io-unexpected-peer",
        "variable-output", "large-variable-output",
        "multicast", "multicast-missing", "multicast-unicast",
        "routed-io-multicast", "routed-io-multicast-large",
        "routed-io-multicast-missing", "routed-io-multicast-unicast",
        "delayed-start", "split-udp", "default-unicast", "routed-io-large-default-unicast",
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
    if args.mode.startswith(("list-services", "list-identity-stream")):
        serve_discovery = (
            serve_encapsulation_stream if args.mode.startswith(("list-identity-stream", "list-services-stream"))
            else serve_list_services
        )
        serve_discovery(address, args.mode, report)
        args.report.write_text(json.dumps(report), encoding="utf-8")
        return
    with socket.socket() as listener:
        listener.bind((address, 44818))
        listener.listen(1)
        listener.settimeout(30)
        print("Ready on loopback", flush=True)
        sequences = {
            "instance-list-gui": ("instance-list", "instance-list"),
            "lldp-neighbor-gui": ("lldp-neighbor", "lldp-neighbor"),
            "explicit-write-gui": ("explicit-write-review", "explicit-write-review", "explicit-write-readback"),
            "explicit-write-gui-current": ("explicit-write-review", "explicit-write-review", "explicit-write-current"),
            "class3-write-gui": ("explicit-write-review", "class3-write", "class3-write"),
            "class3-write-gui-unconfirmed": (
                "explicit-write-review", "class3-write", "class3-write-bad-sequence", "class3-write-current-matches",
            ),
            "class3-write-inspect-gui": ("class3-write", "class3-write", "class3-write-current-changed"),
            "reconnect": ("normal", "normal"),
            "recover": ("stall", "normal"),
            "recover-slower-api": ("stall", "slower-api"),
            "replacement": ("stall", "normal"),
            "qualify": ("normal", "normal", "normal", "normal"),
            "qualify-replacement": ("normal", "normal"),
            "qualify-different-api": ("normal", "different-api", "normal", "normal"),
            "identity-then-owner": ("normal", "owner"),
            "routed-owner": ("routed-io-owner", "routed-io-owner"),
            "routed-large-owner": ("routed-io-large-owner", "routed-io-large-owner"),
            "routed-owner-replacement": ("routed-io-owner", "routed-io-owner"),
            "routed-owner-unexpected-peer": ("routed-io-owner", "routed-io-owner-unexpected-peer"),
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
                udp.bind((address, 2222 if mode == "default-unicast" else 0))
                attempt = {"forward_close": False} if record_attempts else report
                replacement = args.mode in ("replacement", "qualify-replacement", "routed-owner-replacement") and index == 1
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
