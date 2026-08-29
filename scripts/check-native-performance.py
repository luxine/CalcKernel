#!/usr/bin/env python3
"""Validate CK 0.11 strict runtime, baseline, proof-loop, and optimizer gates."""

from __future__ import annotations

import json
import math
import pathlib
import statistics
import sys

BASELINE_COMMIT = "df816502876fba41676f9ebc190e4fadd18cd5a5"
BASELINE_DIGEST_NAMES = {
    "branch_mix",
    "integer_accumulate",
    "proof_loop",
    "remainder_chain",
    "pricing",
    "pricing_soa",
    "f64_kernels",
    "example_pricing",
    "example_dijkstra",
}


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


def geometric_mean(values: list[float], field: str) -> float:
    if not values:
        fail(f"{field} must contain at least one value")
    return math.exp(sum(math.log(value) for value in values) / len(values))


def check_baseline_identity(report: dict[str, object]) -> None:
    baseline = report.get("baselineV010")
    if not isinstance(baseline, dict) or baseline.get("commit") != BASELINE_COMMIT:
        fail("baselineV010 must name the pinned v0.10 commit")
    for field in ("compilerIdentity", "llvmVersion", "target", "harness", "statistics"):
        if not isinstance(baseline.get(field), str) or not baseline[field]:
            fail(f"baselineV010 {field} must be non-empty")
    if baseline.get("sourceDigestCount") != len(BASELINE_DIGEST_NAMES):
        fail("baselineV010 must cover every pinned source digest")
    digests = baseline.get("sourceDigests")
    if not isinstance(digests, dict) or set(digests) != BASELINE_DIGEST_NAMES:
        fail("baselineV010 sourceDigests must name the exact pinned corpus")
    for name, digest in digests.items():
        if (
            not isinstance(digest, str)
            or len(digest) != 64
            or any(character not in "0123456789abcdef" for character in digest)
        ):
            fail(f"baselineV010 source digest {name} must be lowercase SHA-256")


def check_case(case: object, mode: str) -> tuple[str, float, float, bool, float]:
    if not isinstance(case, dict) or not isinstance(case.get("name"), str):
        fail(f"{mode} suite contains a malformed case")
    name = case["name"]
    if case.get("referenceEquivalent") is not True:
        fail(f"{mode}/{name} did not prove reference equivalence")
    native = positive_number(case.get("nativeMedianNs"), f"{mode}/{name} nativeMedianNs")
    clang = positive_number(case.get("clangCMedianNs"), f"{mode}/{name} clangCMedianNs")
    baseline = positive_number(case.get("v010MedianNs"), f"{mode}/{name} v010MedianNs")
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
    if native / clang > 1.10:
        fail(f"{mode}/{name} is more than 10% slower than strict Clang C O3")
    if native / baseline > 1.08:
        fail(f"{mode}/{name} regressed more than 8% from pinned v0.10")
    proof_loop = case.get("proofLoop") is True
    return name, clang / native, native / baseline, proof_loop, native


def check_optimizer(report: dict[str, object]) -> None:
    comparisons = report.get("optimizerComparisons")
    if not isinstance(comparisons, list) or not comparisons:
        fail("optimizerComparisons must be a non-empty array")
    ratios = []
    names = set()
    for comparison in comparisons:
        if not isinstance(comparison, dict) or not isinstance(comparison.get("case"), str):
            fail("optimizerComparisons contains a malformed case")
        name = comparison["case"]
        if name in names:
            fail(f"duplicate optimizer comparison {name}")
        names.add(name)
        kir = positive_number(comparison.get("kirMedianNs"), f"optimizer/{name} kirMedianNs")
        mir = positive_number(
            comparison.get("v010MirMedianNs"), f"optimizer/{name} v010MirMedianNs"
        )
        ratio = kir / mir
        if ratio > 3.0:
            fail(f"optimizer/{name} exceeds the 3x individual v0.10 MIR limit")
        ratios.append(ratio)
    suite_median = statistics.median(ratios)
    if suite_median > 2.0:
        fail("KIR optimizer suite-median time exceeds 2x pinned v0.10 MIR")
    print(
        f"optimizer: v0.10 suite-median ratio {suite_median:.4f}, "
        f"{len(ratios)} case(s)"
    )


def check(path: pathlib.Path) -> None:
    report = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(report, dict) or report.get("schemaVersion") != 4:
        fail("performance report schemaVersion must be 4")
    if report.get("fastMath") is not False:
        fail("fast-math references are forbidden")
    if report.get("cpuPolicy") not in ("baseline", "native"):
        fail("cpuPolicy must be baseline or native")
    if not isinstance(report.get("warmup"), int) or report["warmup"] <= 0:
        fail("warmup must be a positive integer")
    if not isinstance(report.get("sampleRepetitions"), int) or report["sampleRepetitions"] < 3:
        fail("sampleRepetitions must be at least 3")
    check_baseline_identity(report)

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
    proof_times: dict[str, dict[str, float]] = {"checked": {}, "unchecked": {}}
    for mode, cases in modes.items():
        clang_ratios = []
        baseline_ratios = []
        names = set()
        for case in cases:
            name, clang_ratio, baseline_ratio, proof_loop, native = check_case(case, mode)
            if name in names:
                fail(f"duplicate {mode}/{name} case")
            names.add(name)
            clang_ratios.append(clang_ratio)
            baseline_ratios.append(baseline_ratio)
            if proof_loop:
                proof_times[mode][name] = native
        names_by_mode[mode] = names
        clang_mean = geometric_mean(clang_ratios, f"{mode} Clang ratios")
        if clang_mean < 0.95:
            fail(f"{mode} geometric-mean throughput is below 95% of strict Clang C O3")
        baseline_mean = geometric_mean(baseline_ratios, f"{mode} baseline ratios")
        if baseline_mean > 1.03:
            fail(f"{mode} geometric-mean runtime regressed more than 3% from pinned v0.10")
        print(
            f"{mode}: Clang mean {clang_mean:.4f}, v0.10 ratio {baseline_mean:.4f}, "
            f"{len(cases)} case(s)"
        )
    if names_by_mode["checked"] != names_by_mode["unchecked"]:
        fail("checked and unchecked suites must cover identical kernels")
    if not proof_times["checked"] or proof_times["checked"].keys() != proof_times["unchecked"].keys():
        fail("checked and unchecked proof-loop corpus must be identical and non-empty")
    proof_ratios = [
        proof_times["unchecked"][name] / checked
        for name, checked in proof_times["checked"].items()
    ]
    proof_mean = geometric_mean(proof_ratios, "proof-loop ratios")
    if proof_mean < 0.97:
        fail("checked proof-loop throughput is below 97% of unchecked")
    print(f"proof-loop: checked/unchecked throughput {proof_mean:.4f}")
    check_optimizer(report)


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
