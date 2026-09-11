#!/usr/bin/env python3
"""One bounded code-layout comparison; never rewrites or accepts the original gate."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import subprocess


CHANNELS = ("candidate", "cSimd", "rustSimd")


def compiler_path(environment):
    # The release LLVM prefix intentionally does not contain Clang.
    return environment.get("CKC_CLANG_ORACLE") or "cc"


def strict_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def file_identity(path):
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"identity requires a regular non-symlink file: {path}")
    data = path.read_bytes()
    return {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}


def resolve_libraries(report_path):
    file_identity(report_path)
    report = json.loads(report_path.read_text(), object_pairs_hook=strict_object)
    if report.get("schemaVersion") != 7:
        raise ValueError("checked layout comparison requires the original schema-7 report")
    directory = report.get("evidenceDirectory")
    if not isinstance(directory, str) or not re.fullmatch(r"measurement-[0-9]+-[0-9]+", directory):
        raise ValueError("unsafe evidence directory")
    root = report_path.parent / directory
    if root.is_symlink() or not root.is_dir() or root.resolve().parent != report_path.parent.resolve():
        raise ValueError("missing or redirected evidence directory")
    selected = {}
    for record in report["oracleArtifacts"]:
        if (record.get("suite"), record.get("case"), record.get("mode")) != (
                "vector", "specialized_length", "checked"):
            continue
        channel = record.get("channel")
        if channel not in CHANNELS or channel in selected:
            raise ValueError("missing, extra or duplicate checked layout channel")
        name = f"vector-specialized_length-checked-{channel}.so"
        if record.get("file") != name:
            raise ValueError("unsafe checked layout library basename")
        path = root / name
        identity = file_identity(path)
        if type(record.get("bytes")) is not int or record["bytes"] <= 0 or identity != {
                "bytes": record["bytes"], "sha256": record.get("sha256")}:
            raise ValueError(f"library identity mismatch: {name}")
        selected[channel] = {"channel": channel, "path": str(path.resolve()), **identity}
    if set(selected) != set(CHANNELS):
        raise ValueError("missing checked layout channel")
    return [selected[channel] for channel in CHANNELS]


def expected_digests():
    inputs = [((index + 7) * 2654435761 & 0xffffffff) % 1000002 + 1 for index in range(4000)]
    result = []
    for values in (inputs, [value + 13 for value in inputs]):
        digest = 14695981039346656037
        for value in values:
            for byte in value.to_bytes(4, "little"):
                digest = ((digest ^ byte) * 1099511628211) & 0xffffffffffffffff
        result.append(f"{digest:016x}")
    return result


def schedule():
    for warmup, rounds, repetitions in ((True, 3, 1), (False, 20, 7)):
        for round_index in range(rounds):
            for repetition in range(repetitions):
                for layout_offset in range(4):
                    layout = (round_index + repetition + layout_offset) % 4
                    for channel_offset in range(3):
                        channel = (round_index + repetition + channel_offset) % 3
                        yield warmup, round_index, repetition, layout, channel


def upper_median(values):
    return sorted(values)[len(values) // 2]


def analyze_rows(rows):
    expected = list(schedule())
    if len(rows) != len(expected):
        raise ValueError("missing or extra raw comparison rows")
    input_digest, output_digest = expected_digests()
    cpu = rows[0].get("cpuBefore")
    pmu_available = 0
    for sequence, (row, order) in enumerate(zip(rows, expected)):
        if row.get("type") != "batch" or row.get("sequence") != sequence or (
                row.get("warmup"), row.get("round"), row.get("repetition"),
                row.get("layout"), row.get("channel")) != order:
            raise ValueError("raw comparison order changed")
        if row.get("calls") != 5000 or row.get("elements") != 20000000:
            raise ValueError("raw comparison work changed")
        if row.get("inputDigest") != input_digest or row.get("outputDigest") != output_digest:
            raise ValueError("raw comparison input/output digest changed")
        if type(cpu) is not int or cpu < 0 or row.get("cpuBefore") != cpu or row.get("cpuAfter") != cpu:
            raise ValueError("raw comparison CPU affinity changed")
        if any(type(row.get(field)) is not int or row[field] <= 0
               for field in ("threadCpuNs", "wallNs")):
            raise ValueError("invalid raw comparison duration")
        pmu = row.get("pmu")
        if pmu is not None:
            if any(type(pmu.get(field)) is not int or pmu[field] < 0
                   for field in ("cycles", "instructions", "branchMisses", "enabled", "running")):
                raise ValueError("invalid raw PMU counter")
            if not (pmu["enabled"] == pmu["running"] > 0 and pmu["cycles"] > 0 and pmu["instructions"] > 0):
                raise ValueError("unavailable or multiplexed PMU must not become valid counters")
            pmu_available += 1
    layouts = []
    for layout in range(4):
        channels = []
        for channel in range(3):
            selected = [row for row in rows if not row["warmup"]
                        and row["layout"] == layout and row["channel"] == channel]
            medians = [upper_median([row["threadCpuNs"] for row in selected if row["round"] == r])
                       for r in range(20)]
            cycles = None
            if all(row["pmu"] is not None for row in selected):
                cycles = upper_median([upper_median([row["pmu"]["cycles"] for row in selected
                                                    if row["round"] == r]) for r in range(20)])
            channels.append({"channel": CHANNELS[channel], "roundMediansNs": medians,
                             "threadCpuMedianNs": upper_median(medians), "cyclesMedian": cycles})
        layouts.append({"layout": layout, "channels": channels,
                        "throughputRatio": min(row["threadCpuMedianNs"] for row in channels[1:])
                        / channels[0]["threadCpuMedianNs"]})
    return {"status": "collected", "acceptance": False, "rawRows": len(rows),
            "pmuAvailableRows": pmu_available, "layouts": layouts,
            "limitations": "Separate same-process code-placement intervention, not the original gate. "
            "New fixed data buffers; historical mappings are unknown. Wall/resources/PMU bracket clock "
            "boundary work. Missing counters are not zero; no root cause is inferred automatically."}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--self-test", action="store_true", help="untimed semantic checks only")
    arguments = parser.parse_args()
    out = arguments.out
    out.mkdir(parents=True, exist_ok=False)
    summary = {"acceptance": False, "host": list(platform.uname())}
    if platform.system() != "Linux" or platform.machine() != "aarch64":
        summary.update(status="unavailable", reason="requires native Linux/AArch64")
    elif not arguments.report.exists():
        summary.update(status="unavailable", reason="original schema-7 report was not produced")
    else:
        before_report = file_identity(arguments.report)
        libraries = resolve_libraries(arguments.report)
        for name, path in (("cpuinfo.txt", Path("/proc/cpuinfo")),
                           ("perf-event-paranoid.txt", Path("/proc/sys/kernel/perf_event_paranoid"))):
            try:
                (out / name).write_text(path.read_text())
            except OSError as error:
                (out / name).write_text(f"unavailable: {error}\n")
        source = Path(__file__).resolve().parents[1] / "tests/support/checked_layout_diagnostic.c"
        compiler = compiler_path(os.environ)
        binary = out / "checked-layout"
        build_command = [compiler, "-std=c11", "-O3", "-Wall", "-Wextra", "-Werror",
                         str(source), "-ldl", "-o", str(binary)]
        with (out / "build.log").open("x") as log:
            subprocess.run([compiler, "--version"], stdout=log, stderr=subprocess.STDOUT,
                           check=True, timeout=10)
            subprocess.run(build_command, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=60)
        command = [str(binary.resolve()), "--self-test" if arguments.self_test else "--measure",
                   *[record["path"] for record in libraries]]
        identity = {"report": {"path": str(arguments.report.resolve()), **before_report},
                    "libraries": libraries, "source": file_identity(source),
                    "binary": file_identity(binary), "buildCommand": build_command, "command": command}
        (out / "identity-before.json").write_text(json.dumps(identity, indent=2) + "\n")
        with (out / "raw.jsonl").open("x") as log, (out / "stderr.log").open("x") as errors:
            result = subprocess.run(command, stdout=log, stderr=errors, timeout=240)
        if file_identity(arguments.report) != before_report or resolve_libraries(arguments.report) != libraries:
            raise ValueError("original report/library identity changed during comparison")
        identity["returncode"] = result.returncode
        (out / "identity.json").write_text(json.dumps(identity, indent=2) + "\n")
        if result.returncode == 77:
            summary.update(status="unavailable", reason="instruction bodies differ from the bounded known comparison")
        elif result.returncode != 0:
            raise ValueError(f"checked layout comparison failed integrity/semantics: {result.returncode}")
        else:
            records = [json.loads(line, object_pairs_hook=strict_object)
                       for line in (out / "raw.jsonl").read_text().splitlines()]
            if not records or records[0].get("type") != "identity" or not records[0].get("copiesVerified"):
                raise ValueError("missing code-copy and semantic identity")
            if arguments.self_test:
                if len(records) != 1:
                    raise ValueError("untimed self-test unexpectedly produced timing rows")
                summary.update(status="semantic_checks_only", runtime=records[0])
            else:
                summary.update(analyze_rows(records[1:]), runtime=records[0])
    (out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
