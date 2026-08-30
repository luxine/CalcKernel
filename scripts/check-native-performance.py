#!/usr/bin/env python3
"""Validate CK 0.11 strict runtime, baseline, proof-loop, and optimizer gates."""

from __future__ import annotations

import json
import hashlib
import math
import os
import pathlib
import platform
import re
import stat
import statistics
import sys
import tomllib

BASELINE_COMMIT = "df816502876fba41676f9ebc190e4fadd18cd5a5"
BASELINE_COMPILER_IDENTITY = f"calckernel 0.10.0 ({BASELINE_COMMIT})"
BASELINE_LLVM_VERSION = "22.1.8"
BASELINE_HARNESS = (
    "ckc_perf schema 2 + proof-loop ABI adapter "
    "sha256=316b64bf3e24ade271d870444bb66a85018c4dcb66229afce202da2d2b53af6e + "
    "MIR optimizer timer "
    "sha256=828138f376472b177d8bbd1aa4f7888ed323ec03d098e21a74abcfce32a98d0b + "
    "Linux C++ runtime link adapter "
    "sha256=099305e8a9d5ff8d54e574b0fbd202a511f28a8543508f8c0ea06001704cdaff + "
    "Clang CPU policy adapter "
    "sha256=f22d58f4e2712e792a5b933376fe3a81fa1bd44a4cdb39b2790359ab5a40c7f1; "
    "warmup=3; samples=20; repetitions=7; batch=20000000"
)
BASELINE_STATISTICS = (
    "minimum-of-7 call samples; upper-median-of-20; strict-fp; pinned clang 22.1.8"
)
SAMPLE_COUNT = 20
BASELINE_MANIFEST_SHA256 = "27c0b995ba51cd799c2bcb89e1df0a4d40538fbf3200e1197f06ecab2ebad4f3"
REPO = pathlib.Path(__file__).resolve().parents[1]
RECIPE_FILES = ["scripts/prepare-performance-replay.py", "benches/runtime_replay.rs", "benches/ckc_perf.rs"]
ADAPTER_FILES = {
    "benches/baselines/v0_10_linux_cpp_runtime_harness.patch": "099305e8a9d5ff8d54e574b0fbd202a511f28a8543508f8c0ea06001704cdaff",
    "benches/baselines/v0_10_clang_cpu_harness.patch": "f22d58f4e2712e792a5b933376fe3a81fa1bd44a4cdb39b2790359ab5a40c7f1",
    "benches/baselines/v0_10_mir_optimizer_harness.patch": "828138f376472b177d8bbd1aa4f7888ed323ec03d098e21a74abcfce32a98d0b",
    "benches/baselines/v0_10_proof_loop_harness.patch": "316b64bf3e24ade271d870444bb66a85018c4dcb66229afce202da2d2b53af6e",
}
CHANNEL_NAMES = [f"{kind}{mode}" for kind in
                 ["candidateNative", "currentClang", "replayNative", "replayClang"]
                 for mode in ["Unchecked", "Checked"]]
BASELINE_SOURCE_DIGESTS = {
    "branch_mix": "d4f80ba571422feffe4d568bd476b44dde2a3f9086d30ebd77972dcf4254d7b8",
    "integer_accumulate": "4734807a96981f42e85b68ba4b964ce21e354c3486e7f668d89dcaefa391fc39",
    "proof_loop": "ea8c9f1be3e5fffa8c1c0e5e448d6617be15d855fdef2ee49670c4f98b88e30d",
    "remainder_chain": "87a36a9f5cd951c7281480bd180a9d8a657fd85e553f5a93edb2d5e74c00311e",
    "pricing": "be74bd3851e54db09955255b463025a6ee8464620ae1753c88b7d6d453388416",
    "pricing_soa": "5c003b70649f34516a2830584542086ce52ff0adfdd6dd0d76010a33e1d23cad",
    "f64_kernels": "58e10d6c28c5d95088a2e156197eb51c880b361a555d016ae11e9e0b7ecad7be",
    "example_pricing": "aebfe8bc5de317e32a7c945c7424a75b32a4330d7fd6dd53bb2d0c01cfbcb65a",
    "example_dijkstra": "490a7a3a3a04abb9cb9f05c9dbeea60d61690fc32897f36916f1ffa3c28a2f96",
    "v0_10_c_branch_mix_checked": "fb5b95130998c20a0014b01af5659720771d836614c5bd0aa85e5c02d68921e2",
    "v0_10_c_branch_mix_unchecked": "523e5f4af4c4bb64e6949dd7bfcd15578adb8ff47aa4437b5e1d01e6df84512b",
    "v0_10_c_integer_accumulate_checked": "91b9abc17ff50d7d55733ba0972f268779e8f2ea07ed96683dfa376a57113952",
    "v0_10_c_integer_accumulate_unchecked": "82b09a2e7428d99190cc50b03c709e5b018b082d0c265564bb4618e547fadf8a",
    "v0_10_c_proof_loop_checked": "044bc8d4b456a64d9cb6f3af057466796466b8cf32628fa4cb5e78b0e57bfee8",
    "v0_10_c_proof_loop_unchecked": "fed666f2048f254401e8554f8447b874cd4f602c1996f16825ea01d55e968326",
    "v0_10_c_remainder_chain_checked": "1dc89902f0e636a2c0a8f63a644a734ffcbbedb0b3039e299bc0c8b6ac439eda",
    "v0_10_c_remainder_chain_unchecked": "855c5bcb9bf82a8b06aab295c05211663a97a505654613a7b5dae33d2a6e9aeb",
}
RUNTIME_CASE_NAMES = {
    "branch_mix",
    "integer_accumulate",
    "proof_loop",
    "remainder_chain",
}
OPTIMIZER_CASE_NAMES = {
    "pricing",
    "pricing-soa",
    "f64-kernels",
    "proof",
    "example-pricing",
    "example-dijkstra",
}
DEFAULT_BASELINE_MANIFEST = (
    pathlib.Path(__file__).resolve().parents[1]
    / "benches"
    / "baselines"
    / "v0_10_compiler.toml"
)


def fail(message: str) -> None:
    raise ValueError(message)


def positive_number(value: object, field: str) -> float:
    if (isinstance(value, bool) or not isinstance(value, (int, float))
            or value <= 0 or not math.isfinite(value)):
        fail(f"{field} must be a finite positive number")
    return float(value)


def file_digest(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(65536), b""):
            digest.update(chunk)
    return digest.hexdigest()


def named_digest(paths: list[str]) -> str:
    digest = hashlib.sha256()
    for name in sorted(paths):
        digest.update(name.encode() + b"\0" + file_digest(REPO / name).encode() + b"\n")
    return digest.hexdigest()


def hash_value(value: object, label: str) -> str:
    if not isinstance(value, str) or re.fullmatch(r"[0-9a-f]{64}", value) is None:
        fail(f"{label} must be a lowercase SHA-256 hash")
    return value


def verify_file(path: pathlib.Path, size: object, digest: object, label: str) -> None:
    hash_value(digest, label)
    metadata = path.lstat()
    if (type(size) is not int or size <= 0 or not stat.S_ISREG(metadata.st_mode)
            or metadata.st_size != size or file_digest(path) != digest):
        fail(f"{label} file size or SHA-256 mismatch: {path}")


def strict_json_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result = {}
    for key, value in pairs:
        if key in result:
            fail(f"duplicate JSON field {key}")
        result[key] = value
    return result


def expected_order(count: int) -> list[list[int]]:
    return [[(round_index + offset) % 8 for offset in range(8)] for round_index in range(count)]


def check_order(value: object, count: int, field: str) -> None:
    if (not isinstance(value, list) or len(value) != count
            or any(not isinstance(row, list) or len(row) != 8
                   or any(type(channel) is not int for channel in row) for row in value)
            or value != expected_order(count)):
        fail(f"{field} must record the exact rotating eight-channel order")


def check_replay(report: dict[str, object], report_path: pathlib.Path) -> None:
    replay = report.get("runtimeReplay")
    if not isinstance(replay, dict) or set(replay) != {"metadata", "manifestSha256", "artifacts"}:
        fail("runtime replay must include exact metadata, manifest and artifact evidence")
    bundle_value = os.environ.get("CKC_V010_RUNTIME_BUNDLE")
    if not bundle_value:
        fail("set CKC_V010_RUNTIME_BUNDLE after scripts/prepare-performance-replay.py")
    bundle = pathlib.Path(bundle_value).resolve(strict=True)
    manifest_path = bundle / "replay.tsv"
    manifest_digest = hash_value(replay.get("manifestSha256"), "replay manifest")
    if (not stat.S_ISREG(manifest_path.lstat().st_mode)
            or file_digest(manifest_path) != manifest_digest):
        fail("replay manifest SHA-256 mismatch")
    lines = manifest_path.read_text(encoding="utf-8").splitlines()
    if not lines or lines[0] != "ckc-v010-runtime-replay\t1":
        fail("unsupported replay bundle schema")
    fields = {"commit", "compilerIdentity", "compilerSha256", "compilerBytes", "llvmVersion",
              "target", "cpuPolicy", "recipeSha256", "adapterSetSha256", "sourceDiffSha256",
              "baselineManifestSha256", "llvmComponentSha256"}
    metadata = {}
    artifacts = {}
    target = host_target_name()
    suffix = {"linux": ".so", "macos": ".dylib", "windows": ".dll"}[target.split("-")[0]]
    for line in lines[1:]:
        parts = line.split("\t")
        if len(parts) == 2 and parts[0] in fields and parts[1]:
            key, value = parts
            if key in metadata:
                fail(f"duplicate replay metadata {key}")
            metadata[key] = value
        elif len(parts) == 6 and parts[0] == "artifact":
            _, mode, case, filename, size, digest = parts
            if (mode not in {"unchecked", "checked"} or case not in RUNTIME_CASE_NAMES
                    or filename != f"{case}-{mode}{suffix}" or (mode, case) in artifacts
                    or re.fullmatch(r"[0-9]+", size) is None or not 0 < int(size) < 2**64):
                fail("invalid or duplicate replay artifact record")
            verify_file(bundle / filename, int(size), digest, "replay artifact")
            artifacts[mode, case] = dict(mode=mode, case=case, file=filename, bytes=int(size), sha256=digest)
        else:
            fail("unknown or malformed replay manifest record")
    if set(metadata) != fields or len(artifacts) != 8 or replay.get("metadata") != metadata:
        fail("replay metadata/artifact set does not match the exact bundle")
    prefix_value = os.environ.get("CKC_LLVM_PREFIX")
    if not prefix_value:
        fail("CKC_LLVM_PREFIX is required to verify the replay LLVM component identity")
    for name, digest in ADAPTER_FILES.items():
        if file_digest(REPO / name) != digest:
            fail(f"pinned replay adapter changed: {name}")
    expected = dict(commit=BASELINE_COMMIT, compilerIdentity=BASELINE_COMPILER_IDENTITY,
                    llvmVersion=BASELINE_LLVM_VERSION, target=target, cpuPolicy="baseline",
                    recipeSha256=named_digest(RECIPE_FILES), adapterSetSha256=named_digest(list(ADAPTER_FILES)),
                    baselineManifestSha256=BASELINE_MANIFEST_SHA256,
                    llvmComponentSha256=file_digest(pathlib.Path(prefix_value) / "share/ckc/llvm-build.toml"))
    for key, value in expected.items():
        if metadata[key] != value:
            fail(f"replay {key} does not match the pinned identity")
    for field in fields:
        if field.endswith("Sha256"):
            hash_value(metadata[field], f"replay {field}")
    compiler_size = metadata["compilerBytes"]
    if re.fullmatch(r"[0-9]+", compiler_size) is None or not 0 < int(compiler_size) < 2**64:
        fail("replay compiler size must be a positive u64")
    verify_file(bundle / "ckc-v010", int(compiler_size), metadata["compilerSha256"], "replay compiler")
    reported = replay.get("artifacts")
    if not isinstance(reported, list) or len(reported) != 8:
        fail("replay must report exactly eight artifact records")
    seen = set()
    for item in reported:
        if not isinstance(item, dict) or not isinstance(item.get("mode"), str) or not isinstance(item.get("case"), str):
            fail("malformed replay artifact")
        key = item["mode"], item["case"]
        if key in seen or artifacts.get(key) != item or type(item.get("bytes")) is not int:
            fail("replay artifact record mismatch or duplicate")
        seen.add(key)
    directory = report.get("evidenceDirectory")
    if not isinstance(directory, str) or re.fullmatch(r"measurement-[0-9]+-[0-9]+", directory) is None:
        fail("invalid measured evidence directory")
    evidence_root = report_path.parent / directory
    if not stat.S_ISDIR(evidence_root.lstat().st_mode):
        fail("measured evidence directory must not be a symlink")
    measured = report.get("measuredArtifacts")
    if not isinstance(measured, list) or len(measured) != 24:
        fail("report must retain exactly twenty-four measured artifact records")
    seen = set()
    sizes = {}
    for item in measured:
        if not isinstance(item, dict) or set(item) != {"case", "mode", "channel", "file", "bytes", "sha256"}:
            fail("malformed measured artifact")
        case, mode, channel = (item[key] for key in ("case", "mode", "channel"))
        if not all(isinstance(value, str) for value in (case, mode, channel)):
            fail("invalid measured artifact identity")
        endings = {"candidateNative": "native", "currentClang": "clang", "replayClang": "replay-clang"}
        if case not in RUNTIME_CASE_NAMES or mode not in {"checked", "unchecked"} or channel not in endings:
            fail("unknown measured artifact identity")
        key = mode, case, channel
        filename = f"{case}-{mode}-{endings[channel]}{suffix}"
        if key in seen or item["file"] != filename:
            fail("duplicate or escaping measured artifact")
        seen.add(key)
        verify_file(evidence_root / filename, item["bytes"], item["sha256"], "measured artifact")
        sizes[key] = item["bytes"]
    for suite in report["suites"]:
        for case in suite["cases"]:
            for field, channel in [("nativeArtifactBytes", "candidateNative"), ("clangCArtifactBytes", "currentClang")]:
                if case[field] != sizes[suite["mode"], case["name"], channel]:
                    fail("runtime artifact size does not match measured evidence")


def stable_samples(value: object, field: str) -> list[float]:
    if not isinstance(value, list) or len(value) != SAMPLE_COUNT:
        fail(f"{field} must contain exactly {SAMPLE_COUNT} samples")
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


def upper_median(values: list[float]) -> float:
    ordered = sorted(values)
    return ordered[len(ordered) // 2]


def host_target_name() -> str:
    os_name = {"Darwin": "macos", "Linux": "linux", "Windows": "windows"}.get(
        platform.system()
    )
    arch_name = {
        "aarch64": "aarch64",
        "arm64": "aarch64",
        "amd64": "x86_64",
        "x86_64": "x86_64",
    }.get(platform.machine().lower())
    if os_name is None or arch_name is None:
        fail(
            f"unsupported performance host identity: {platform.system()}/"
            f"{platform.machine()}"
        )
    return f"{os_name}-{arch_name}"


def load_runtime_baseline(path: pathlib.Path) -> dict[tuple[str, str, str, str], tuple[int, int]]:
    if file_digest(path) != BASELINE_MANIFEST_SHA256:
        fail("frozen manifest SHA-256 does not match the accepted V0.10 baseline")
    with path.open("rb") as source:
        manifest = tomllib.load(source)
    if manifest.get("schema_version") != 2:
        fail("v0.10 baseline manifest schema_version must be 2")
    if manifest.get("commit") != BASELINE_COMMIT:
        fail("v0.10 baseline manifest commit does not match the pinned identity")
    if manifest.get("compiler_identity") != BASELINE_COMPILER_IDENTITY:
        fail("v0.10 baseline manifest compiler identity does not match")
    if manifest.get("llvm_version") != BASELINE_LLVM_VERSION:
        fail("v0.10 baseline manifest LLVM version does not match")
    entries = manifest.get("runtime")
    if not isinstance(entries, list) or not entries:
        fail("v0.10 baseline manifest must contain runtime entries")
    runtime: dict[tuple[str, str, str, str], tuple[int, int]] = {}
    for entry in entries:
        if not isinstance(entry, dict):
            fail("v0.10 baseline manifest contains a malformed runtime entry")
        identity = tuple(entry.get(field) for field in ("target", "cpu", "mode", "case"))
        if not all(isinstance(value, str) for value in identity):
            fail("v0.10 baseline runtime identity must contain strings")
        key = (identity[0], identity[1], identity[2], identity[3])
        if key in runtime:
            fail(f"duplicate v0.10 baseline runtime entry {key}")
        native = entry.get("median_ns")
        clang = entry.get("clang_median_ns")
        if type(native) is not int or native <= 0 or type(clang) is not int or clang <= 0:
            fail(f"v0.10 baseline runtime entry {key} must contain positive integer medians")
        runtime[key] = (native, clang)
    return runtime


def check_baseline_identity(report: dict[str, object]) -> None:
    baseline = report.get("baselineV010")
    if not isinstance(baseline, dict) or baseline.get("commit") != BASELINE_COMMIT:
        fail("baselineV010 must name the pinned v0.10 commit")
    expected = {
        "compilerIdentity": BASELINE_COMPILER_IDENTITY,
        "llvmVersion": BASELINE_LLVM_VERSION,
        "target": host_target_name(),
        "harness": BASELINE_HARNESS,
        "statistics": BASELINE_STATISTICS,
    }
    for field, value in expected.items():
        if baseline.get(field) != value:
            fail(f"baselineV010 {field} does not match the pinned identity")
    if baseline.get("sourceDigestCount") != len(BASELINE_SOURCE_DIGESTS):
        fail("baselineV010 must cover every pinned source digest")
    digests = baseline.get("sourceDigests")
    if digests != BASELINE_SOURCE_DIGESTS:
        fail("baselineV010 sourceDigests do not match the exact pinned corpus")


def check_case(
    case: object,
    mode: str,
    expected_baseline: tuple[int, int],
) -> tuple[str, float, float, bool, float]:
    if not isinstance(case, dict) or not isinstance(case.get("name"), str):
        fail(f"{mode} suite contains a malformed case")
    name = case["name"]
    if case.get("referenceEquivalent") is not True:
        fail(f"{mode}/{name} did not prove reference equivalence")
    native = positive_number(case.get("nativeMedianNs"), f"{mode}/{name} nativeMedianNs")
    clang = positive_number(case.get("clangCMedianNs"), f"{mode}/{name} clangCMedianNs")
    positive_number(case.get("v010MedianNs"), f"{mode}/{name} v010MedianNs")
    positive_number(
        case.get("v010ClangMedianNs"), f"{mode}/{name} v010ClangMedianNs"
    )
    if case.get("v010MedianNs") != expected_baseline[0]:
        fail(f"{mode}/{name} v010MedianNs does not match the frozen manifest")
    if case.get("v010ClangMedianNs") != expected_baseline[1]:
        fail(f"{mode}/{name} v010ClangMedianNs does not match the frozen manifest")
    native_samples = stable_samples(
        case.get("nativeSamplesNs"), f"{mode}/{name} nativeSamplesNs"
    )
    clang_samples = stable_samples(
        case.get("clangCSamplesNs"), f"{mode}/{name} clangCSamplesNs"
    )
    if native != upper_median(native_samples):
        fail(f"{mode}/{name} nativeMedianNs does not match its sample array")
    if clang != upper_median(clang_samples):
        fail(f"{mode}/{name} clangCMedianNs does not match its sample array")
    replay_medians = []
    for stream in ("replayNative", "replayClang"):
        replay_median = positive_number(case.get(stream + "MedianNs"), f"{mode}/{name} {stream}MedianNs")
        replay_samples = stable_samples(case.get(stream + "SamplesNs"), f"{mode}/{name} {stream}SamplesNs")
        if replay_median != upper_median(replay_samples):
            fail(f"{mode}/{name} {stream}MedianNs does not match its sample array")
        replay_medians.append(replay_median)
    check_order(case.get("warmupOrder"), 3, f"{mode}/{name} warmup order")
    check_order(case.get("sampleOrder"), 20, f"{mode}/{name} sample order")
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
    if type(case.get("batchIterations")) is not int or case["batchIterations"] != 20_000_000:
        fail(f"{mode}/{name} batchIterations must be exactly 20000000")
    if type(case.get("result")) is not int or not -(2**63) <= case["result"] < 2**63:
        fail(f"{mode}/{name} must report a validated i64 result")
    if native / clang > 1.10:
        fail(f"{mode}/{name} is more than 10% slower than strict Clang C O3")
    normalized_baseline_ratio = (native / clang) / (replay_medians[0] / replay_medians[1])
    if normalized_baseline_ratio > 1.08:
        fail(f"{mode}/{name} regressed more than 8% from pinned v0.10")
    proof_loop = case.get("proofLoop") is True
    return name, clang / native, normalized_baseline_ratio, proof_loop, native


def check_optimizer(report: dict[str, object], baseline_manifest: pathlib.Path) -> None:
    baseline = tomllib.loads(baseline_manifest.read_text(encoding="utf-8"))
    expected = {row["case"]: row["median_ns"] for row in baseline["optimizer"]
                if row["target"] == host_target_name()}
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
        if mir != expected.get(name):
            fail(f"optimizer/{name} v010MirMedianNs does not match the frozen manifest")
        ratio = kir / mir
        if ratio > 3.0:
            fail(f"optimizer/{name} exceeds the 3x individual v0.10 MIR limit")
        ratios.append(ratio)
    if names != OPTIMIZER_CASE_NAMES:
        fail("optimizerComparisons must cover the exact frozen optimizer corpus")
    suite_median = statistics.median(ratios)
    if suite_median > 2.0:
        fail("KIR optimizer suite-median time exceeds 2x pinned v0.10 MIR")
    print(
        f"optimizer: v0.10 suite-median ratio {suite_median:.4f}, "
        f"{len(ratios)} case(s)"
    )


def check(path: pathlib.Path, baseline_manifest: pathlib.Path) -> None:
    report = json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=strict_json_object)
    baseline_runtime = load_runtime_baseline(baseline_manifest)
    if not isinstance(report, dict) or report.get("schemaVersion") != 6:
        fail("performance report schemaVersion must be 6")
    if report.get("candidateVersion") != "0.11.0":
        fail("candidateVersion must identify the 0.11.0 candidate")
    if report.get("samplingProtocol") != "rotating-eight-channel-v1":
        fail("sampling protocol must be rotating-eight-channel-v1")
    if report.get("channelNames") != CHANNEL_NAMES:
        fail("sampling channel names/order do not match the pinned protocol")
    if report.get("fastMath") is not False:
        fail("fast-math references are forbidden")
    if report.get("cpuPolicy") != "baseline":
        fail("the release performance gate requires the portable baseline CPU policy")
    if report.get("clangVersion") != BASELINE_LLVM_VERSION:
        fail("clangVersion must match the pinned Clang 22.1.8 oracle")
    if type(report.get("warmup")) is not int or report.get("warmup") != 3:
        fail("warmup must match the pinned value 3")
    if (
        type(report.get("sampleRepetitions")) is not int
        or report.get("sampleRepetitions") != 7
    ):
        fail("sampleRepetitions must match the pinned value 7")
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
            if not isinstance(case, dict) or not isinstance(case.get("name"), str):
                fail(f"{mode} suite contains a malformed case")
            baseline_key = (host_target_name(), "baseline", mode, case["name"])
            expected_baseline = baseline_runtime.get(baseline_key)
            if expected_baseline is None:
                fail(f"v0.10 baseline manifest is missing runtime {baseline_key}")
            name, clang_ratio, baseline_ratio, proof_loop, native = check_case(
                case, mode, expected_baseline
            )
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
    if any(names != RUNTIME_CASE_NAMES for names in names_by_mode.values()):
        fail("checked and unchecked suites must cover the exact frozen runtime corpus")
    results = [{case["name"]: case["result"] for case in cases} for cases in modes.values()]
    if results[0] != results[1]:
        fail("checked and unchecked results must match on the same inputs")
    expected_proof_loops = {"proof_loop"}
    if any(set(times) != expected_proof_loops for times in proof_times.values()):
        fail("checked and unchecked suites must identify the exact proof-loop corpus")
    proof_ratios = [
        proof_times["unchecked"][name] / checked
        for name, checked in proof_times["checked"].items()
    ]
    proof_mean = geometric_mean(proof_ratios, "proof-loop ratios")
    if proof_mean < 0.97:
        fail("checked proof-loop throughput is below 97% of unchecked")
    print(f"proof-loop: checked/unchecked throughput {proof_mean:.4f}")
    check_optimizer(report, baseline_manifest)
    check_replay(report, path)


def main() -> int:
    if len(sys.argv) not in (2, 3):
        print(
            f"usage: {pathlib.Path(sys.argv[0]).name} <results.json> "
            "[baseline.toml]",
            file=sys.stderr,
        )
        return 2
    baseline_manifest = (
        pathlib.Path(sys.argv[2]) if len(sys.argv) == 3 else DEFAULT_BASELINE_MANIFEST
    )
    try:
        check(pathlib.Path(sys.argv[1]), baseline_manifest)
    except (OSError, json.JSONDecodeError, tomllib.TOMLDecodeError, ValueError) as error:
        print(f"native performance gate failed: {error}", file=sys.stderr)
        return 1
    print("native performance gate passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
