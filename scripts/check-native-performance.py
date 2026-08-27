#!/usr/bin/env python3
"""Validate the strict CK native-versus-Clang O3 performance report."""

from __future__ import annotations

import json
import math
import pathlib
import statistics
import sys


def fail(message: str) -> None:
    raise ValueError(message)


def positive_number(value: object, field: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)) or value <= 0:
        fail(f"{field} must be a positive number")
    return float(value)


def stable_samples(value: object, field: str) -> list[float]:
    if not isinstance(value, list) or len(value) < 3:
        fail(f"{field} must contain at least three samples")
    samples = [positive_number(sample, field) for sample in value]
    median = statistics.median(samples)
    stable = sum(median * 0.75 <= sample <= median * 1.25 for sample in samples)
    if stable / len(samples) < 0.80:
        fail(f"{field} is unstable around its median")
    return samples


def check_case(case: object, mode: str) -> tuple[str, float, float]:
    if not isinstance(case, dict) or not isinstance(case.get("name"), str):
        fail(f"{mode} suite contains a malformed case")
    name = case["name"]
    if case.get("referenceEquivalent") is not True:
        fail(f"{mode}/{name} did not prove reference equivalence")
    native = positive_number(case.get("nativeMedianNs"), f"{mode}/{name} nativeMedianNs")
    clang = positive_number(case.get("clangCMedianNs"), f"{mode}/{name} clangCMedianNs")
    stable_samples(case.get("nativeSamplesNs"), f"{mode}/{name} nativeSamplesNs")
    stable_samples(case.get("clangCSamplesNs"), f"{mode}/{name} clangCSamplesNs")
    for field in (
        "nativeCompileNs",
        "clangCCompileNs",
        "nativeColdNs",
        "clangCColdNs",
        "peakMemoryBytes",
        "nativeArtifactBytes",
        "clangCArtifactBytes",
        "batchIterations",
    ):
        positive_number(case.get(field), f"{mode}/{name} {field}")
    ratio = clang / native
    if native / clang > 1.10:
        fail(f"{mode}/{name} is more than 10% slower than strict Clang C O3")
    return name, ratio, native / clang


def check(path: pathlib.Path) -> None:
    report = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(report, dict) or report.get("schemaVersion") != 2:
        fail("performance report schemaVersion must be 2")
    if report.get("fastMath") is not False:
        fail("fast-math references are forbidden")
    if report.get("cpuPolicy") not in ("baseline", "native"):
        fail("cpuPolicy must be baseline or native")
    if not isinstance(report.get("warmup"), int) or report["warmup"] <= 0:
        fail("warmup must be a positive integer")
    if not isinstance(report.get("sampleRepetitions"), int) or report["sampleRepetitions"] < 3:
        fail("sampleRepetitions must be at least 3")

    suites = report.get("suites")
    if not isinstance(suites, list):
        fail("suites must be an array")
    modes: dict[str, list[object]] = {}
    for suite in suites:
        if not isinstance(suite, dict) or suite.get("mode") not in ("checked", "unchecked"):
            fail("every suite must have checked or unchecked mode")
        mode = suite["mode"]
        if mode in modes or not isinstance(suite.get("cases"), list) or not suite["cases"]:
            fail(f"duplicate or empty {mode} suite")
        modes[mode] = suite["cases"]
    if set(modes) != {"checked", "unchecked"}:
        fail("checked and unchecked suites must be reported separately")

    names_by_mode: dict[str, set[str]] = {}
    for mode, cases in modes.items():
        ratios = []
        names = set()
        for case in cases:
            name, ratio, _ = check_case(case, mode)
            if name in names:
                fail(f"duplicate {mode}/{name} case")
            names.add(name)
            ratios.append(ratio)
        names_by_mode[mode] = names
        geometric_mean = math.exp(sum(math.log(ratio) for ratio in ratios) / len(ratios))
        if geometric_mean < 0.95:
            fail(f"{mode} geometric-mean throughput is below 95% of strict Clang C O3")
        print(f"{mode}: geometric mean {geometric_mean:.4f}, {len(ratios)} case(s)")
    if names_by_mode["checked"] != names_by_mode["unchecked"]:
        fail("checked and unchecked suites must cover identical kernels")


def main() -> int:
    if len(sys.argv) != 2:
        print(f"usage: {pathlib.Path(sys.argv[0]).name} <results.json>", file=sys.stderr)
        return 2
    try:
        check(pathlib.Path(sys.argv[1]))
    except (OSError, json.JSONDecodeError, ValueError) as error:
        print(f"native performance gate failed: {error}", file=sys.stderr)
        return 1
    print("native performance gate passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
