#!/usr/bin/env python3
"""Validate the fail-closed CK 0.13 schema-8 and cumulative schema-7 contracts."""

from __future__ import annotations

import hashlib
import json
import math
import os
import pathlib
import platform
import re
import stat
import statistics
import subprocess
import sys
import tomllib

REPO = pathlib.Path(__file__).resolve().parents[1]
V010_COMMIT = "df816502876fba41676f9ebc190e4fadd18cd5a5"
V010_COMPILER = f"calckernel 0.10.0 ({V010_COMMIT})"
V010_MANIFEST_SHA256 = "27c0b995ba51cd799c2bcb89e1df0a4d40538fbf3200e1197f06ecab2ebad4f3"
V011_COMMIT = "80c0acf6bb5d65e4d9d40352b9501ea32b79f43d"
V011_COMPILER = f"calckernel 0.11.0 ({V011_COMMIT})"
V011_MANIFEST_SHA256 = "495cde2e3a2afb847ddcad9707fec4e6880f26dc6c3085442290af7e2737421e"
LLVM_VERSION = "22.1.8"
RUST_VERSION = "1.90.0"
ORACLE_MANIFEST_SHA256 = "8bc9a23daf5f625b9855dd58c3e31e111ee58c8dc00dadc8af7be767c18f5f06"
DEFAULT_BASELINE_MANIFEST = REPO / "benches/baselines/v0_10_compiler.toml"
RECIPE_FILES = [
    "scripts/prepare-performance-replay.py",
    "scripts/audit-performance-oracles.py",
    "benches/runtime_replay.rs",
    "benches/ckc_perf.rs",
    "benches/vector_perf.rs",
    "benches/pgo_perf.rs",
    "benches/cases/pgo-cases.tsv",
    "scripts/measure-v013-performance.py",
    "benches/oracles/manifest.toml",
    "benches/oracles/pgo/manifest.toml",
]
V010_ADAPTERS = [
    "benches/baselines/v0_10_linux_cpp_runtime_harness.patch",
    "benches/baselines/v0_10_clang_cpu_harness.patch",
    "benches/baselines/v0_10_mir_optimizer_harness.patch",
    "benches/baselines/v0_10_proof_loop_harness.patch",
]
SCALAR_CASES = {"branch_mix", "integer_accumulate", "proof_loop", "remainder_chain"}
VECTOR_CASES = {
    "map_u32", "zip_u32", "strict_f64", "integer_cast", "modular_reduction",
    "slp_quad", "runtime_noalias", "specialized_length",
}
DOMAIN_CASES = {"contract_noalias", "contract_fixed_length"}
CHANNEL_NAMES = [
    f"{kind}{mode}"
    for kind in [
        "candidateNative", "currentClang", "replayV011Native",
        "replayV011Clang", "replayV010Native", "replayV010Clang",
    ]
    for mode in ["Unchecked", "Checked"]
]
SAMPLE_COUNT = 20

V012_COMMIT = "11ca3dbb1220710f184e3c32c873b267d24a22cb"
V012_COMPILER = f"calckernel 0.12.0 ({V012_COMMIT})"
V012_MANIFEST_SHA256 = "39ac2622bd827d902e827945b3394e804a93eaa9798619901170d2b0d2c5cb65"
PGO_CASES = {
    "branch-layout": (True, False),
    "call-constant-length": (True, True),
    "trip-unroll-simd": (True, True),
    "memory-bound": (False, True),
    "compute-bound": (True, True),
}
PGO_CHANNELS = [
    "ordinary", "replayV012", "pgo", "multiversion",
    "combined", "selectedDirect", "clangPgo", "rustPgo",
]
SCHEMA8_RECIPE_FILES = RECIPE_FILES + [
    "benches/baselines/v0_12_replay.toml",
    "benches/fixtures/pgo/training.tsv",
    "benches/fixtures/pgo/held-out.tsv",
    "benches/fixtures/pgo/adversarial.tsv",
    "benches/oracles/pgo/c/pgo_oracle.c",
    "benches/oracles/pgo/rust/pgo_oracle.rs",
]

# Normative thresholds; source-string tests guard their exact values.
SCHEMA8_THRESHOLDS = (
    "ordinaryGeoSlowdown=1.02;ordinaryIndividualSlowdown=1.05;"
    "pgoGeoImprovement=1.05;pgoIndividualSlowdown=1.03;"
    "dispatchGeoImprovement=1.08;dispatchDirectGeoThroughput=0.98;"
    "combinedGeoSlowdown=1.02;oracleGeoThroughput=0.95;"
    "generationOverhead=5.0;archiveGrowth=1.15"
)


def fail(message: str):
    raise ValueError(message)


def strict_json_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            fail(f"duplicate JSON key {key!r}")
        result[key] = value
    return result


def file_digest(path: pathlib.Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def named_digest(paths) -> str:
    digest = hashlib.sha256()
    for name in sorted(paths):
        digest.update(name.encode())
        digest.update(b"\0")
        digest.update(file_digest(REPO / name).encode())
        digest.update(b"\n")
    return digest.hexdigest()


def hash_value(value, field):
    if not isinstance(value, str) or re.fullmatch(r"[0-9a-f]{64}", value) is None:
        fail(f"{field} must be a lowercase SHA-256 digest")
    return value


def positive(value, field):
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        fail(f"{field} must be a finite positive number")
    if not math.isfinite(value) or value <= 0:
        fail(f"{field} must be a finite positive number")
    return float(value)


def stable_samples(value, field, count=SAMPLE_COUNT):
    if not isinstance(value, list) or len(value) != count:
        fail(f"{field} must contain exactly {count} samples")
    samples = [positive(sample, field) for sample in value]
    median = statistics.median(samples)
    if sum(median * .75 <= sample <= median * 1.25 for sample in samples) < math.ceil(.8 * count):
        fail(f"{field} is unstable around its median")
    return samples


def upper_median(values):
    ordered = sorted(values)
    return ordered[len(ordered) // 2]


def geometric_mean(values, field):
    if not values:
        fail(f"{field} has no values")
    return math.exp(sum(math.log(value) for value in values) / len(values))


def exact_keys(value, expected, field):
    if not isinstance(value, dict):
        fail(f"{field} must be an object")
    unknown = set(value) - set(expected)
    missing = set(expected) - set(value)
    if unknown or missing:
        fail(f"{field} has unknown or missing fields: unknown={sorted(unknown)}, missing={sorted(missing)}")


def check_order(value, width, rows, field):
    expected = [[(row + offset) % width for offset in range(width)] for row in range(rows)]
    if value != expected:
        fail(f"{field} does not match the exact rotating order")


def verify_file(path, size, digest, field):
    hash_value(digest, f"{field} digest")
    try:
        metadata = path.lstat()
    except OSError as error:
        fail(f"{field} is missing: {error}")
    if not stat.S_ISREG(metadata.st_mode) or metadata.st_size <= 0:
        fail(f"{field} must be a nonempty regular file, not a symlink")
    if type(size) is not int or size <= 0 or metadata.st_size != size:
        fail(f"{field} size mismatch")
    if file_digest(path) != digest:
        fail(f"{field} SHA-256 mismatch")


def host_target_name():
    system = {"Darwin": "macos", "Linux": "linux", "Windows": "windows"}.get(platform.system())
    arch = {"aarch64": "aarch64", "arm64": "aarch64", "amd64": "x86_64",
            "x86_64": "x86_64"}.get(platform.machine().lower())
    if system is None or arch is None:
        fail(f"unsupported performance host {platform.system()}/{platform.machine()}")
    return f"{system}-{arch}"


def dynamic_suffix(target):
    if target.startswith("windows-"):
        return ".dll"
    if target.startswith("macos-"):
        return ".dylib"
    return ".so"


def replay_expected(generation):
    if generation == "v011":
        return V011_COMMIT, V011_COMPILER, V011_MANIFEST_SHA256, "ckc-v011", hashlib.sha256(b"").hexdigest()
    return V010_COMMIT, V010_COMPILER, V010_MANIFEST_SHA256, "ckc-v010", named_digest(V010_ADAPTERS)


def check_replay(report, key, generation, report_path):
    env_name = f"CKC_{generation.upper()}_RUNTIME_BUNDLE"
    raw = os.environ.get(env_name)
    if not raw:
        fail(f"{env_name} is required")
    bundle = pathlib.Path(raw)
    commit, compiler_identity, manifest_digest, compiler_file, adapter_digest = replay_expected(generation)
    manifest_path = bundle / "replay.tsv"
    try:
        manifest_metadata = manifest_path.lstat()
    except OSError as error:
        fail(f"{generation} replay manifest is missing: {error}")
    if not stat.S_ISREG(manifest_metadata.st_mode):
        fail(f"{generation} replay manifest must not be a symlink")
    text = manifest_path.read_text(encoding="utf-8")
    lines = text.splitlines()
    if not lines or lines[0] != f"ckc-{generation}-runtime-replay\t1":
        fail(f"{generation} replay schema is unsupported")
    fields = {
        "commit", "compilerIdentity", "compilerSha256", "compilerBytes", "llvmVersion",
        "target", "cpuPolicy", "recipeSha256", "adapterSetSha256", "sourceDiffSha256",
        "baselineManifestSha256", "llvmComponentSha256",
    }
    metadata = {}
    artifacts = {}
    target = host_target_name()
    suffix = dynamic_suffix(target)
    for line in lines[1:]:
        parts = line.split("\t")
        if len(parts) == 2 and parts[0] in fields and parts[1]:
            if parts[0] in metadata:
                fail(f"duplicate {generation} replay metadata")
            metadata[parts[0]] = parts[1]
        elif len(parts) == 6 and parts[0] == "artifact":
            _, mode, case, filename, size, sha = parts
            identity = mode, case
            if (mode not in {"checked", "unchecked"} or case not in SCALAR_CASES
                    or filename != f"{case}-{mode}{suffix}" or identity in artifacts
                    or re.fullmatch(r"[0-9]+", size) is None):
                fail(f"invalid or duplicate {generation} replay artifact")
            item = dict(case=case, mode=mode, file=filename, bytes=int(size), sha256=sha)
            verify_file(bundle / filename, item["bytes"], sha, f"{generation} replay artifact")
            artifacts[identity] = item
        else:
            fail(f"unknown or malformed {generation} replay record")
    if set(metadata) != fields or len(artifacts) != 8:
        fail(f"{generation} replay must contain every identity and eight artifact records")
    prefix = os.environ.get("CKC_LLVM_PREFIX")
    if not prefix:
        fail("CKC_LLVM_PREFIX is required to verify replay identity")
    expected = {
        "commit": commit, "compilerIdentity": compiler_identity, "llvmVersion": LLVM_VERSION,
        "target": target, "cpuPolicy": "baseline", "recipeSha256": named_digest(RECIPE_FILES),
        "adapterSetSha256": adapter_digest, "baselineManifestSha256": manifest_digest,
        "llvmComponentSha256": file_digest(pathlib.Path(prefix) / "share/ckc/llvm-build.toml"),
    }
    for name, value in expected.items():
        if metadata.get(name) != value:
            fail(f"{generation} replay {name} does not match pinned identity")
    for name in fields:
        if name.endswith("Sha256"):
            hash_value(metadata[name], f"{generation} replay {name}")
    if re.fullmatch(r"[0-9]+", metadata["compilerBytes"]) is None:
        fail(f"{generation} replay compiler size is invalid")
    verify_file(bundle / compiler_file, int(metadata["compilerBytes"]), metadata["compilerSha256"],
                f"{generation} replay compiler")
    replay = report.get(key)
    exact_keys(replay, {"metadata", "manifestSha256", "artifacts"}, key)
    if replay["metadata"] != metadata or replay["manifestSha256"] != hashlib.sha256(text.encode()).hexdigest():
        fail(f"{generation} replay report does not match its exact bundle")
    if not isinstance(replay["artifacts"], list) or len(replay["artifacts"]) != 8:
        fail(f"{generation} replay must report exactly eight artifact records")
    if {(item["mode"], item["case"]): item for item in replay["artifacts"]} != artifacts:
        fail(f"{generation} replay artifact report is incomplete or duplicated")


def check_measured_artifacts(report, report_path):
    directory = report.get("evidenceDirectory")
    if not isinstance(directory, str) or re.fullmatch(r"measurement-[0-9]+-[0-9]+", directory) is None:
        fail("invalid measured evidence directory")
    root = report_path.parent / directory
    try:
        if not stat.S_ISDIR(root.lstat().st_mode):
            fail("measured evidence directory must not be a symlink")
    except OSError as error:
        fail(f"measured evidence directory is missing: {error}")
    suffix = dynamic_suffix(host_target_name())
    endings = {
        "candidateNative": "native", "currentClang": "clang",
        "replayV011Clang": "replay-v011-clang", "replayV010Clang": "replay-v010-clang",
    }
    records = report.get("measuredArtifacts")
    if not isinstance(records, list) or len(records) != 32:
        fail("report must retain exactly thirty-two scalar measured artifact records")
    seen = set()
    sizes = {}
    for item in records:
        exact_keys(item, {"case", "mode", "channel", "file", "bytes", "sha256"}, "measured artifact")
        case, mode, channel = item["case"], item["mode"], item["channel"]
        identity = mode, case, channel
        if (case not in SCALAR_CASES or mode not in {"checked", "unchecked"}
                or channel not in endings or identity in seen):
            fail("unknown or duplicate measured artifact")
        expected = f"{case}-{mode}-{endings[channel]}{suffix}"
        if item["file"] != expected:
            fail("measured artifact filename escapes its exact identity")
        verify_file(root / expected, item["bytes"], item["sha256"], "measured artifact")
        seen.add(identity)
        sizes[identity] = item["bytes"]
    return root, sizes


def check_stream(record, prefix, field, count=20):
    samples = stable_samples(record.get(prefix + "SamplesNs"), f"{field} {prefix}SamplesNs", count)
    median = positive(record.get(prefix + "MedianNs"), f"{field} {prefix}MedianNs")
    if median != upper_median(samples):
        fail(f"{field} {prefix} median does not match its sample array")
    return median


def load_v010(path):
    if file_digest(path) != V010_MANIFEST_SHA256:
        fail("frozen manifest SHA-256 does not match accepted V0.10")
    with path.open("rb") as source:
        manifest = tomllib.load(source)
    if (manifest.get("schema_version") != 2 or manifest.get("commit") != V010_COMMIT
            or manifest.get("compiler_identity") != V010_COMPILER
            or manifest.get("llvm_version") != LLVM_VERSION):
        fail("V0.10 baseline identity is not pinned")
    runtime = {}
    for row in manifest.get("runtime", []):
        key = row["target"], row["cpu"], row["mode"], row["case"]
        if key in runtime:
            fail("duplicate V0.10 runtime baseline")
        runtime[key] = row["median_ns"], row["clang_median_ns"]
    return manifest, runtime


def check_baseline_identity(report, manifest):
    baseline = report.get("baselineV010")
    source_digests = {key.removeprefix("source_digest_"): value for key, value in manifest.items()
                      if key.startswith("source_digest_")}
    expected = {
        "commit": V010_COMMIT, "compilerIdentity": V010_COMPILER,
        "llvmVersion": LLVM_VERSION, "target": host_target_name(),
        "harness": manifest["harness"], "statistics": manifest["statistics"],
        "sourceDigestCount": len(source_digests), "sourceDigests": source_digests,
    }
    if baseline != expected:
        fail("baselineV010 identity/corpus does not match the frozen manifest")


def check_scalar(report, runtime_baseline, sizes):
    suites = report.get("suites")
    if not isinstance(suites, list) or len(suites) != 2:
        fail("checked and unchecked scalar suites must be reported separately")
    modes = {}
    proof = {}
    results = {}
    for suite in suites:
        if not isinstance(suite, dict) or set(suite) != {"mode", "cases"}:
            fail("malformed scalar suite")
        mode = suite["mode"]
        if mode not in {"checked", "unchecked"} or mode in modes or not isinstance(suite["cases"], list):
            fail("duplicate or malformed scalar suite")
        names = set()
        clang_ratios = []
        v011_ratios = []
        v010_ratios = []
        for case in suite["cases"]:
            required = {
                "name", "referenceEquivalent", "nativeCompileNs", "clangCCompileNs",
                "nativeColdNs", "clangCColdNs", "nativeMedianNs", "clangCMedianNs",
                "v010MedianNs", "v010ClangMedianNs", "proofLoop", "nativeSamplesNs",
                "clangCSamplesNs", "peakMemoryBytes", "nativeArtifactBytes",
                "clangCArtifactBytes", "batchIterations", "result",
                "replayV011NativeMedianNs", "replayV011ClangMedianNs",
                "replayV011NativeSamplesNs", "replayV011ClangSamplesNs",
                "replayV010NativeMedianNs", "replayV010ClangMedianNs",
                "replayV010NativeSamplesNs", "replayV010ClangSamplesNs",
                "warmupOrder", "sampleOrder",
            }
            exact_keys(case, required, f"scalar {mode} case")
            name = case["name"]
            if name not in SCALAR_CASES or name in names or case["referenceEquivalent"] is not True:
                fail("scalar suites must cover the exact equivalent corpus without duplicates")
            names.add(name)
            field = f"scalar {mode}/{name}"
            values = {prefix: check_stream(case, prefix, field) for prefix in [
                "native", "clangC", "replayV011Native", "replayV011Clang",
                "replayV010Native", "replayV010Clang",
            ]}
            check_order(case["warmupOrder"], 12, 3, f"{field} warmup order")
            check_order(case["sampleOrder"], 12, 20, f"{field} sample order")
            for key in ["nativeCompileNs", "clangCCompileNs", "nativeColdNs", "clangCColdNs",
                        "peakMemoryBytes", "nativeArtifactBytes", "clangCArtifactBytes"]:
                positive(case[key], f"{field} {key}")
            if case["batchIterations"] != 20_000_000 or type(case["result"]) is not int:
                fail(f"{field} must use the exact batch and validated integer result")
            historical = runtime_baseline.get((host_target_name(), "baseline", mode, name))
            if historical != (case["v010MedianNs"], case["v010ClangMedianNs"]):
                fail(f"{field} historical medians do not match the frozen manifest")
            if case["nativeArtifactBytes"] != sizes[mode, name, "candidateNative"] or case["clangCArtifactBytes"] != sizes[mode, name, "currentClang"]:
                fail(f"{field} artifact size does not match retained evidence")
            if values["native"] / values["clangC"] > 1.10:
                fail(f"{field} is more than 10% slower than strict Clang")
            clang_ratios.append(values["clangC"] / values["native"])
            for generation in ["V011", "V010"]:
                ratio = (values["native"] / values["clangC"]) / (
                    values[f"replay{generation}Native"] / values[f"replay{generation}Clang"]
                )
                if ratio > 1.08:
                    fail(f"{field} regressed more than 8% from pinned {generation.lower()}")
                (v011_ratios if generation == "V011" else v010_ratios).append(ratio)
            if case["proofLoop"] is True:
                if name != "proof_loop":
                    fail("proof-loop corpus contains a wrong case")
                proof[mode] = values["native"]
            results[mode, name] = case["result"]
        if names != SCALAR_CASES:
            fail("scalar suites do not cover the exact frozen corpus")
        if geometric_mean(clang_ratios, field) < .95:
            fail(f"{mode} scalar geometric-mean throughput is below 95% of Clang")
        if geometric_mean(v011_ratios, field) > 1.03:
            fail(f"{mode} scalar geometric-mean regression exceeds 3% from v0.11")
        if geometric_mean(v010_ratios, field) > 1.03:
            fail(f"{mode} scalar geometric-mean regression exceeds 3% from v0.10")
        modes[mode] = names
    if set(proof) != {"checked", "unchecked"} or proof["unchecked"] / proof["checked"] < .97:
        fail("checked proof-loop throughput is below 97% of unchecked")
    for name in SCALAR_CASES:
        if results["checked", name] != results["unchecked", name]:
            fail("checked and unchecked scalar results differ")


def check_oracle_identity(report):
    identity = report.get("oracleIdentity")
    expected_keys = {"manifestSha256", "clangVersion", "rustVersion", "fastMath", "contraction",
                     "differentialAudit", "ubAudit"}
    exact_keys(identity, expected_keys, "oracleIdentity")
    if (identity["manifestSha256"] != ORACLE_MANIFEST_SHA256
            or identity["clangVersion"] != LLVM_VERSION or identity["rustVersion"] != RUST_VERSION
            or identity["fastMath"] is not False or identity["contraction"] is not False
            or identity["differentialAudit"] is not True or identity["ubAudit"] is not True):
        fail("oracle identity requires pinned compilers, strict math, differential and UB audit")


def check_oracle_artifacts(report, root):
    records = report.get("oracleArtifacts")
    expected_count = 2 * 3 * (len(VECTOR_CASES) + len(DOMAIN_CASES))
    if not isinstance(records, list) or len(records) != expected_count:
        fail("oracle artifact evidence is incomplete")
    suffix = dynamic_suffix(host_target_name())
    seen = set()
    for item in records:
        exact_keys(item, {"suite", "case", "mode", "channel", "file", "bytes", "sha256"}, "oracle artifact")
        suite, case, mode, channel = item["suite"], item["case"], item["mode"], item["channel"]
        valid_channels = {"candidate", "cSimd", "rustSimd"} if suite == "vector" else {"candidate", "cGeneric", "rustGeneric"}
        valid_cases = VECTOR_CASES if suite == "vector" else DOMAIN_CASES
        identity = suite, case, mode, channel
        expected = f"{suite}-{case}-{mode}-{channel}{suffix}"
        if (suite not in {"vector", "domain"} or case not in valid_cases
                or mode not in {"checked", "unchecked"} or channel not in valid_channels
                or identity in seen or item["file"] != expected):
            fail("unknown, duplicate, or escaping oracle artifact")
        verify_file(root / expected, item["bytes"], item["sha256"], "oracle artifact")
        seen.add(identity)


def check_oracle_suites(report, key, names, prefixes, domain):
    suites = report.get(key)
    if not isinstance(suites, list) or len(suites) != 2:
        fail(f"{key} must report checked and unchecked separately")
    mode_results = {}
    for suite in suites:
        if not isinstance(suite, dict) or set(suite) != {"mode", "cases"}:
            fail(f"malformed {key}")
        mode = suite["mode"]
        if mode not in {"checked", "unchecked"} or mode in mode_results:
            fail(f"duplicate {key} mode")
        seen = set()
        ratios = []
        results = {}
        for case in suite["cases"]:
            required = {"name", "referenceEquivalent", "validDomain", "resultDigest",
                        "batchIterations", "warmupOrder", "sampleOrder"}
            required |= {prefix + suffix for prefix in prefixes
                         for suffix in ["MedianNs", "SamplesNs"]}
            exact_keys(case, required, f"{key} case")
            name = case["name"]
            if (name not in names or name in seen or case["referenceEquivalent"] is not True
                    or case["validDomain"] is not True):
                fail(f"{key} does not cover its exact valid corpus")
            seen.add(name)
            hash_value(case["resultDigest"], f"{key}/{name} result digest")
            if case["batchIterations"] != 20_000_000:
                fail(f"{key}/{name} batch identity is wrong")
            check_order(case["warmupOrder"], 3, 3, f"{key}/{name} warmup order")
            check_order(case["sampleOrder"], 3, 20, f"{key}/{name} sample order")
            values = {prefix: check_stream(case, prefix, f"{key}/{mode}/{name}") for prefix in prefixes}
            oracle = min(values[prefixes[1]], values[prefixes[2]])
            throughput = oracle / values[prefixes[0]]
            if not domain and throughput < .90:
                fail(f"{key}/{mode}/{name} is below 90% of its faster SIMD oracle")
            ratios.append(throughput)
            results[name] = case["resultDigest"]
        if seen != names:
            fail(f"{key} corpus is incomplete")
        mean = geometric_mean(ratios, key)
        if domain and mean < 1.05:
            fail(f"{key} does not exceed generic oracles by 5%")
        if not domain and mean < .95:
            fail(f"{key} geometric-mean throughput is below 95% of SIMD oracles")
        mode_results[mode] = results
    if mode_results["checked"] != mode_results["unchecked"]:
        fail(f"{key} checked and unchecked results differ")


def check_size_and_compile(report):
    expected = {(mode, case) for mode in ["checked", "unchecked"] for case in VECTOR_CASES}
    sizes = report.get("artifactSizeComparisons")
    if not isinstance(sizes, list):
        fail("artifactSizeComparisons must be an array")
    seen = set()
    candidate_total = replay_total = 0
    for row in sizes:
        exact_keys(row, {"case", "mode", "sourceSha256", "candidateBytes", "replayV011Bytes"}, "size comparison")
        identity = row["mode"], row["case"]
        if identity not in expected or identity in seen:
            fail("duplicate or unknown size comparison")
        seen.add(identity)
        hash_value(row["sourceSha256"], "size source")
        candidate = positive(row["candidateBytes"], "candidate size")
        replay = positive(row["replayV011Bytes"], "replay size")
        if candidate / replay > 2.5:
            fail("artifact size exceeds the 2.5x individual limit")
        candidate_total += candidate
        replay_total += replay
    if seen != expected:
        fail("artifact size corpus is incomplete")
    if candidate_total / replay_total > 1.35:
        fail("aggregate artifact size grows more than 35%")

    rows = report.get("compileTimeComparisons")
    if not isinstance(rows, list):
        fail("compileTimeComparisons must be an array")
    seen = set()
    ratios = []
    for row in rows:
        required = {"case", "mode", "sourceSha256", "candidateMedianNs", "candidateSamplesNs",
                    "replayV011MedianNs", "replayV011SamplesNs", "warmupOrder", "sampleOrder"}
        exact_keys(row, required, "compile-time comparison")
        identity = row["mode"], row["case"]
        if identity not in expected or identity in seen:
            fail("duplicate or unknown compile-time comparison")
        seen.add(identity)
        hash_value(row["sourceSha256"], "compile-time source")
        check_order(row["warmupOrder"], 2, 3, "compile-time warmup order")
        check_order(row["sampleOrder"], 2, 15, "compile-time sample order")
        candidate = check_stream(row, "candidate", "compile-time", 15)
        replay = check_stream(row, "replayV011", "compile-time", 15)
        ratio = candidate / replay
        if ratio > 2:
            fail("source-to-object compile time exceeds the 2x individual limit")
        ratios.append(ratio)
    if seen != expected:
        fail("compile-time corpus is incomplete")
    if geometric_mean(ratios, "compile time") > 1.5:
        fail("source-to-object compile-time geometric mean exceeds 1.5")


def check_optimizer(report, manifest):
    expected = {row["case"]: row["median_ns"] for row in manifest["optimizer"]
                if row["target"] == host_target_name()}
    rows = report.get("optimizerComparisons")
    if not isinstance(rows, list) or len(rows) != len(expected):
        fail("optimizer comparison corpus is incomplete")
    seen = set()
    ratios = []
    for row in rows:
        exact_keys(row, {"case", "kirMedianNs", "v010MirMedianNs"}, "optimizer comparison")
        name = row["case"]
        if name in seen or row["v010MirMedianNs"] != expected.get(name):
            fail("optimizer comparison does not match frozen corpus")
        seen.add(name)
        ratio = positive(row["kirMedianNs"], "KIR optimizer time") / positive(row["v010MirMedianNs"], "MIR optimizer time")
        if ratio > 3:
            fail("KIR optimizer exceeds the 3x individual limit")
        ratios.append(ratio)
    if set(expected) != seen or statistics.median(ratios) > 2:
        fail("KIR optimizer suite-median exceeds the 2x limit")


def current_candidate_sha():
    completed = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=REPO, text=True,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False,
    )
    if completed.returncode or re.fullmatch(r"[0-9a-f]{40}\n?", completed.stdout) is None:
        fail("cannot resolve the exact candidate SHA")
    sha = completed.stdout.strip()
    github_sha = os.environ.get("GITHUB_SHA")
    if github_sha is not None and github_sha != sha:
        fail("GITHUB_SHA does not equal the checked-out candidate SHA")
    return sha


def file_identity(path: pathlib.Path, name: str):
    metadata = path.lstat()
    if not stat.S_ISREG(metadata.st_mode) or metadata.st_size <= 0:
        fail(f"{name} must be a nonempty regular file")
    return {"path": name, "bytes": metadata.st_size, "sha256": file_digest(path)}


def check_file_identity(record, base: pathlib.Path, field: str):
    exact_keys(record, {"path", "bytes", "sha256"}, field)
    name = record["path"]
    if not isinstance(name, str) or pathlib.PurePosixPath(name).is_absolute() or ".." in pathlib.PurePosixPath(name).parts:
        fail(f"{field} path must be repository-relative and non-escaping")
    verify_file(base / name, record["bytes"], record["sha256"], field)


def canonical_digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def clang_profile_runtime_digest():
    clang = os.environ.get("CKC_CLANG_ORACLE")
    if not clang:
        fail("CKC_CLANG_ORACLE is required")
    completed = subprocess.run(
        [clang, "--print-resource-dir"], text=True,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False,
    )
    if completed.returncode:
        fail("cannot inspect the pinned Clang resource directory")
    resource = pathlib.Path(completed.stdout.strip())
    candidates = sorted(resource.glob("lib/**/libclang_rt.profile*.a"))
    candidates += sorted(resource.glob("lib/**/clang_rt.profile*.lib"))
    if len(candidates) != 1:
        fail("pinned Clang resource directory must contain one host profile runtime")
    return file_digest(candidates[0])


def check_v012_replay(report):
    raw = os.environ.get("CKC_V012_RUNTIME_BUNDLE")
    if not raw:
        fail("CKC_V012_RUNTIME_BUNDLE is required")
    bundle = pathlib.Path(raw)
    manifest = bundle / "replay.tsv"
    text = manifest.read_text(encoding="utf-8")
    lines = text.splitlines()
    if not lines or lines[0] != "ckc-v012-runtime-replay\t2":
        fail("v0.12 replay schema is unsupported")
    fields = {
        "commit", "compilerIdentity", "compilerSha256", "compilerBytes", "llvmVersion",
        "target", "cpuPolicy", "recipeSha256", "adapterSetSha256", "sourceDiffSha256",
        "baselineManifestSha256", "llvmComponentSha256",
    }
    metadata = {}
    archive = None
    for line in lines[1:]:
        parts = line.split("\t")
        if len(parts) == 2 and parts[0] in fields and parts[1]:
            if parts[0] in metadata:
                fail("duplicate v0.12 replay metadata")
            metadata[parts[0]] = parts[1]
        elif len(parts) == 4 and parts[0] == "distributionArchive":
            if archive is not None or parts[1] != "ckc-v012-distribution.tar.gz" or not parts[2].isdigit():
                fail("invalid v0.12 distribution archive record")
            archive = {"file": parts[1], "bytes": int(parts[2]), "sha256": parts[3]}
        else:
            fail("unknown v0.12 replay record")
    if set(metadata) != fields or archive is None:
        fail("v0.12 replay identity or archive is incomplete")
    prefix = os.environ.get("CKC_LLVM_PREFIX")
    if not prefix:
        fail("CKC_LLVM_PREFIX is required to verify replay identity")
    expected = {
        "commit": V012_COMMIT,
        "compilerIdentity": V012_COMPILER,
        "llvmVersion": LLVM_VERSION,
        "target": host_target_name(),
        "cpuPolicy": "baseline",
        "recipeSha256": named_digest(RECIPE_FILES),
        "adapterSetSha256": hashlib.sha256(b"").hexdigest(),
        "sourceDiffSha256": hashlib.sha256(b"").hexdigest(),
        "baselineManifestSha256": V012_MANIFEST_SHA256,
        "llvmComponentSha256": file_digest(pathlib.Path(prefix) / "share/ckc/llvm-build.toml"),
    }
    for key, value in expected.items():
        if metadata.get(key) != value:
            fail(f"v0.12 replay {key} does not match pinned identity")
    compiler = {"file": "ckc-v012", "bytes": int(metadata["compilerBytes"]),
                "sha256": metadata["compilerSha256"]}
    for record, field in [(compiler, "v0.12 replay compiler"), (archive, "v0.12 replay archive")]:
        verify_file(bundle / record["file"], record["bytes"], record["sha256"], field)
    expected_report = {
        "metadata": metadata,
        "manifestSha256": hashlib.sha256(text.encode()).hexdigest(),
        "compiler": compiler,
        "archive": archive,
    }
    if report != expected_report:
        fail("schema-8 replayBundle does not equal the exact v0.12 replay bundle")
    return archive


def check_evidence_artifacts(records, root, field, expected_roles=None):
    if not isinstance(records, list) or not records:
        fail(f"{field} must be a nonempty artifact array")
    seen = set()
    roles = set()
    for record in records:
        exact_keys(record, {"case", "role", "file", "bytes", "sha256"}, field)
        case, role, name = record["case"], record["role"], record["file"]
        if case not in PGO_CASES or not isinstance(role, str) or not role:
            fail(f"{field} has an unknown case or empty role")
        if not isinstance(name, str) or pathlib.PurePath(name).name != name:
            fail(f"{field} artifact filename must be a basename")
        identity = case, role
        if identity in seen:
            fail(f"{field} contains a duplicate case/role")
        seen.add(identity)
        roles.add(role)
        verify_file(root / name, record["bytes"], record["sha256"], field)
    if expected_roles is not None:
        expected = {(case, role) for case in PGO_CASES for role in expected_roles}
        if seen != expected:
            fail(f"{field} does not cover the exact case/role set")
    return roles


def check_workload(report):
    exact_keys(report, {"manifest", "sources", "training", "heldOut", "adversarial"}, "workload")
    expected_files = {
        "manifest": "benches/cases/pgo-cases.tsv",
        "training": "benches/fixtures/pgo/training.tsv",
        "heldOut": "benches/fixtures/pgo/held-out.tsv",
        "adversarial": "benches/fixtures/pgo/adversarial.tsv",
    }
    for key, name in expected_files.items():
        check_file_identity(report[key], REPO, f"workload {key}")
        if report[key]["path"] != name:
            fail(f"workload {key} path is not pinned")
    expected_sources = {
        "benches/fixtures/pgo/branch_layout.ck",
        "benches/fixtures/pgo/call_constant_length.ck",
        "benches/oracles/fixtures/map_u32.ck",
        "benches/oracles/fixtures/zip_u32.ck",
        "benches/fixtures/pgo/compute_bound.ck",
    }
    if not isinstance(report["sources"], list) or {row.get("path") for row in report["sources"]} != expected_sources:
        fail("workload source corpus is not exact")
    for row in report["sources"]:
        check_file_identity(row, REPO, "workload source")
    split_records = []
    for key, split in [("training", "training"), ("heldOut", "held-out"), ("adversarial", "adversarial")]:
        lines = (REPO / expected_files[key]).read_text(encoding="utf-8").splitlines()
        if not lines or lines[0] != f"ckc-pgo-inputs\t1\t{split}":
            fail(f"{split} workload header is invalid")
        rows = [line.split("\t") for line in lines[1:] if line and not line.startswith("#")]
        if any(len(row) != 5 or row[0] not in PGO_CASES for row in rows):
            fail(f"{split} workload contains a malformed row")
        split_records.extend((split, (row[0], row[1])) for row in rows)
        if key != "adversarial" and {row[0] for row in rows} != set(PGO_CASES):
            fail(f"{split} workload must cover every case")
    identities = [record for _, record in split_records]
    if len(identities) != len(set(identities)):
        fail("training, held-out, and adversarial record identities must be disjoint")


def check_schema8_stream(case, prefix, field):
    return check_stream(case, prefix, field, SAMPLE_COUNT)


def check_schema8_cases(report):
    rows = report.get("cases")
    if not isinstance(rows, list) or len(rows) != len(PGO_CASES):
        fail("schema-8 cases must cover the exact corpus")
    seen = set()
    ordinary_replay = []
    pgo_ratios = []
    dispatch_ratios = []
    direct_ratios = []
    combined_ratios = []
    oracle_ratios = []
    for row in rows:
        required = {
            "name", "pgoSensitive", "multiversionEligible", "heldOutOnly", "referenceEquivalent",
            "batchCalls", "resultDigest", "generationMedianNs", "generationSamplesNs",
            "resolverCalls", "warmupOrder", "sampleOrder",
        } | {prefix + suffix for prefix in PGO_CHANNELS for suffix in ["MedianNs", "SamplesNs"]}
        exact_keys(row, required, "schema-8 case")
        name = row["name"]
        if name in seen or name not in PGO_CASES:
            fail("unknown or duplicate schema-8 case")
        seen.add(name)
        pgo_sensitive, eligible = PGO_CASES[name]
        if (row["pgoSensitive"], row["multiversionEligible"], row["heldOutOnly"],
                row["referenceEquivalent"]) != (pgo_sensitive, eligible, True, True):
            fail(f"schema-8 case flags are wrong for {name}")
        if type(row["batchCalls"]) is not int or row["batchCalls"] != 16:
            fail("schema-8 batchCalls must be exactly 16")
        hash_value(row["resultDigest"], f"{name} resultDigest")
        check_order(row["warmupOrder"], 8, 3, f"{name} warmup order")
        check_order(row["sampleOrder"], 8, 20, f"{name} sample order")
        values = {prefix: check_schema8_stream(row, prefix, name) for prefix in PGO_CHANNELS}
        generation = positive(row["generationMedianNs"], f"{name} generationMedianNs")
        generation_samples = stable_samples(row["generationSamplesNs"], f"{name} generationSamplesNs")
        if generation != upper_median(generation_samples) or generation / values["ordinary"] > 5.0:
            fail(f"{name} generation execution exceeds generationOverhead=5.0")
        ordinary_ratio = values["ordinary"] / values["replayV012"]
        if ordinary_ratio > 1.05:
            fail(f"{name} ordinary regression exceeds ordinaryIndividualSlowdown=1.05")
        ordinary_replay.append(ordinary_ratio)
        if pgo_sensitive:
            ratio = values["pgo"] / values["ordinary"]
            if ratio > 1.03:
                fail(f"{name} PGO regression exceeds pgoIndividualSlowdown=1.03")
            pgo_ratios.append(ratio)
        if eligible:
            if row["resolverCalls"] != 1:
                fail(f"{name} resolverCalls must be exactly one")
            dispatch = values["multiversion"] / values["ordinary"]
            if dispatch > 1.03:
                fail(f"{name} dispatch regression exceeds pgoIndividualSlowdown=1.03")
            dispatch_ratios.append(dispatch)
            direct = values["multiversion"] / values["selectedDirect"]
            if direct > 1.05:
                fail(f"{name} dispatch/direct regression exceeds 5%")
            direct_ratios.append(direct)
        faster = min(values["pgo"], values["multiversion"])
        combined = values["combined"] / faster
        if combined > 1.05:
            fail(f"{name} combined individual slowdown exceeds 5%")
        combined_ratios.append(combined)
        oracle = min(values["clangPgo"], values["rustPgo"])
        ck_oracle = values["combined"] / oracle
        if ck_oracle > 1 / .90:
            fail(f"{name} CK throughput is below 90% of its PGO oracle")
        oracle_ratios.append(ck_oracle)
    if seen != set(PGO_CASES):
        fail("schema-8 case corpus is incomplete")
    if geometric_mean(ordinary_replay, "ordinary replay") > 1.02:
        fail("ordinary geometric mean exceeds ordinaryGeoSlowdown=1.02")
    if geometric_mean(pgo_ratios, "PGO") > 1 / 1.05:
        fail("PGO improvement is below pgoGeoImprovement=1.05")
    if geometric_mean(dispatch_ratios, "dispatch") > 1 / 1.08:
        fail("dispatch improvement is below dispatchGeoImprovement=1.08")
    if geometric_mean(direct_ratios, "dispatch direct") > 1 / .98:
        fail("dispatch throughput is below dispatchDirectGeoThroughput=0.98")
    if geometric_mean(combined_ratios, "combined") > 1.02:
        fail("combined geometric mean exceeds combinedGeoSlowdown=1.02")
    if geometric_mean(oracle_ratios, "PGO oracle") > 1 / .95:
        fail("CK geometric mean is below oracleGeoThroughput=0.95")


def check_schema8_compile_size(report):
    compile_rows = report.get("compileTime")
    size_rows = report.get("artifactSize")
    if not isinstance(compile_rows, list) or len(compile_rows) != len(PGO_CASES):
        fail("schema-8 compileTime corpus is incomplete")
    if not isinstance(size_rows, list) or len(size_rows) != len(PGO_CASES):
        fail("schema-8 artifactSize corpus is incomplete")
    compile_ratios = {"pgo": [], "multiversion": [], "combined": []}
    seen = set()
    for row in compile_rows:
        required = {"case", "warmupOrder", "sampleOrder"} | {
            prefix + suffix for prefix in ["ordinary", "pgo", "multiversion", "combined"]
            for suffix in ["MedianNs", "SamplesNs"]
        }
        exact_keys(row, required, "compileTime row")
        name = row["case"]
        if name not in PGO_CASES or name in seen:
            fail("unknown or duplicate compileTime case")
        seen.add(name)
        check_order(row["warmupOrder"], 4, 3, f"{name} compile warmup order")
        check_order(row["sampleOrder"], 4, 15, f"{name} compile sample order")
        values = {prefix: check_stream(row, prefix, f"{name} compile", 15)
                  for prefix in ["ordinary", "pgo", "multiversion", "combined"]}
        for prefix, individual in [("pgo", 2), ("multiversion", 3), ("combined", 4)]:
            ratio = values[prefix] / values["ordinary"]
            if ratio > individual:
                fail(f"{name} {prefix} compile time exceeds {individual}x")
            compile_ratios[prefix].append(ratio)
    for prefix, maximum in [("pgo", 1.5), ("multiversion", 2.5), ("combined", 3.5)]:
        if geometric_mean(compile_ratios[prefix], f"{prefix} compile") > maximum:
            fail(f"{prefix} source-to-object geometric mean exceeds {maximum}x")

    totals = {prefix: 0.0 for prefix in ["ordinary", "pgo", "multiversion", "combined"]}
    seen = set()
    for row in size_rows:
        exact_keys(row, {"case", "ordinaryBytes", "pgoBytes", "multiversionBytes", "combinedBytes"}, "artifactSize row")
        name = row["case"]
        if name not in PGO_CASES or name in seen:
            fail("unknown or duplicate artifactSize case")
        seen.add(name)
        ordinary = positive(row["ordinaryBytes"], f"{name} ordinary size")
        for prefix, individual in [("pgo", 1.5), ("multiversion", 2.5), ("combined", 2.5)]:
            value = positive(row[prefix + "Bytes"], f"{name} {prefix} size")
            if value / ordinary > individual:
                fail(f"{name} {prefix} artifact size exceeds {individual}x")
            totals[prefix] += value
        totals["ordinary"] += ordinary
    for prefix, maximum in [("pgo", 1.25), ("multiversion", 2.0), ("combined", 2.0)]:
        if totals[prefix] / totals["ordinary"] > maximum:
            fail(f"{prefix} aggregate artifact size exceeds {maximum}x")


def check_schema8(report, path, baseline_manifest):
    top_keys = {
        "schemaVersion", "candidateVersion", "candidateSha", "replayCommit", "evidenceDirectory",
        "toolchain", "hardware", "capabilityManifest", "recipe", "workload", "candidateBinary",
        "replayBundle", "cumulativeSchemaSeven", "trainingShards", "finalProfiles", "targetSets",
        "variantObjects", "sampling", "cases", "compileTime", "artifactSize", "archiveSize", "correctness",
    }
    exact_keys(report, top_keys, "schema-8 performance report")
    if report["schemaVersion"] != 8 or report["candidateVersion"] != "0.13.0":
        fail("schemaVersion: 8 and candidate 0.13.0 are required")
    if report["candidateSha"] != current_candidate_sha() or report["replayCommit"] != V012_COMMIT:
        fail("candidateSha or exact v0.12 replay commit mismatch")
    directory = report["evidenceDirectory"]
    if not isinstance(directory, str) or re.fullmatch(r"v013-measurement-[0-9]+-[0-9]+", directory) is None:
        fail("invalid schema-8 evidenceDirectory")
    root = path.parent / directory
    if not stat.S_ISDIR(root.lstat().st_mode):
        fail("schema-8 evidenceDirectory must be a real directory")

    exact_keys(report["toolchain"], {"llvmVersion", "clangVersion", "rustVersion", "componentManifestSha256", "clangProfileRuntimeSha256"}, "toolchain")
    prefix = os.environ.get("CKC_LLVM_PREFIX")
    if not prefix:
        fail("CKC_LLVM_PREFIX is required")
    if report["toolchain"] != {
        "llvmVersion": LLVM_VERSION, "clangVersion": LLVM_VERSION, "rustVersion": RUST_VERSION,
        "componentManifestSha256": file_digest(pathlib.Path(prefix) / "share/ckc/llvm-build.toml"),
        "clangProfileRuntimeSha256": clang_profile_runtime_digest(),
    }:
        fail("schema-8 toolchain identity mismatch")
    exact_keys(report["hardware"], {"target", "arch", "os", "cpuModel", "logicalCpus"}, "hardware")
    if report["hardware"]["target"] != host_target_name() or not isinstance(report["hardware"]["cpuModel"], str):
        fail("schema-8 hardware identity mismatch")
    positive(report["hardware"]["logicalCpus"], "hardware logicalCpus")

    capability = report["capabilityManifest"]
    exact_keys(capability, {"schema", "targetSetSchema", "requiredTier", "availableTiers", "features", "osState", "resolverPolicy", "digest"}, "capabilityManifest")
    material = dict(capability)
    digest = material.pop("digest")
    if capability["schema"] != 1 or capability["targetSetSchema"] != 1 or capability["resolverPolicy"] != "resolve-once-before-timing":
        fail("capabilityManifest schema or resolver policy mismatch")
    if not isinstance(capability["availableTiers"], list) or len(capability["availableTiers"]) < 2:
        fail("capabilityManifest lacks a required enhanced tier")
    allowed_tiers = (
        {"baseline", "x86-64-v3", "x86-64-v4"}
        if report["hardware"]["arch"] == "x86_64"
        else {"baseline", "aarch64-sve", "aarch64-sve2"}
    )
    if not set(capability["availableTiers"]).issubset(allowed_tiers):
        fail("capabilityManifest contains an unsupported or diagnostic tier")
    if capability["requiredTier"] not in capability["availableTiers"] or digest != canonical_digest(material):
        fail("capabilityManifest tier or digest mismatch")
    hash_value(digest, "capabilityManifest digest")

    recipe = report["recipe"]
    exact_keys(recipe, {"schema", "files", "digest", "thresholds"}, "recipe")
    if recipe["schema"] != 1 or recipe["thresholds"] != SCHEMA8_THRESHOLDS:
        fail("schema-8 recipe schema or thresholds mismatch")
    if not isinstance(recipe["files"], list) or {row.get("path") for row in recipe["files"]} != set(SCHEMA8_RECIPE_FILES):
        fail("schema-8 recipe file set is incomplete")
    for row in recipe["files"]:
        check_file_identity(row, REPO, "recipe file")
    if recipe["digest"] != named_digest(SCHEMA8_RECIPE_FILES):
        fail("schema-8 recipe digest mismatch")
    check_workload(report["workload"])

    candidate = report["candidateBinary"]
    exact_keys(candidate, {"file", "bytes", "sha256"}, "candidateBinary")
    if candidate["file"] != "ckc-v013":
        fail("candidateBinary filename is not pinned")
    verify_file(root / candidate["file"], candidate["bytes"], candidate["sha256"], "candidateBinary")
    replay_archive = check_v012_replay(report["replayBundle"])

    cumulative = report["cumulativeSchemaSeven"]
    exact_keys(cumulative, {"file", "bytes", "sha256"}, "cumulativeSchemaSeven")
    if cumulative["file"] != "results-schema7.json":
        fail("cumulativeSchemaSeven filename is not pinned")
    cumulative_path = root / cumulative["file"]
    verify_file(cumulative_path, cumulative["bytes"], cumulative["sha256"], "cumulativeSchemaSeven")
    check_schema7(
        json.loads(cumulative_path.read_text(encoding="utf-8"), object_pairs_hook=strict_json_object),
        cumulative_path,
        baseline_manifest,
    )

    check_evidence_artifacts(report["trainingShards"], root, "trainingShards", ["baseline", "multiversion"])
    check_evidence_artifacts(report["finalProfiles"], root, "finalProfiles", ["baseline", "multiversion"])
    variant_roles = [
        "ordinary", "pgo", "multiversion", "combined",
        "selected-direct", "clang-pgo", "rust-pgo",
    ]
    check_evidence_artifacts(
        report["variantObjects"], root, "variantObjects", variant_roles
    )

    target_sets = report["targetSets"]
    if not isinstance(target_sets, list) or len(target_sets) != len(PGO_CASES) * 2:
        fail("targetSets must bind baseline and multiversion profiles for every case")
    seen = set()
    for row in target_sets:
        exact_keys(row, {"case", "policy", "schema", "digest", "tiers"}, "targetSets row")
        identity = row["case"], row["policy"]
        if row["case"] not in PGO_CASES or row["policy"] not in {"baseline", "multiversion"} or identity in seen:
            fail("unknown or duplicate targetSets row")
        seen.add(identity)
        if row["schema"] != 1 or not isinstance(row["tiers"], list) or not row["tiers"]:
            fail("targetSets row has invalid schema or tiers")
        hash_value(row["digest"], "targetSets digest")

    sampling = report["sampling"]
    exact_keys(sampling, {"protocol", "warmupRows", "sampleRows", "callsPerSample", "channelNames", "stabilityPolicy", "rerunPolicy"}, "sampling")
    if sampling != {
        "protocol": "rotating-eight-channel-v1", "warmupRows": 3, "sampleRows": 20,
        "callsPerSample": 7, "channelNames": PGO_CHANNELS,
        "stabilityPolicy": "at-least-80-percent-within-25-percent-of-median",
        "rerunPolicy": "unstable-evidence-is-invalid-no-selective-rerun",
    }:
        fail("schema-8 sampling protocol is not pinned")
    check_schema8_cases(report)
    check_schema8_compile_size(report)

    archive = report["archiveSize"]
    exact_keys(archive, {"candidateFile", "candidateBytes", "candidateSha256", "replayFile", "replayBytes", "replaySha256"}, "archiveSize")
    verify_file(root / archive["candidateFile"], archive["candidateBytes"], archive["candidateSha256"], "candidate archive")
    if (archive["replayFile"], archive["replayBytes"], archive["replaySha256"]) != (
        replay_archive["file"], replay_archive["bytes"], replay_archive["sha256"]
    ):
        fail("archiveSize does not bind the exact v0.12 archive")
    if archive["candidateBytes"] / archive["replayBytes"] > 1.15:
        fail("distributed compiler archive exceeds archiveGrowth=1.15")
    expected_correctness = {
        "training": True, "heldOut": True, "adversarial": True,
        "differential": True, "ubAudit": True, "featureAudit": True,
    }
    if report["correctness"] != expected_correctness:
        fail("schema-8 correctness evidence is incomplete")


def check_schema7(report, path: pathlib.Path, baseline_manifest: pathlib.Path):
    top_keys = {
        "schemaVersion", "candidateVersion", "cpuPolicy", "fastMath", "clangVersion",
        "rustVersion", "warmup", "sampleRepetitions", "samplingProtocol", "channelNames",
        "targetProfile", "runtimeReplayV011", "runtimeReplayV010", "evidenceDirectory",
        "measuredArtifacts", "suites", "vectorSuites", "domainFactSuites", "oracleIdentity",
        "oracleArtifacts", "artifactSizeComparisons", "compileTimeComparisons", "baselineV010",
        "optimizerComparisons",
    }
    exact_keys(report, top_keys, "performance report")
    if report["schemaVersion"] != 7:
        fail("performance report schemaVersion must be 7")
    if report["candidateVersion"] != "0.13.0":
        fail("candidateVersion must identify the 0.13.0 candidate")
    if report["cpuPolicy"] != "baseline":
        fail("release performance requires baseline CPU policy")
    if report["fastMath"] is not False:
        fail("fast-math is forbidden")
    if report["clangVersion"] != LLVM_VERSION:
        fail("Clang identity must be 22.1.8")
    if report["rustVersion"] != RUST_VERSION:
        fail("Rust identity must be 1.90.0")
    if report["warmup"] != 3 or report["sampleRepetitions"] != 7:
        fail("warmup/sampleRepetitions do not match the pinned schedule")
    if report["samplingProtocol"] != "rotating-twelve-channel-v1" or report["channelNames"] != CHANNEL_NAMES:
        fail("sampling protocol or channel order is not pinned")
    exact_keys(report["targetProfile"], {"digest", "costSchema", "proofSchema", "budgetSchema"}, "target profile")
    hash_value(report["targetProfile"]["digest"], "target profile digest")
    if any(report["targetProfile"][name] != 1 for name in ["costSchema", "proofSchema", "budgetSchema"]):
        fail("target profile schema identities must be version 1")
    manifest, runtime = load_v010(baseline_manifest)
    check_baseline_identity(report, manifest)
    check_replay(report, "runtimeReplayV011", "v011", path)
    check_replay(report, "runtimeReplayV010", "v010", path)
    evidence_root, sizes = check_measured_artifacts(report, path)
    check_scalar(report, runtime, sizes)
    check_oracle_identity(report)
    check_oracle_artifacts(report, evidence_root)
    check_oracle_suites(report, "vectorSuites", VECTOR_CASES,
                        ["candidate", "cSimd", "rustSimd"], False)
    check_oracle_suites(report, "domainFactSuites", DOMAIN_CASES,
                        ["candidate", "cGeneric", "rustGeneric"], True)
    check_size_and_compile(report)
    check_optimizer(report, manifest)


def check(path: pathlib.Path, baseline_manifest: pathlib.Path):
    report = json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=strict_json_object)
    if report.get("schemaVersion") == 8:
        check_schema8(report, path, baseline_manifest)
    else:
        check_schema7(report, path, baseline_manifest)


def main():
    if len(sys.argv) not in {2, 3}:
        print(f"usage: {pathlib.Path(sys.argv[0]).name} <results.json> [v0_10_baseline.toml]", file=sys.stderr)
        return 2
    baseline = pathlib.Path(sys.argv[2]) if len(sys.argv) == 3 else DEFAULT_BASELINE_MANIFEST
    try:
        check(pathlib.Path(sys.argv[1]), baseline)
    except (OSError, ValueError, json.JSONDecodeError, tomllib.TOMLDecodeError) as error:
        print(f"native performance gate failed: {error}", file=sys.stderr)
        return 1
    print("native performance gate passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
