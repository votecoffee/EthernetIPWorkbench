"""Validate and join a single Assembly Scanner capture (including rotated parts)."""
import argparse
from collections import Counter
from contextlib import nullcontext
import csv
import json
from pathlib import Path, PureWindowsPath
import tomllib

METADATA = [
    "capture_start_utc", "elapsed_us", "sample_index", "connection_generation",
    "packet_sequence", "cip_sequence",
]
SOCKET_FIELDS = (
    "generation", "state", "requested_ot_us", "requested_to_us", "actual_ot_us", "actual_to_us",
    "accepted", "sequence_gaps", "duplicates", "late", "invalid", "timeouts",
    "sent", "skipped_deadlines", "forward_close", "receive_spacing", "send_spacing",
    "identity_error", "cip_error", "network_error",
)


class SavedTiming:
    def __init__(self, generation, elapsed):
        self.generation = generation
        self.first = self.last = elapsed
        self.last_packet = None
        self.rows = self.gaps = self.repeats = self.non_forward = 0
        self.intervals = Counter()

    def record(self, elapsed, packet):
        if self.last_packet is not None:
            self.intervals[elapsed - self.last] += 1
            delta = (packet - self.last_packet) % (1 << 32)
            if delta == 0:
                self.repeats += 1
            elif delta < 1 << 31:
                self.gaps += delta - 1
            else:
                self.non_forward += 1
        self.last_packet = packet
        self.last = elapsed
        self.rows += 1

    def summary(self):
        intervals = self.rows - 1
        span = self.last - self.first
        spacing = {"intervals": intervals, "span_us": span}
        if intervals:
            ordered = sorted(self.intervals.items())
            for percentile in (50, 95, 99):
                rank = (intervals * percentile + 99) // 100
                for elapsed, count in ordered:
                    rank -= count
                    if rank <= 0:
                        spacing[f"p{percentile}_us"] = elapsed
                        break
            spacing["max_us"] = ordered[-1][0]
        if span:
            spacing["observed_rows_per_second"] = intervals * 1_000_000 / span
        return {
            "generation": self.generation, "rows": self.rows,
            "first_elapsed_us": self.first, "last_elapsed_us": self.last,
            "saved_packet_sequence_gaps": self.gaps,
            "saved_packet_sequence_repeats": self.repeats,
            "saved_packet_sequence_non_forward": self.non_forward,
            "spacing": spacing,
        }


def read_header(part):
    with part.open(encoding="utf-8", newline="") as file:
        header = next(csv.reader(file, strict=True), [])
    if (header[:len(METADATA)] != METADATA or len(header) <= len(METADATA)
            or any(not name for name in header)
            or len(set(name.lower() for name in header)) != len(header)):
        raise ValueError(f"Unexpected header in {part}")
    return header


def rows(parts, header=None):
    if header is None:
        header = read_header(parts[0])
    start = None
    last_index = last_us = last_generation = -1
    for part in parts:
        with part.open(encoding="utf-8", newline="") as file:
            reader = csv.reader(file, strict=True)
            if next(reader, None) != header:
                raise ValueError(f"Unexpected header in {part}")
            for row in reader:
                if len(row) != len(header):
                    raise ValueError(f"Unexpected field count in {part}:{reader.line_num}")
                if start is None:
                    start = row[0]
                elapsed, index, generation, packet, cip = map(int, row[1:6])
                if (row[0] != start or index <= last_index or elapsed < last_us or
                        generation < last_generation or not 0 <= packet <= 0xFFFFFFFF or not 0 <= cip <= 0xFFFF):
                    raise ValueError(f"Wrong capture or row order in {part}:{reader.line_num}")
                last_index, last_us, last_generation = index, elapsed, generation
                yield row


def parts_for(prefix):
    result = sorted(prefix.parent.glob(f"{prefix.name}-*.csv"))
    if not result:
        raise ValueError(f"No files for capture prefix {prefix}")
    for number, path in enumerate(result, 1):
        if path.name != f"{prefix.name}-{number:06}.csv":
            raise ValueError(f"Missing or unrecognized capture part before {path}")
    return result


def counter(report, key):
    value = report.get(key)
    if type(value) is not int or value < 0:
        raise ValueError(f"Run report needs a nonnegative integer {key}")
    return value


def check_report(report, prefix, part_count):
    if report.get("mode") == "gui":
        report = report.get("capture")
        if not isinstance(report, dict):
            raise ValueError("GUI run report has no capture table")
    source = report.get("csv_prefix")
    if not isinstance(source, str) or PureWindowsPath(source).name != prefix.name:
        raise ValueError("Run report belongs to a different capture prefix")
    if "disk_error" in report:
        raise ValueError(f"Run report records a CSV writer error ({report['disk_error']})")
    seen, written, overruns = (counter(report, key) for key in ("seen", "written", "buffer_overruns"))
    if "unchanged" not in report and seen == written + overruns:
        # Earlier every-packet reports accounted for every row with these three counters.
        report = {**report, "unchanged": 0}
    unchanged = counter(report, "unchanged")
    if seen != written + unchanged + overruns:
        raise ValueError("Run report has inconsistent CSV row counts")
    if "csv_parts" in report:
        expected = counter(report, "csv_parts")
        if expected != part_count:
            raise ValueError(f"Run report lists {expected} CSV parts, found {part_count}")
    return report


def transfer_checked(parts, header, report, writer, timings=None):
    started = report.get("capture_start_utc", report.get("started_utc"))
    if started is not None and (not isinstance(started, str) or not started):
        raise ValueError("Run report has an invalid capture start timestamp")
    summaries = report.get("field_summaries", [])
    if not isinstance(summaries, list) or any(not isinstance(field, dict) for field in summaries):
        raise ValueError("Run report has invalid field summaries")
    if summaries and [field.get("name") for field in summaries] != header[len(METADATA):]:
        raise ValueError("CSV columns differ from the run report")
    first_values = [field.get("first_value") for field in summaries]
    last_values = [field.get("last_value") for field in summaries]
    count = generations = 0
    last_index = -1
    last_row = None
    generation = None
    for row in rows(parts, header):
        if started is not None and row[0] != started:
            raise ValueError("CSV start timestamp differs from the run report")
        if not count and summaries and row[len(METADATA):] != first_values:
            raise ValueError("First CSV values differ from the run report")
        count += 1
        last_index = int(row[2])
        last_row = row
        row_generation = int(row[3])
        if row_generation != generation:
            generations += 1
            generation = row_generation
            if timings is not None:
                timings.append(SavedTiming(generation, int(row[1])))
        if timings is not None:
            timings[-1].record(int(row[1]), int(row[4]))
        if writer is not None:
            writer.writerow(row)
    if count != report["written"]:
        raise ValueError(f"Run report lists {report['written']} written rows, found {count}")
    if summaries and (last_row is None or last_row[len(METADATA):] != last_values):
        raise ValueError("Last CSV values differ from the run report")
    missing_indexes = last_index + 1 - count
    trailing_overruns = report["buffer_overruns"] - missing_indexes
    if trailing_overruns < 0:
        raise ValueError(
            f"Found {missing_indexes} missing sample indexes but the run report lists "
            f"{report['buffer_overruns']} buffer overruns"
        )
    return {
        "rows": count, "parts": len(parts), "generation_count": generations,
        "sample_index_gaps": missing_indexes, "overruns_after_last_row": trailing_overruns,
    }


def timing_report(document, capture, audit, timings):
    def socket_fields(report):
        return {field: report[field] for field in SOCKET_FIELDS if field in report}

    gui = document.get("mode") == "gui"
    connection = document.get("connection") if gui else document
    attempts = document.get("attempts", [])
    if (connection is not None and not isinstance(connection, dict)
            or not isinstance(attempts, list) or any(not isinstance(run, dict) for run in attempts)):
        raise ValueError("Run report has invalid connection diagnostics")
    return {
        "report_kind": "gui" if gui else "cli",
        "requested_duration_seconds": capture.get("duration_seconds"),
        "duration_reached": capture.get("duration_reached"),
        "csv": {
            **audit,
            "seen": capture["seen"],
            "on_change_suppressed": capture["unchanged"],
            "buffer_overruns": capture["buffer_overruns"],
            "after_capture_deadline": capture.get("after_capture_deadline"),
            "generations": [timing.summary() for timing in timings],
        },
        "socket": socket_fields(connection) if connection is not None else None,
        "connection_attempts": [socket_fields(attempt) for attempt in attempts],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture_prefix", type=Path, help="Use the prefix shown in the app, without -000001.csv")
    parser.add_argument("output", type=Path, nargs="?", help="New joined CSV path")
    parser.add_argument("--report", type=Path, help="Verify against the capture.toml or GUI run report")
    parser.add_argument("--timing", action="store_true",
                        help="Report saved-row timing and socket diagnostics as JSON (requires --report)")
    args = parser.parse_args()
    if args.output is None and args.report is None:
        parser.error("provide a new joined CSV path or --report")
    if args.timing and args.report is None:
        parser.error("--timing requires --report")
    output_created = False
    audit = None
    try:
        parts = parts_for(args.capture_prefix)
        header = read_header(parts[0])
        document = report = None
        if args.report:
            with args.report.open("rb") as file:
                document = tomllib.load(file)
            report = check_report(document, args.capture_prefix, len(parts))
        destination = args.output.open("x", encoding="utf-8", newline="") if args.output else nullcontext()
        with destination as output:
            writer = None
            if output is not None:
                output_created = True
                writer = csv.writer(output, lineterminator="\r\n")
                writer.writerow(header)
            if report is None:
                writer.writerows(rows(parts, header))
            else:
                timings = [] if args.timing else None
                audit = transfer_checked(parts, header, report, writer, timings)
                if timings is not None:
                    audit = json.dumps(timing_report(document, report, audit, timings),
                                       indent=2, allow_nan=False)
                else:
                    audit = (
                        f"Verified the CSV capture. Saved row count {audit['rows']}, "
                        f"part count {audit['parts']}, and logged generation count {audit['generation_count']}. "
                        f"Sample-index gaps account for {audit['sample_index_gaps']} of "
                        f"{report['buffer_overruns']} reported overruns. "
                        f"Overruns after the last saved row total {audit['overruns_after_last_row']}."
                    )
    except (OSError, ValueError, TypeError, csv.Error) as error:
        if output_created:
            args.output.unlink(missing_ok=True)
        parser.exit(1, f"{error}\n")
    if audit:
        print(audit)


if __name__ == "__main__":
    main()
