#!/usr/bin/env python3
"""Validate an optional original-process sidecar; never accept a release gate."""

import argparse
import hashlib
import importlib.util
import json
from pathlib import Path


SPEC = importlib.util.spec_from_file_location(
    "checked_layout_identity", Path(__file__).with_name("diagnose-checked-aarch64.py"))
IDENTITY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(IDENTITY)
CHANNELS = IDENTITY.CHANNELS
FIELDS = ("wallNs", "threadCpuNs", "cpu", "userCpuNs", "systemCpuNs",
          "minorFaults", "majorFaults", "voluntarySwitches", "involuntarySwitches")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def reject_constant(value):
    raise ValueError(f"non-finite JSON value: {value}")


def read_json(text):
    return json.loads(text, object_pairs_hook=IDENTITY.strict_object,
                      parse_constant=reject_constant)


def schedule():
    for warmup, rounds, repetitions in ((True, 3, 1), (False, 20, 7)):
        for round_index in range(rounds):
            for repetition in range(repetitions):
                for offset in range(3):
                    yield warmup, round_index, (round_index + repetition + offset) % 3


def analyze(report_path):
    # Reuse the existing strict regular-file, safe-path and SHA-256 verification.
    libraries = IDENTITY.resolve_libraries(report_path)
    report = read_json(report_path.read_text())
    directory = report_path.parent / report["evidenceDirectory"]
    raw_path = directory / "checked-runtime-observations.jsonl"
    raw_identity = IDENTITY.file_identity(raw_path)
    rows = [read_json(line) for line in raw_path.read_text().splitlines()]
    require(len(rows) == 431 and all(isinstance(row, dict) for row in rows),
            "missing or extra original-process rows")
    identity, footer = rows[0], rows[-1]
    for key, value in dict(type="identity", schemaVersion=1, acceptance=False,
                           case="specialized_length", mode="checked", warmup=3,
                           iterations=20, repetitions=7, calls=5000, elements=20000000).items():
        require(type(identity.get(key)) is type(value) and identity[key] == value,
                f"observed identity/work changed: {key}")
    for key in ("pid", "inputAddress", "outputAddress"):
        require(type(identity.get(key)) is int and identity[key] > 0, f"invalid {key}")
    entries = identity.get("entries")
    require(isinstance(entries, list) and len(entries) == 3
            and all(type(value) is int and value > 0 for value in entries), "invalid entries")
    expected_libraries = [dict(file=Path(item["path"]).name, bytes=item["bytes"],
                               sha256=item["sha256"]) for item in libraries]
    require(identity.get("libraries") == expected_libraries, "observed library identity changed")
    values = [((i + 7) * 2654435761 & 0xffffffff) % 1000002 + 1 for i in range(4000)]
    for key, content in (("inputSha256", values), ("resultDigest", [v + 13 for v in values])):
        expected = hashlib.sha256(b"".join(v.to_bytes(4, "little") for v in content)).hexdigest()
        require(identity.get(key) == expected, f"observed {key} changed")
    cases = [case for suite in report["vectorSuites"] if suite.get("mode") == "checked"
             for case in suite["cases"] if case.get("name") == "specialized_length"]
    require(len(cases) == 1, "missing or duplicate original report case")
    case = cases[0]
    require(case.get("batchIterations") == 20000000
            and case.get("resultDigest") == identity["resultDigest"], "report work/result changed")
    for key, rounds in (("warmupOrder", 3), ("sampleOrder", 20)):
        require(case.get(key) == [[(i + j) % 3 for j in range(3)] for i in range(rounds)],
                f"report {key} changed")
    require(footer == dict(type="complete", rows=429, samplingSucceeded=True, overflow=False),
            "incomplete original-process sampling")
    samples = [[[] for _ in range(20)] for _ in CHANNELS]
    deltas = [{field: [] for field in FIELDS if field != "cpu"} for _ in CHANNELS]
    cpus = set()
    for sequence, (row, (warmup, round_index, channel)) in enumerate(zip(rows[1:-1], schedule())):
        require(row.get("type") == "sample" and type(row.get("sequence")) is int
                and row["sequence"] == sequence and type(row.get("channel")) is int
                and row["channel"] == channel and row.get("warmup") is warmup,
                "original-process sample order changed")
        duration = row.get("gateNs")
        require(type(duration) is int and duration > 0, "missing/invalid original gate duration")
        if not warmup:
            samples[channel][round_index].append(duration)
        for edge in ("before", "after"):
            snapshot = row.get(edge)
            require(isinstance(snapshot, dict) and set(snapshot) == set(FIELDS), "invalid snapshot")
            for value in snapshot.values():
                require(value is None or (type(value) is int and value >= 0), "invalid resource counter")
            if snapshot["cpu"] is not None:
                cpus.add(snapshot["cpu"])
        for field in deltas[channel]:
            before, after = row["before"][field], row["after"][field]
            if before is not None and after is not None:
                require(after >= before, f"resource counter went backwards: {field}")
                if not warmup:
                    deltas[channel][field].append(after - before)
    channels = []
    for channel, name in enumerate(CHANNELS):
        stored = [IDENTITY.upper_median(values) for values in samples[channel]]
        median = IDENTITY.upper_median(stored)
        require(stored == case.get(name + "SamplesNs") and median == case.get(name + "MedianNs"),
                f"original report median/sample mismatch: {name}")
        channels.append(dict(channel=name, medianNs=median,
                             availableDeltas={k: len(v) for k, v in deltas[channel].items()},
                             positiveDeltas={k: sum(x > 0 for x in v) for k, v in deltas[channel].items()},
                             medianOuterDeltas={k: IDENTITY.upper_median(v) if v else None
                                                for k, v in deltas[channel].items()}))
    return dict(acceptance=False, rawRows=429, rawIdentity=raw_identity,
                reportIdentity=IDENTITY.file_identity(report_path), observedCpus=sorted(cpus),
                channels=channels,
                limitations="Outer snapshots include result hashing and boundary work, are not atomic, "
                             "may perturb process state, and do not establish historical causality.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path, help="original schema-7 report with retained evidence directory")
    args = parser.parse_args()
    try:
        print(json.dumps(analyze(args.report), indent=2))
    except (OSError, ValueError, KeyError, TypeError) as error:
        parser.exit(1, f"optional original-process evidence invalid/unavailable: {error}\n")


if __name__ == "__main__":
    main()
