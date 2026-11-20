"""Review the four saved GUI qualification runs and their CSV parts offline."""

import argparse
import csv
from ipaddress import IPv4Address
import json
from pathlib import Path, PureWindowsPath
import tomllib

from join_capture import METADATA, check_report, counter, parts_for, read_header, timing_report, transfer_checked

RUNS = (("baseline-8ms", 8000, 15), ("trial-1", 1000, 60), ("trial-2", 1000, 60), ("trial-3", 1000, 60))
IDENTITY_FIELDS = ("vendor", "device_type", "code", "serial")
PROFILE_FIELDS = (
    "adapter_ip", "local_ip", "io_route", "input_assembly", "input_bytes", "input_min_bytes",
    "input_multicast", "input_run_idle", "output_assembly", "schedule",
)


def expected_identity(text):
    limits = (65535, 65535, 65535, 0xFFFFFFFF)
    try:
        fields = tuple(int(value, 10) for value in text.split(":"))
        if len(fields) == len(limits) and all(0 <= value <= limit for value, limit in zip(fields, limits)):
            return fields
    except ValueError:
        pass
    raise argparse.ArgumentTypeError("use decimal vendor:type:product:serial")


def check_trial(document, errors, result, expected, loopback):
    capture, connection = document["capture"], document.get("connection")
    if not isinstance(connection, dict):
        errors.append("No final GUI connection report")
        return
    timing = result["timing"]
    csv_summary = timing["csv"]
    timeout = connection.get("connection_timeout", {})
    if not isinstance(timeout, dict):
        raise ValueError("Run report has invalid timeout settings")
    result["profile"] = {
        **{field: connection.get(field) for field in PROFILE_FIELDS},
        "version": document.get("version"),
        "timeout_multiplier_code": timeout.get("multiplier_code"),
    }
    addresses = [IPv4Address(connection.get(field)) for field in ("adapter_ip", "local_ip")]
    if any(address.is_loopback != loopback for address in addresses):
        errors.append("Adapter and NIC addresses do not match the declared link path")

    identity = connection.get("identity")
    if not isinstance(identity, dict) or tuple(identity.get(field) for field in IDENTITY_FIELDS) != expected:
        errors.append("Adapter identity differs from the expected vendor, device type, product, or serial")
    else:
        result["profile"]["revision"] = [identity.get("major"), identity.get("minor")]
    if (connection.get("input_only") is not True or connection.get("listen_only") is True
            or connection.get("output_run_idle") is not False
            or any(connection.get(field) != 0 for field in ("output_bytes", "configuration_bytes"))):
        errors.append("Run requires a controlling input-only heartbeat with zero output and configuration bytes")

    minimum = result["minimum_duration_seconds"]
    if capture.get("duration_reached") is not True or counter(capture, "duration_seconds") < minimum:
        errors.append(f"Timed capture did not complete the required {minimum} seconds")
    if capture.get("mode") != "every_packet" or csv_summary["on_change_suppressed"]:
        errors.append("Capture was not set to every packet")
    if connection.get("state") != "Closed" or connection.get("forward_close") != {"outcome": "confirmed"}:
        errors.append("Connection did not close with a confirmed Forward_Close")
    for error in ("identity_error", "cip_error", "network_error"):
        if error in connection:
            errors.append(f"Connection reported {error}: {connection[error]}")

    rpi = result["target_rpi_us"]
    for prefix, label in (("requested", "Requested"), ("actual", "Forward_Open")):
        if any(connection.get(f"{prefix}_{direction}_us") != rpi for direction in ("ot", "to")):
            errors.append(f"{label} intervals did not match {rpi} µs in both directions")
    for key in ("sequence_gaps", "duplicates", "late", "invalid", "timeouts"):
        count = counter(connection, key)
        if count:
            errors.append(f"Connection reported {count} {key.replace('_', ' ')}")
    for key in ("receive_spacing", "send_spacing"):
        spacing = connection.get(key)
        if not isinstance(spacing, dict) or not counter(spacing, "intervals"):
            errors.append(f"No {key.replace('_', ' ')} measurements")

    generation = connection.get("generation")
    logged = csv_summary["generations"]
    if document.get("attempts") != [connection] or [entry["generation"] for entry in logged] != [generation]:
        errors.append("Expected one completed connection generation in both the report and the CSV")
    if any(entry[f"saved_packet_sequence_{key}"] for entry in logged for key in ("gaps", "repeats", "non_forward")):
        errors.append("Saved packet sequences were not consecutive within the capture")
    if csv_summary["rows"] < 2 or csv_summary["buffer_overruns"] or counter(capture, "unconfirmed_rows"):
        errors.append("CSV capture has too few samples, writer overruns, or unconfirmed rows")
    if counter(connection, "accepted") < csv_summary["seen"] + counter(capture, "after_capture_deadline"):
        errors.append("CSV and post-deadline counts exceed accepted connection packets")


def review_trial(path, name, rpi, duration, expected, loopback):
    errors = []
    result = {
        "name": name, "report": str(path), "target_rpi_us": rpi,
        "minimum_duration_seconds": duration, "errors": errors,
    }
    try:
        with path.open("rb") as file:
            document = tomllib.load(file)
        capture = document.get("capture")
        if document.get("mode") != "gui" or not isinstance(capture, dict):
            raise ValueError("Expected a saved GUI run report")
        source, start = capture.get("csv_prefix"), capture.get("started_utc")
        if not isinstance(source, str) or not PureWindowsPath(source).name or not isinstance(start, str) or not start:
            raise ValueError("GUI report must identify the CSV prefix and start time")
        result["capture_started_utc"] = start
        prefix = path.with_name(PureWindowsPath(source).name)
        parts = parts_for(prefix)
        header = read_header(parts[0])
        capture = check_report(document, prefix, len(parts))
        timings = []
        audit = transfer_checked(parts, header, capture, None, timings)
        result["csv_fields"] = header[len(METADATA):]
        result["timing"] = timing_report(document, capture, audit, timings)
        check_trial(document, errors, result, expected, loopback)
    except (OSError, ValueError, TypeError, csv.Error) as error:
        errors.append(str(error))
    try:
        json.dumps(result, allow_nan=False)
    except (ValueError, TypeError) as error:
        result.pop("timing", None)
        result.pop("profile", None)
        errors.append(f"Invalid diagnostic value: {error}")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--expected", required=True, type=expected_identity, metavar="VENDOR:TYPE:PRODUCT:SERIAL")
    parser.add_argument("--link-path", required=True, choices=("direct", "switched", "loopback"))
    parser.add_argument("--loopback-seconds", type=int, metavar="SECONDS",
                        help="Use one shorter window for all four loopback runs (1–60 seconds)")
    parser.add_argument("reports", type=Path, nargs=4, metavar="REPORT",
                        help="Saved GUI baseline, trial 1, trial 2, and trial 3 reports, in that order")
    args = parser.parse_args()
    loopback = args.link_path == "loopback"
    override = args.loopback_seconds
    if override is not None and (not loopback or not 1 <= override <= 60):
        parser.error("--loopback-seconds requires a loopback review and a duration from 1 to 60")
    runs = [
        review_trial(path, name, rpi, override or duration, args.expected, loopback)
        for path, (name, rpi, duration) in zip(args.reports, RUNS)
    ]
    starts = set()
    baseline = runs[0]
    for run in runs:
        start = run.get("capture_started_utc")
        if start in starts:
            run["errors"].append("Capture start timestamp was already used by another run")
        if start:
            starts.add(start)
        for key, label in (("profile", "connection settings"), ("csv_fields", "CSV field selection")):
            if key in baseline and key in run and baseline[key] != run[key]:
                run["errors"].append(f"Run {label} differ from the baseline")

    passed = not any(run["errors"] for run in runs)
    print(json.dumps({
        "source": "saved_gui_reports",
        "link_path": args.link_path,
        "expected_identity": dict(zip(IDENTITY_FIELDS, args.expected)),
        "outcome": "passed" if passed else "failed",
        "runs": runs,
    }, indent=2, allow_nan=False))
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
