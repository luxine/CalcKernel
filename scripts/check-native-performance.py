#!/usr/bin/env python3
"""Validate the fail-closed CK 0.13 schema-8 and cumulative schema-7 contracts."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import io
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
import tarfile
import tempfile
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
ORACLE_MANIFEST_SHA256 = "c97e6d52e7de437993c89f9cb0e31b2c32017642429331f18300b57438799414"
DEFAULT_BASELINE_MANIFEST = REPO / "benches/baselines/v0_10_compiler.toml"
V013_REPLAY_MANIFEST = REPO / "benches/baselines/v0_13_replay.toml"
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

V012_COMMIT = "a49fa419669c400447dc13bcfa41ea464b3b040d"
V012_COMPILER = f"calckernel 0.12.0 ({V012_COMMIT})"
V012_MANIFEST_SHA256 = "ad528ab399ec0ad5111e44731e55ddc06e23c15f05740c8dcaf2a3002eed5c67"
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

SCHEMA9_TOP_KEYS = {
    "schemaVersion", "candidateVersion", "candidateSha", "v013ReplayCommit",
    "evidenceDirectory", "toolchain", "hardware", "recipe", "candidateBinary",
    "v013ReplayBundle", "cumulativeSchemaEight", "workload", "tuningDecisions",
    "tuningArtifacts", "sampling", "cases", "validationCases", "domainCases",
    "tuneUseCompileTime", "ordinaryCompileRegression", "artifactSize", "archiveSize",
    "resourceUse", "determinism", "correctness",
}
SCHEMA9_THRESHOLDS = {
    "archiveMaximumDen": 100, "archiveMaximumNum": 110,
    "artifactMaximumDen": 100, "artifactMaximumNum": 110,
    "cacheBytesMaximum": 4_294_967_296,
    "domainThroughputMinimumDen": 100, "domainThroughputMinimumNum": 108,
    "heldOutGeomeanMaximumDen": 100, "heldOutGeomeanMaximumNum": 95,
    "oracleCaseThroughputMinimumDen": 100, "oracleCaseThroughputMinimumNum": 92,
    "oracleGeomeanThroughputMinimumDen": 100, "oracleGeomeanThroughputMinimumNum": 98,
    "ordinaryCompileCaseMaximumDen": 100, "ordinaryCompileCaseMaximumNum": 108,
    "ordinaryCompileGeomeanMaximumDen": 100, "ordinaryCompileGeomeanMaximumNum": 103,
    "peakRssMaximumDen": 1, "peakRssMaximumNum": 2,
    "selectedCaseMaximumDen": 100, "selectedCaseMaximumNum": 98,
    "standardWallMsMaximum": 1_800_000,
    "tuneUseCompileCaseMaximumDen": 100, "tuneUseCompileCaseMaximumNum": 120,
    "tuneUseCompileGeomeanMaximumDen": 100, "tuneUseCompileGeomeanMaximumNum": 110,
    "validationOrHeldOutMaximumDen": 100, "validationOrHeldOutMaximumNum": 102,
}
SCHEMA9_RECIPE_FILES = [
    "benches/cases/tune-cases.tsv",
    *[f"benches/tune/workloads/{name}.cktune.toml" for name in [
        "branch-layout", "call-constant-length", "compute-bound",
        "contract-fixed-length", "contract-noalias", "memory-bound", "trip-unroll-simd",
    ]],
    "benches/tune/runner.rs", "benches/oracles/tune/manifest.toml",
    "benches/oracles/tune/c/tune_oracle.c", "benches/oracles/tune/rust/tune_oracle.rs",
    "benches/fixtures/pgo/branch_layout.ck", "benches/fixtures/pgo/call_constant_length.ck",
    "benches/oracles/fixtures/map_u32.ck", "benches/oracles/fixtures/zip_u32.ck",
    "benches/fixtures/pgo/compute_bound.ck", "benches/oracles/fixtures/contract_noalias.ck",
    "benches/oracles/fixtures/contract_fixed_length.ck", "benches/fixtures/pgo/training.tsv",
    "benches/fixtures/pgo/held-out.tsv", "benches/fixtures/pgo/adversarial.tsv",
    "benches/fixtures/tune/release-held-out.tsv", "benches/tune_perf.rs",
    "scripts/measure-v014-performance.py", "scripts/check-native-performance.py",
    "scripts/audit-performance-oracles.py", "scripts/package-v014-performance-archive.py",
    "LICENSE", "THIRD_PARTY_NOTICES.md", "benches/baselines/v0_13_replay.toml",
    "specs/0.14/performance-schema-9.md",
]
SCHEMA9_CASES = {
    "branch-layout", "call-constant-length", "compute-bound", "contract-fixed-length",
    "contract-noalias", "memory-bound", "trip-unroll-simd",
}
SCHEMA9_MAIN_CASES = SCHEMA9_CASES - {"contract-fixed-length", "contract-noalias"}
SCHEMA9_DOMAIN_CASES = {"contract-fixed-length", "contract-noalias"}
SCHEMA9_MAIN_CHANNELS = [
    "tuned", "v014Ordinary", "v013Ordinary", "v013Pgo", "cSimd", "rustSimd",
]
SCHEMA9_VALIDATION_CHANNELS = ["tuned", "v013Ordinary", "v013Pgo"]
SCHEMA9_DOMAIN_CHANNELS = ["tuned", "genericC", "genericRust"]


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


def schema9_text(value):
    encoded = value.encode("utf-8")
    return len(encoded).to_bytes(4, "big") + encoded


def schema9_list(values):
    return len(values).to_bytes(4, "big") + b"".join(values)


def schema9_file_value(value):
    return ({"repository": 1, "evidence": 2}[value["root"]].to_bytes(1, "big")
            + schema9_text(value["path"]) + value["bytes"].to_bytes(8, "big")
            + bytes.fromhex(value["sha256"]))


def schema9_digest(domain, *values):
    digest = hashlib.sha256(domain)
    for value in values:
        digest.update(value)
    return digest.hexdigest()


def schema9_relative(value, field):
    if not isinstance(value, str) or not value or "\\" in value:
        fail(f"{field} path must be a nonempty portable relative path")
    path = pathlib.PurePosixPath(value)
    if path.is_absolute() or any(part in {"", ".", ".."} for part in path.parts):
        fail(f"{field} path is absolute or traversing")
    return path


def check_schema9_file(value, evidence_root, field, expected_root=None):
    exact_keys(value, {"root", "path", "bytes", "sha256"}, field)
    root = value["root"]
    if root not in {"repository", "evidence"} or (expected_root and root != expected_root):
        fail(f"{field} has the wrong root")
    relative = schema9_relative(value["path"], field)
    base = REPO if root == "repository" else evidence_root
    target = base.joinpath(*relative.parts)
    try:
        metadata = target.lstat()
    except OSError as error:
        fail(f"{field} is missing: {error}")
    if target.is_symlink() or not stat.S_ISREG(metadata.st_mode) or metadata.st_size <= 0:
        fail(f"{field} must resolve to a nonempty regular non-symlink file")
    if type(value["bytes"]) is not int or value["bytes"] != metadata.st_size:
        fail(f"{field} byte count mismatch")
    hash_value(value["sha256"], f"{field} sha256")
    if file_digest(target) != value["sha256"]:
        fail(f"{field} SHA-256 mismatch")
    return target


def schema9_case_table():
    lines = (REPO / "benches/cases/tune-cases.tsv").read_text(encoding="utf-8").splitlines()
    if not lines or lines[0] != "ckc-tune-cases\t1":
        fail("schema-9 case table header is invalid")
    rows = {}
    for line in lines[1:]:
        if not line or line.startswith("#"):
            continue
        fields = line.split("\t")
        if len(fields) != 13 or fields[0] in rows:
            fail("schema-9 case table contains a malformed or duplicate row")
        rows[fields[0]] = {
            "source": fields[1], "manifest": fields[2], "searchDigest": fields[5],
            "validationDigest": fields[8], "releaseDigest": fields[11],
            "partition": fields[12],
        }
    if set(rows) != SCHEMA9_CASES:
        fail("schema-9 case table corpus is incomplete")
    return rows


def check_schema9_hardware(value, contract_only):
    keys = {
        "target", "arch", "os", "osBuild", "kernel", "cpuModel", "logicalCpus",
        "physicalCpus", "numaNodes", "features", "requiredTier", "availableTiers",
        "osState", "capabilityDigest",
    }
    exact_keys(value, keys, "schema-9 hardware")
    for key in ["target", "arch", "os", "osBuild", "kernel", "cpuModel", "requiredTier", "osState"]:
        if not isinstance(value[key], str) or not value[key]:
            fail(f"schema-9 hardware {key} must be nonempty text")
    for key in ["logicalCpus", "physicalCpus", "numaNodes"]:
        if type(value[key]) is not int or not 0 < value[key] <= 0xffff_ffff:
            fail(f"schema-9 hardware {key} must be a positive u32")
    for key in ["features", "availableTiers"]:
        if (not isinstance(value[key], list) or value[key] != sorted(set(value[key]))
                or any(not isinstance(item, str) or not item for item in value[key])):
            fail(f"schema-9 hardware {key} must be sorted unique text")
    if value["requiredTier"] not in value["availableTiers"]:
        fail("schema-9 required hardware tier is unavailable")
    material = [
        schema9_text(value[key]) for key in [
            "target", "arch", "os", "osBuild", "kernel", "cpuModel",
        ]
    ]
    material += [value[key].to_bytes(4, "big") for key in ["logicalCpus", "physicalCpus", "numaNodes"]]
    material += [schema9_list([schema9_text(item) for item in value["features"]]),
                 schema9_text(value["requiredTier"]),
                 schema9_list([schema9_text(item) for item in value["availableTiers"]]),
                 schema9_text(value["osState"])]
    if value["capabilityDigest"] != schema9_digest(b"CK-V014-PERF-HARDWARE\0", *material):
        fail("schema-9 hardware capabilityDigest mismatch")
    if not contract_only:
        if value["arch"] not in {"x86_64", "aarch64"}:
            fail("schema-9 release evidence has an unsupported architecture")
        required = "x86-64-v4" if value["arch"] == "x86_64" else "aarch64-sve2"
        needed = ({"avx512f", "avx512bw", "avx512cd", "avx512dq", "avx512vl"}
                  if value["arch"] == "x86_64" else {"sve", "sve2"})
        if value["os"] != "linux" or value["requiredTier"] != required:
            fail("schema-9 release evidence requires a stable Linux hardware tier")
        if not needed.issubset(value["features"]):
            fail("schema-9 release evidence lacks required hardware features")


def check_schema9_recipe(value, evidence_root):
    exact_keys(value, {"schema", "files", "digest", "thresholds"}, "schema-9 recipe")
    if value["schema"] != 1 or value["thresholds"] != SCHEMA9_THRESHOLDS:
        fail("schema-9 recipe schema or thresholds mismatch")
    if not isinstance(value["files"], list) or len(value["files"]) != len(SCHEMA9_RECIPE_FILES):
        fail("schema-9 recipe file cardinality mismatch")
    paths = []
    for item in value["files"]:
        check_schema9_file(item, evidence_root, "schema-9 recipe file", "repository")
        paths.append(item["path"])
    if paths != sorted(SCHEMA9_RECIPE_FILES) or len(paths) != len(set(paths)):
        fail("schema-9 recipe file set/order mismatch")
    threshold_values = [schema9_text(name) + number.to_bytes(8, "big")
                        for name, number in sorted(SCHEMA9_THRESHOLDS.items())]
    expected = schema9_digest(
        b"CK-V014-PERF-RECIPE\0", (1).to_bytes(4, "big"),
        schema9_list([schema9_file_value(item) for item in value["files"]]),
        schema9_list(threshold_values),
    )
    if value["digest"] != expected:
        fail("schema-9 recipe digest mismatch")


def check_schema9_workload(value, evidence_root, table):
    keys = {
        "casesManifest", "sources", "search", "validation", "adversarial",
        "releaseHeldOut", "tuneManifests", "runner", "oracleManifest", "cOracle",
        "rustOracle", "profiles", "expectedResults",
    }
    exact_keys(value, keys, "schema-9 workload")
    scalar = {
        "casesManifest": "benches/cases/tune-cases.tsv",
        "search": "benches/fixtures/pgo/training.tsv",
        "validation": "benches/fixtures/pgo/held-out.tsv",
        "adversarial": "benches/fixtures/pgo/adversarial.tsv",
        "releaseHeldOut": "benches/fixtures/tune/release-held-out.tsv",
        "oracleManifest": "benches/oracles/tune/manifest.toml",
        "cOracle": "benches/oracles/tune/c/tune_oracle.c",
        "rustOracle": "benches/oracles/tune/rust/tune_oracle.rs",
    }
    for key, path in scalar.items():
        check_schema9_file(value[key], evidence_root, f"schema-9 workload {key}", "repository")
        if value[key]["path"] != path:
            fail(f"schema-9 workload {key} path mismatch")
    check_schema9_file(value["runner"], evidence_root, "schema-9 workload runner", "evidence")
    for key, expected in [
        ("sources", sorted(row["source"] for row in table.values())),
        ("tuneManifests", sorted(f"benches/tune/workloads/{row['manifest']}" for row in table.values())),
    ]:
        if not isinstance(value[key], list) or [row.get("path") for row in value[key]] != expected:
            fail(f"schema-9 workload {key} set/order mismatch")
        for item in value[key]:
            check_schema9_file(item, evidence_root, f"schema-9 workload {key}", "repository")
    if not isinstance(value["profiles"], list):
        fail("schema-9 workload profiles must be a list")
    results = value["expectedResults"]
    if not isinstance(results, list) or len(results) != 7:
        fail("schema-9 expectedResults must contain seven rows")
    seen = set()
    for row in results:
        exact_keys(row, {"case", "split", "input", "canonicalBytes", "digest"},
                   "schema-9 expected result")
        case = row["case"]
        if case not in table or case in seen or row["split"] != "release-held-out":
            fail("schema-9 expected result case/split mismatch")
        seen.add(case)
        if row["input"] != value["releaseHeldOut"]:
            fail("schema-9 expected result input foreign key mismatch")
        raw_path = check_schema9_file(row["canonicalBytes"], evidence_root,
                                     "schema-9 canonical result", "evidence")
        raw = raw_path.read_bytes()
        expected = schema9_digest(
            b"CK-TUNE-RESULT\0", (1).to_bytes(4, "big"), schema9_text(f"{case}.release"),
            len(raw).to_bytes(8, "big"), raw,
        )
        if row["digest"] != expected or expected != table[case]["releaseDigest"]:
            fail("schema-9 expected result digest mismatch")
    if [row["case"] for row in results] != sorted(SCHEMA9_CASES):
        fail("schema-9 expected results are not case-name sorted")


def check_schema9_sampling(value):
    expected = {
        "mainProtocol": "rotating-six-channel-v1",
        "validationProtocol": "rotating-three-channel-v1",
        "domainProtocol": "rotating-three-channel-v1",
        "mainChannels": SCHEMA9_MAIN_CHANNELS,
        "validationChannels": SCHEMA9_VALIDATION_CHANNELS,
        "domainChannels": SCHEMA9_DOMAIN_CHANNELS,
        "warmupRows": 3, "sampleRows": 20, "callsPerSample": 7,
        "statistic": "minimum-then-upper-median",
        "stabilityPolicy": "at-least-80-percent-within-20-percent-of-upper-median",
        "rerunPolicy": "unstable-evidence-is-invalid-no-selective-rerun",
    }
    if value != expected:
        fail("schema-9 sampling contract mismatch")


def check_schema9_schema_only(report, path):
    exact_keys(report, SCHEMA9_TOP_KEYS, "schema-9 performance report")
    if report["schemaVersion"] != 9 or report["candidateVersion"] != "0.14.0":
        fail("schema-9 version identity mismatch")
    if report["candidateSha"] != current_candidate_sha():
        fail("schema-9 candidateSha mismatch")
    with (REPO / "benches/baselines/v0_13_replay.toml").open("rb") as source:
        replay = tomllib.load(source)
    if report["v013ReplayCommit"] != replay.get("commit"):
        fail("schema-9 v013ReplayCommit mismatch")
    directory = report["evidenceDirectory"]
    if not isinstance(directory, str) or re.fullmatch(r"v014-measurement-[0-9]+-[0-9]+", directory) is None:
        fail("schema-9 evidenceDirectory is invalid")
    root = path.parent / directory
    try:
        metadata = root.lstat()
    except OSError as error:
        fail(f"schema-9 evidenceDirectory is missing: {error}")
    if root.is_symlink() or not stat.S_ISDIR(metadata.st_mode):
        fail("schema-9 evidenceDirectory must be a real non-symlink directory")
    correctness_keys = {
        "search", "validation", "adversarial", "validationDifferential",
        "releaseHeldOutDifferential", "domainDifferential", "oracleUbAudit",
        "aliasAudit", "featureAudit",
    }
    exact_keys(report["correctness"], correctness_keys, "schema-9 correctness")
    if any(type(value) is not bool for value in report["correctness"].values()):
        fail("schema-9 correctness fields must be booleans")
    contract_only = all(value is False for value in report["correctness"].values())
    toolchain_keys = {
        "llvmVersion", "clangVersion", "rustVersion", "componentManifest",
        "clangBinary", "clangProfileRuntime", "rustCompiler", "systemLinker",
    }
    exact_keys(report["toolchain"], toolchain_keys, "schema-9 toolchain")
    if (report["toolchain"]["llvmVersion"], report["toolchain"]["clangVersion"],
            report["toolchain"]["rustVersion"]) != (LLVM_VERSION, LLVM_VERSION, RUST_VERSION):
        fail("schema-9 toolchain versions are not pinned")
    for key in ["componentManifest", "clangBinary", "clangProfileRuntime", "rustCompiler",
                "systemLinker"]:
        check_schema9_file(report["toolchain"][key], root, f"schema-9 toolchain {key}", "evidence")
    check_schema9_file(report["candidateBinary"], root, "schema-9 candidateBinary", "evidence")
    check_schema9_hardware(report["hardware"], contract_only)
    check_schema9_recipe(report["recipe"], root)
    table = schema9_case_table()
    check_schema9_workload(report["workload"], root, table)
    check_schema9_sampling(report["sampling"])
    if report["resourceUse"].get("cacheHardLimitBytes") != SCHEMA9_THRESHOLDS["cacheBytesMaximum"]:
        fail("schema-9 cache hard limit mismatch")
    return contract_only, root, table


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


def check_schema8(report, path, baseline_manifest, *, candidate_version="0.13.0",
                  candidate_sha=None):
    top_keys = {
        "schemaVersion", "candidateVersion", "candidateSha", "replayCommit", "evidenceDirectory",
        "toolchain", "hardware", "capabilityManifest", "recipe", "workload", "candidateBinary",
        "replayBundle", "cumulativeSchemaSeven", "trainingShards", "finalProfiles", "targetSets",
        "variantObjects", "sampling", "cases", "compileTime", "artifactSize", "archiveSize", "correctness",
    }
    exact_keys(report, top_keys, "schema-8 performance report")
    expected_sha = candidate_sha or current_candidate_sha()
    if report["schemaVersion"] != 8 or report["candidateVersion"] != candidate_version:
        fail(f"schemaVersion: 8 and candidate {candidate_version} are required")
    if report["candidateSha"] != expected_sha or report["replayCommit"] != V012_COMMIT:
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
    cumulative_report = json.loads(
        cumulative_path.read_text(encoding="utf-8"), object_pairs_hook=strict_json_object)
    if candidate_version == "0.13.0":
        check_schema7(cumulative_report, cumulative_path, baseline_manifest)
    else:
        check_schema7(
            cumulative_report, cumulative_path, baseline_manifest,
            candidate_version=candidate_version)

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


def check_schema7(report, path: pathlib.Path, baseline_manifest: pathlib.Path, *,
                  candidate_version="0.13.0"):
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
    if report["candidateVersion"] != candidate_version:
        fail(f"candidateVersion must identify the {candidate_version} candidate")
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


def schema9_u64(value, field, *, positive_value=False):
    if type(value) is not int or value < (1 if positive_value else 0) or value > 0xffff_ffff_ffff_ffff:
        fail(f"{field} must be a {'positive ' if positive_value else ''}u64")
    return value


def schema9_sorted_files(values, evidence_root, field, expected_root=None):
    if not isinstance(values, list):
        fail(f"{field} must be a list")
    keys = []
    for index, value in enumerate(values):
        check_schema9_file(value, evidence_root, f"{field}[{index}]", expected_root)
        keys.append((0 if value["root"] == "repository" else 1, value["path"].encode("utf-8")))
    if keys != sorted(keys) or len(keys) != len(set(keys)):
        fail(f"{field} must be path-sorted and unique")
    return values


def schema9_tagged(nodes, tag, field):
    if not isinstance(nodes, list):
        fail(f"{field} must be an inspection node list")
    matches = [node.get("value") for node in nodes if isinstance(node, dict) and node.get("tag") == tag]
    if len(matches) != 1:
        fail(f"{field} is missing unique tag {tag}")
    return matches[0]


def schema9_inspect_decision(candidate, decision, field):
    result = subprocess.run(
        [candidate, "tune", "inspect", decision, "--json"], cwd=REPO, text=True,
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False,
    )
    if result.returncode:
        fail(f"{field} decision inspection failed: {result.stdout[-3000:]}")
    try:
        inspection = json.loads(result.stdout, object_pairs_hook=strict_json_object)
    except json.JSONDecodeError as error:
        fail(f"{field} decision inspection is not JSON: {error}")
    records = inspection.get("records")
    if not isinstance(records, list) or len(records) != 8:
        fail(f"{field} decision inspection tree is incomplete")
    selection = schema9_tagged(records, 7, field)
    replay = schema9_tagged(records, 8, field)
    candidates = schema9_tagged(records, 6, field)
    frontier = schema9_tagged(records, 5, field)
    reason = schema9_tagged(selection, 4, field)
    certificate = schema9_tagged(selection, 5, field)
    output_records = []
    for node in schema9_tagged(replay, 6, field):
        output_records.append({
            "role": schema9_tagged(node, 1, field),
            "logicalName": schema9_tagged(node, 2, field),
            "sha256": schema9_tagged(node, 3, field),
            "bytes": schema9_tagged(node, 4, field),
        })
    certificate_digest = None
    if certificate is not None:
        certificate_digest = schema9_digest(
            b"CK-V014-TUNE-CERTIFICATE\0",
            *(bytes.fromhex(schema9_tagged(certificate, tag, field)) for tag in range(1, 9)),
        )
    summary = {
        "decisionDigest": inspection.get("decisionDigest"),
        "choiceIdentityDigest": schema9_tagged(replay, 10, field),
        "selectionReason": reason,
        "planDigest": schema9_tagged(selection, 3, field),
        "objectGraphDigest": schema9_tagged(replay, 4, field),
        "linkRecipeDigest": schema9_tagged(replay, 5, field),
        "certificateDigest": certificate_digest,
        "outputRecords": output_records,
    }
    trials = schema9_tagged(candidates, 2, field)
    first_round = schema9_tagged(selection, 1, field)
    counts = {
        "compiled": len(trials),
        "measured": sum(bool(schema9_tagged(node, 9, field)) for node in trials),
        "expansions": len(schema9_tagged(frontier, 4, field)),
        "validationEntrants": len(schema9_tagged(first_round, 2, field)),
    }
    return summary, counts


def schema9_output_digest(outputs):
    values = []
    for output in outputs:
        file = output["file"]
        values.append(schema9_text(output["role"]) + file["bytes"].to_bytes(8, "big")
                      + bytes.fromhex(file["sha256"]))
    return schema9_digest(b"CK-V014-PERF-OUTPUT-CONTENT\0", schema9_list(values))


def schema9_check_output_set(outputs, evidence_root, field):
    if not isinstance(outputs, list) or not 1 <= len(outputs) <= 3:
        fail(f"{field} must contain one through three outputs")
    rank = {"primary": 0, "header": 1, "import-library": 2}
    roles = []
    for index, output in enumerate(outputs):
        exact_keys(output, {"role", "file"}, f"{field}[{index}]")
        if output["role"] not in rank:
            fail(f"{field} contains an unknown output role")
        check_schema9_file(output["file"], evidence_root, f"{field}[{index}].file", "evidence")
        roles.append(output["role"])
    if roles != sorted(roles, key=rank.get) or len(roles) != len(set(roles)) or "primary" not in roles:
        fail(f"{field} output roles are not complete/canonical")
    return outputs


def schema9_check_environment(entries, evidence_root, field):
    allowed = {
        "CKC_LLVM_PREFIX", "CKC_CLANG_ORACLE", "CKC_CANDIDATE_COMPILER",
        "CKC_V013_REPLAY_BUNDLE", "XDG_CACHE_HOME", "HOME", "LOCALAPPDATA",
        "SystemRoot", "WINDIR",
    }
    if not isinstance(entries, list):
        fail(f"{field} must be a list")
    names = []
    encoded = []
    for index, entry in enumerate(entries):
        exact_keys(entry, {"name", "value", "references"}, f"{field}[{index}]")
        name, value = entry["name"], entry["value"]
        if name not in allowed or not isinstance(value, str):
            fail(f"{field} contains an unknown name or non-text value")
        references = schema9_sorted_files(entry["references"], evidence_root,
                                           f"{field}[{index}].references")
        if name.startswith("CKC_") and not references:
            fail(f"{field} {name} lacks retained references")
        if not name.startswith("CKC_") and references:
            fail(f"{field} {name} must not carry references")
        names.append(name)
        encoded.append(schema9_text(name) + schema9_text(value)
                       + schema9_list([schema9_file_value(item) for item in references]))
    if names != sorted(names) or len(names) != len(set(names)):
        fail(f"{field} names must be sorted and unique")
    return schema9_digest(b"CK-V014-PERF-COMMAND-ENV\0", schema9_list(encoded))


def schema9_check_command(command, evidence_root, field):
    exact_keys(command, {
        "argv", "workingDirectory", "executable", "inputs", "environment",
        "environmentDigest",
    }, field)
    argv = command["argv"]
    if (not isinstance(argv, list) or not argv
            or any(not isinstance(item, str) or not item for item in argv)):
        fail(f"{field} argv must be nonempty text")
    if command["workingDirectory"] != "repository":
        fail(f"{field} workingDirectory must be repository")
    for argument in argv:
        if argument.startswith("/") or "\\" in argument or "../" in argument:
            fail(f"{field} argv contains an absolute or traversing path")
    check_schema9_file(command["executable"], evidence_root, f"{field}.executable")
    executable_argument_matches = (
        argv[0] == command["executable"]["path"]
        if command["executable"]["root"] == "repository"
        else argv[0].endswith(command["executable"]["path"])
    )
    if not executable_argument_matches:
        fail(f"{field} argv[0] does not identify its executable")
    schema9_sorted_files(command["inputs"], evidence_root, f"{field}.inputs")
    digest = schema9_check_environment(command["environment"], evidence_root,
                                       f"{field}.environment")
    if command["environmentDigest"] != digest:
        fail(f"{field} environmentDigest mismatch")
    return schema9_digest(
        b"CK-V014-PERF-COMMAND\0", schema9_list([schema9_text(item) for item in argv]),
        schema9_text("repository"), schema9_file_value(command["executable"]),
        schema9_list([schema9_file_value(item) for item in command["inputs"]]),
        bytes.fromhex(digest),
    )


def schema9_check_build(build, evidence_root, field, *, tuned=None):
    exact_keys(build, {"command", "decision", "outputs"}, field)
    command_digest = schema9_check_command(build["command"], evidence_root, f"{field}.command")
    outputs = schema9_check_output_set(build["outputs"], evidence_root, f"{field}.outputs")
    if build["decision"] is None:
        if tuned is True:
            fail(f"{field} tuned build lacks a decision")
    else:
        check_schema9_file(build["decision"], evidence_root, f"{field}.decision", "evidence")
        if tuned is False:
            fail(f"{field} nontuned build unexpectedly has a decision")
    argv = build["command"]["argv"]
    if tuned is not None:
        if "--out" not in argv:
            fail(f"{field} command omits --out")
        required = ["--kind", "dynamic", "--cpu", "native", "-O3", "--overflow",
                    "unchecked", "--bounds", "unchecked"]
        for token in required:
            if token not in argv:
                fail(f"{field} command omits required token {token}")
        if tuned is True:
            if (argv[1:3] != ["tune", "build"] or "--budget" not in argv
                    or "--tune-out" not in argv):
                fail(f"{field} does not use the closed tuned build template")
        elif len(argv) < 2 or argv[1] != "build":
            fail(f"{field} does not use the closed ordinary build template")
    return outputs, command_digest


def schema9_file_argument(file, evidence_root):
    if file["root"] == "repository":
        return file["path"]
    try:
        return (evidence_root / file["path"]).resolve().relative_to(REPO.resolve()).as_posix()
    except ValueError:
        fail("schema-9 evidence command argument is outside the repository")


def schema9_check_channel_template(build, evidence_root, field, channel, source,
                                   manifest=None, profile=None, oracle=None, case_index=None,
                                   case_name=None, oracle_linkers=None):
    command, argv = build["command"], build["command"]["argv"]
    executable = schema9_file_argument(command["executable"], evidence_root)
    primary = next(item["file"] for item in build["outputs"] if item["role"] == "primary")
    primary_arg = schema9_file_argument(primary, evidence_root)
    if not primary_arg.endswith(".so"):
        fail(f"{field} primary output is not a Linux dynamic library")
    base_arg = primary_arg[:-3]
    ordinary = [
        executable, "build", schema9_file_argument(source, evidence_root), "--out", base_arg,
        "--kind", "dynamic", "--cpu", "native", "-O3", "--overflow", "unchecked",
        "--bounds", "unchecked",
    ]
    if channel == "tuned":
        if manifest is None or build["decision"] is None:
            fail(f"{field} tuned template lacks manifest/decision")
        expected = [
            executable, "tune", "build", schema9_file_argument(source, evidence_root),
            "--config", schema9_file_argument(manifest, evidence_root), "--out", base_arg,
            "--kind", "dynamic", "--cpu", "native", "-O3", "--overflow", "unchecked",
            "--bounds", "unchecked", "--budget", "standard", "--tune-out",
            schema9_file_argument(build["decision"], evidence_root),
        ]
        expected_inputs = [source, manifest]
    elif channel in {"v014Ordinary", "v013Ordinary"}:
        expected = ordinary
        expected_inputs = [source]
    elif channel == "v013Pgo":
        if profile is None:
            fail(f"{field} v013Pgo template lacks profile")
        expected = [*ordinary, "--pgo-use", schema9_file_argument(profile, evidence_root)]
        expected_inputs = [source, profile]
    elif channel in {"cSimd", "genericC"}:
        if oracle is None or oracle_linkers is None:
            fail(f"{field} C oracle template lacks its retained linker closure")
        flavor = "simd_args" if channel == "cSimd" else "generic_args"
        expected = [
            executable, *oracle["c"][flavor],
            f"--ld-path={oracle_linkers['system']}",
            f"-DCK_TUNE_ORACLE_CASE={case_index}",
            schema9_file_argument(source, evidence_root), "-o", primary_arg,
        ]
        expected_inputs = [source, manifest, oracle_linkers["systemIdentity"]]
    elif channel in {"rustSimd", "genericRust"}:
        if oracle is None or oracle_linkers is None:
            fail(f"{field} Rust oracle template lacks its retained linker closure")
        flavor = "simd_args" if channel == "rustSimd" else "generic_args"
        expected = [
            executable, *oracle["rust"][flavor],
            "-C", f"linker={oracle_linkers['clang']}",
            "-C", f"link-arg=--ld-path={oracle_linkers['system']}",
            "--cfg", f'tune_case="{case_name}"',
            schema9_file_argument(source, evidence_root), "-o", primary_arg,
        ]
        expected_inputs = [source, manifest, oracle_linkers["clangIdentity"],
                           oracle_linkers["systemIdentity"]]
    else:
        fail(f"{field} has an unknown build channel {channel}")
    if argv != expected:
        fail(f"{field} does not match the closed {channel} command template")
    expected_inputs.sort(key=lambda item: (item["root"], item["path"].encode("utf-8")))
    if command["inputs"] != expected_inputs:
        fail(f"{field} does not retain the complete {channel} input set")


def schema9_check_ck_environment(command, evidence_root, report, field):
    entries = {entry["name"]: entry for entry in command["environment"]}
    expected = {
        "CKC_CANDIDATE_COMPILER": [report["candidateBinary"]],
        "CKC_CLANG_ORACLE": [report["toolchain"]["clangBinary"]],
        "CKC_LLVM_PREFIX": sorted([
            report["toolchain"]["componentManifest"], report["toolchain"]["clangBinary"],
            report["toolchain"]["clangProfileRuntime"],
        ], key=lambda item: (item["root"], item["path"].encode("utf-8"))),
        "CKC_V013_REPLAY_BUNDLE": sorted([
            report["v013ReplayBundle"][key]
            for key in ["manifest", "compiler", "archive", "schemaEight", "checker"]
        ], key=lambda item: (item["root"], item["path"].encode("utf-8"))),
    }
    if set(entries) != {*expected, "XDG_CACHE_HOME"}:
        fail(f"{field} does not contain the exact Linux CK environment")
    for name, references in expected.items():
        if entries[name]["references"] != references:
            fail(f"{field} {name} retained-reference mapping mismatch")
    cache = pathlib.Path(entries["XDG_CACHE_HOME"]["value"])
    try:
        cache.resolve().relative_to(evidence_root.resolve())
    except ValueError:
        fail(f"{field} cache base is outside the evidence directory")
    for name in ["CKC_CANDIDATE_COMPILER", "CKC_CLANG_ORACLE"]:
        original = pathlib.Path(entries[name]["value"])
        if (not original.is_file() or original.is_symlink()
                or file_digest(original) != entries[name]["references"][0]["sha256"]):
            fail(f"{field} {name} live path differs from retained identity")
    prefix = pathlib.Path(entries["CKC_LLVM_PREFIX"]["value"])
    component = prefix / "share/ckc/llvm-build.toml"
    if not component.is_file() or file_digest(component) != report["toolchain"]["componentManifest"]["sha256"]:
        fail(f"{field} CKC_LLVM_PREFIX component identity mismatch")
    replay = pathlib.Path(entries["CKC_V013_REPLAY_BUNDLE"]["value"])
    expected_names = {
        "ckc-v013": report["v013ReplayBundle"]["compiler"],
        "ckc-v013-distribution.tar.gz": report["v013ReplayBundle"]["archive"],
        "check-native-performance-v013.py": report["v013ReplayBundle"]["checker"],
    }
    for name, identity in expected_names.items():
        original = replay / name
        if not original.is_file() or file_digest(original) != identity["sha256"]:
            fail(f"{field} CKC_V013_REPLAY_BUNDLE {name} identity mismatch")


def schema9_order(candidate_sha, protocol, split, case, phase, row, channels):
    material = (b"CK-V014-PERF-ORDER\0" + schema9_text(candidate_sha)
                + schema9_text(protocol) + schema9_text(split) + schema9_text(case)
                + phase.to_bytes(1, "big") + row.to_bytes(4, "big"))
    rotation = int.from_bytes(hashlib.sha256(material).digest()[:8], "big") % len(channels)
    return channels[rotation:] + channels[:rotation]


def schema9_receipt(value, field, iterations, digest):
    exact_keys(value, {"elapsedNs", "iterations", "completed", "correctnessDigest"}, field)
    schema9_u64(value["elapsedNs"], f"{field}.elapsedNs", positive_value=True)
    if value["iterations"] != iterations or value["completed"] != iterations:
        fail(f"{field} iteration/completion mismatch")
    if value["correctnessDigest"] != digest:
        fail(f"{field} correctness digest mismatch")


def schema9_calibration(value, field, channel, digest):
    exact_keys(value, {"channel", "attempts", "selectedIterationsPerCall", "confirmation"}, field)
    if value["channel"] != channel:
        fail(f"{field} calibration channel mismatch")
    selected = schema9_u64(value["selectedIterationsPerCall"],
                           f"{field}.selectedIterationsPerCall", positive_value=True)
    attempts = value["attempts"]
    if not isinstance(attempts, list) or not 1 <= len(attempts) <= 32:
        fail(f"{field} attempts must contain one through 32 entries")
    iterations = 1
    for index, receipt in enumerate(attempts):
        schema9_receipt(receipt, f"{field}.attempts[{index}]", iterations, digest)
        if index + 1 < len(attempts) and receipt["elapsedNs"] >= 50_000_000:
            fail(f"{field} continued after reaching the calibration floor")
        iterations *= 2
    if attempts[-1]["elapsedNs"] < 50_000_000 or selected != attempts[-1]["iterations"]:
        fail(f"{field} did not stop at the first qualifying calibration attempt")
    schema9_receipt(value["confirmation"], f"{field}.confirmation", selected, digest)
    return selected


def schema9_case_record(value, evidence_root, field, *, case, channels, protocol, split,
                        candidate_sha, source, input_file, decision, tuned_artifacts,
                        expected_digest, eligible=False):
    keys = {
        "case", "source", "input", "decisionDigest", "correctnessDigest",
        "correctnessDigests", "artifacts", "buildCommands", "calibration",
        "warmupOrder", "sampleOrder", "warmupReceipts", "callReceipts", "callsNs",
        "samplesNs", "mediansNs",
    }
    if eligible:
        keys.add("eligible")
    exact_keys(value, keys, field)
    if value["case"] != case or value["source"] != source or value["input"] != input_file:
        fail(f"{field} case/source/input foreign key mismatch")
    if eligible and value["eligible"] is not True:
        fail(f"{field} eligible must be true")
    if value["decisionDigest"] != decision["decisionDigest"]:
        fail(f"{field} decision foreign key mismatch")
    if value["correctnessDigest"] != expected_digest:
        fail(f"{field} expected correctness digest mismatch")
    for key in ["correctnessDigests", "artifacts", "buildCommands", "warmupReceipts",
                "callReceipts", "callsNs", "samplesNs", "mediansNs"]:
        exact_keys(value[key], set(channels), f"{field}.{key}")
    if any(value["correctnessDigests"][channel] != expected_digest for channel in channels):
        fail(f"{field} differential correctness mismatch")
    calibration_channel = {
        "release-held-out": "v014Ordinary", "validation": "v013Ordinary",
        "domain-release-held-out": "genericC",
    }[split]
    iterations = schema9_calibration(value["calibration"], f"{field}.calibration",
                                     calibration_channel, expected_digest)
    for phase, key, rows in [(1, "warmupOrder", 3), (2, "sampleOrder", 20)]:
        orders = value[key]
        if not isinstance(orders, list) or len(orders) != rows:
            fail(f"{field}.{key} row count mismatch")
        for row, actual in enumerate(orders):
            expected = schema9_order(candidate_sha, protocol, split, case, phase, row, channels)
            if actual != expected:
                fail(f"{field}.{key}[{row}] rotation mismatch")
    for channel in channels:
        check_schema9_file(value["artifacts"][channel], evidence_root,
                           f"{field}.artifacts.{channel}", "evidence")
        ck_channel = channel in {"tuned", "v014Ordinary", "v013Ordinary", "v013Pgo"}
        outputs, _ = schema9_check_build(
            value["buildCommands"][channel], evidence_root,
            f"{field}.buildCommands.{channel}",
            tuned=(channel == "tuned") if ck_channel else None,
        )
        primary = next(item["file"] for item in outputs if item["role"] == "primary")
        if primary != value["artifacts"][channel]:
            fail(f"{field} primary artifact/build foreign key mismatch for {channel}")
        if channel == "tuned":
            if (value["buildCommands"][channel]["decision"] != tuned_artifacts["decision"]
                    or outputs != tuned_artifacts["outputs"]
                    or primary != next(item["file"] for item in tuned_artifacts["outputs"]
                                       if item["role"] == "primary")):
                fail(f"{field} tuned artifact foreign key mismatch")
        warm = value["warmupReceipts"][channel]
        calls = value["callReceipts"][channel]
        call_ns = value["callsNs"][channel]
        samples = value["samplesNs"][channel]
        if (not isinstance(warm, list) or len(warm) != 3
                or not isinstance(calls, list) or len(calls) != 20
                or not isinstance(call_ns, list) or len(call_ns) != 20
                or not isinstance(samples, list) or len(samples) != 20):
            fail(f"{field} sampling row count mismatch for {channel}")
        for group_name, receipts, expected_rows in [
            ("warmupReceipts", warm, 3), ("callReceipts", calls, 20),
        ]:
            for row, receipt_row in enumerate(receipts):
                if not isinstance(receipt_row, list) or len(receipt_row) != 7:
                    fail(f"{field}.{group_name}.{channel}[{row}] must have seven calls")
                for call, receipt in enumerate(receipt_row):
                    schema9_receipt(receipt,
                                    f"{field}.{group_name}.{channel}[{row}][{call}]",
                                    iterations, expected_digest)
        for row, numbers in enumerate(call_ns):
            if (not isinstance(numbers, list) or len(numbers) != 7
                    or any(type(number) is not int or number <= 0 for number in numbers)):
                fail(f"{field}.callsNs.{channel}[{row}] must have seven positive integers")
            if numbers != [receipt["elapsedNs"] for receipt in calls[row]]:
                fail(f"{field}.callsNs.{channel}[{row}] receipt mismatch")
            if samples[row] != min(numbers):
                fail(f"{field}.samplesNs.{channel}[{row}] is not the row minimum")
        median = sorted(samples)[10]
        if value["mediansNs"][channel] != median:
            fail(f"{field}.mediansNs.{channel} is not the upper median")
        if sum(median * 80 <= sample * 100 <= median * 120 for sample in samples) < 16:
            fail(f"{field}.{channel} sampling stream is unstable")


def schema9_ratio_le(numerators, denominators, num, den):
    return math.prod(value * den for value in numerators) <= math.prod(value * num for value in denominators)


def schema9_throughput_ge(candidate, baseline, num, den, *, strict=False):
    left = math.prod(value * den for value in baseline)
    right = math.prod(value * num for value in candidate)
    return left > right if strict else left >= right


def schema9_check_profiles(workload, evidence_root, table, replay_compiler, target):
    rows = workload["profiles"]
    if not isinstance(rows, list) or len(rows) != 7:
        fail("schema-9 workload profiles must contain seven rows")
    if [row.get("case") for row in rows] != sorted(SCHEMA9_CASES):
        fail("schema-9 workload profiles are not case-name sorted")
    result = {}
    for row in rows:
        exact_keys(row, {"case", "file", "compilerSource", "source", "trainingInput"},
                   "schema-9 workload profile")
        case = row["case"]
        profile_path = check_schema9_file(row["file"], evidence_root,
                                          f"schema-9 profile {case}", "evidence")
        if row["source"] != next(item for item in workload["sources"]
                                  if item["path"] == table[case]["source"]):
            fail(f"schema-9 profile {case} source foreign key mismatch")
        if row["trainingInput"] != workload["search"]:
            fail(f"schema-9 profile {case} training input mismatch")
        hash_value(row["compilerSource"], f"schema-9 profile {case} compilerSource")
        inspected = subprocess.run(
            [replay_compiler, "pgo", "inspect", profile_path, "--json"], cwd=REPO,
            text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False, env={},
        )
        if inspected.returncode:
            fail(f"schema-9 profile {case} inspection failed: {inspected.stdout[-2000:]}")
        decoded = json.loads(inspected.stdout, object_pairs_hook=strict_json_object)
        identity = decoded.get("identity")
        profile_target = identity.get("target") if isinstance(identity, dict) else None
        profile_modes = identity.get("modes") if isinstance(identity, dict) else None
        if (decoded.get("schema") != 1 or decoded.get("format") != "CKPROF01"
                or not isinstance(identity, dict)
                or identity.get("compilerPackage") != "0.13.0"
                or not isinstance(profile_target, dict)
                or profile_target.get("triple") != target
                or profile_modes != {
                    "overflowChecked": False, "boundsChecked": False,
                    "strictFloat": True, "sanitizer": False,
                    "topology": "native-library", "optimizationFamily": "o3",
                    "cpuPolicy": "native",
                }
                or decoded.get("compatibleCompilerPackage") is not True
                or type(decoded.get("completedRuns")) is not int
                or decoded["completedRuns"] < 1
                or type(decoded.get("observedSites")) is not int
                or decoded["observedSites"] < 1
                or decoded.get("incompleteObservations") is not False):
            fail(f"schema-9 profile {case} inspection identity/content mismatch")
        source_identity = identity.get("compilerSource")
        if source_identity != row["compilerSource"]:
            fail(f"schema-9 profile {case} compiler identity mismatch")
        result[case] = row
    return result


def schema9_check_decisions(report, evidence_root, candidate_path):
    decisions = report["tuningDecisions"]
    artifacts = report["tuningArtifacts"]
    if (not isinstance(decisions, list) or len(decisions) != 7
            or [row.get("case") for row in decisions] != sorted(SCHEMA9_CASES)):
        fail("schema-9 tuningDecisions must contain seven sorted rows")
    if (not isinstance(artifacts, list) or len(artifacts) != 7
            or [row.get("case") for row in artifacts] != sorted(SCHEMA9_CASES)):
        fail("schema-9 tuningArtifacts must contain seven sorted rows")
    decision_map, artifact_map, count_map = {}, {}, {}
    for row in decisions:
        exact_keys(row, {
            "case", "file", "decisionDigest", "choiceIdentityDigest", "selectionReason",
            "planDigest", "objectGraphDigest", "linkRecipeDigest", "certificateDigest",
            "outputRecords",
        }, "schema-9 tuning decision")
        case = row["case"]
        path = check_schema9_file(row["file"], evidence_root,
                                  f"schema-9 tuning decision {case}", "evidence")
        summary, counts = schema9_inspect_decision(candidate_path, path, case)
        if {key: row[key] for key in summary} != summary:
            fail(f"schema-9 tuning decision {case} disagrees with decoded decision")
        for key in ["decisionDigest", "choiceIdentityDigest", "planDigest",
                    "objectGraphDigest", "linkRecipeDigest"]:
            hash_value(row[key], f"schema-9 tuning decision {case} {key}")
        if row["certificateDigest"] is not None:
            hash_value(row["certificateDigest"], f"schema-9 tuning decision {case} certificate")
        decision_map[case], count_map[case] = row, counts
    for row in artifacts:
        exact_keys(row, {"case", "decision", "outputs"}, "schema-9 tuning artifact")
        case = row["case"]
        if row["decision"] != decision_map[case]["file"]:
            fail(f"schema-9 tuning artifact {case} decision foreign key mismatch")
        outputs = schema9_check_output_set(row["outputs"], evidence_root,
                                           f"schema-9 tuning artifact {case}")
        records = {record["role"]: record for record in decision_map[case]["outputRecords"]}
        if set(records) != {output["role"] for output in outputs}:
            fail(f"schema-9 tuning artifact {case} output role mismatch")
        for output in outputs:
            record, file = records[output["role"]], output["file"]
            exact_keys(record, {"role", "logicalName", "bytes", "sha256"},
                       f"schema-9 tuning decision {case} output record")
            if (record["logicalName"] != pathlib.PurePosixPath(file["path"]).name
                    or record["bytes"] != file["bytes"] or record["sha256"] != file["sha256"]):
                fail(f"schema-9 tuning artifact {case} output identity mismatch")
        artifact_map[case] = row
    return decision_map, artifact_map, count_map


def schema9_check_cache(value, evidence_root, field, *, current=False):
    exact_keys(value, {"namespace", "files", "digest"}, field)
    relative = schema9_relative(value["namespace"], f"{field}.namespace")
    namespace = evidence_root.joinpath(*relative.parts)
    if namespace.is_symlink() or not namespace.is_dir():
        fail(f"{field} namespace is not a real directory")
    files = schema9_sorted_files(value["files"], evidence_root, f"{field}.files", "evidence")
    if current:
        actual = []
        for entry in namespace.rglob("*"):
            if entry.is_symlink() or (not entry.is_dir() and not entry.is_file()):
                fail(f"{field} namespace contains an unsafe entry")
            if entry.is_file():
                actual.append(entry.relative_to(evidence_root).as_posix())
        if sorted(actual) != [item["path"] for item in files]:
            fail(f"{field} cache snapshot is not complete")
    digest = schema9_digest(
        b"CK-V014-CACHE-SNAPSHOT\0", schema9_text(value["namespace"]),
        schema9_list([schema9_file_value(item) for item in files]),
    )
    if value["digest"] != digest:
        fail(f"{field} digest mismatch")


def schema9_check_supervisor(file, evidence_root, command_digest, field):
    path = check_schema9_file(file, evidence_root, field, "evidence")
    lines = path.read_text(encoding="utf-8").splitlines()
    if len(lines) != 3 or lines[0] != "CK-TUNE-SUPERVISOR\t1":
        fail(f"{field} supervisor framing mismatch")
    start = lines[1].split("\t")
    wait = lines[2].split("\t")
    if (len(start) != 3 or start[:2] != ["start", command_digest]
            or len(wait) != 4 or wait[0] != "wait4"):
        fail(f"{field} supervisor record mismatch")
    begin = schema9_u64(int(start[2]), f"{field}.start")
    end = schema9_u64(int(wait[1]), f"{field}.end")
    status = schema9_u64(int(wait[2]), f"{field}.status")
    rss_kib = schema9_u64(int(wait[3]), f"{field}.rss", positive_value=True)
    if status != 0 or end < begin:
        fail(f"{field} supervisor process did not complete successfully")
    return max(1, (end - begin + 999_999) // 1_000_000), rss_kib * 1024


def schema9_check_events(file, evidence_root, field, plan_digest, *, warm):
    path = check_schema9_file(file, evidence_root, field, "evidence")
    lines = path.read_text(encoding="utf-8").splitlines()
    if not lines or lines[0] != "CK-TUNE-EVENTS\t1":
        fail(f"{field} event framing mismatch")
    allowed = {"compile-attempt", "measurement-evaluation", "cache-hit", "cache-miss", "publication"}
    counts = {name: 0 for name in allowed}
    kinds = []
    for ordinal, line in enumerate(lines[1:]):
        fields = line.split("\t")
        if len(fields) != 6 or fields[0] != str(ordinal) or fields[1] not in allowed:
            fail(f"{field} malformed event row")
        if fields[2] not in {"-", plan_digest} or fields[3] not in {"-", "search"} or fields[4] != "-":
            fail(f"{field} event provenance mismatch")
        schema9_u64(int(fields[5]), f"{field}.calls")
        counts[fields[1]] += 1
        kinds.append(fields[1])
    if counts["publication"] != 1:
        fail(f"{field} must contain one publication")
    if warm:
        if counts["cache-hit"] != 1 or any(counts[name] for name in
                                            ["cache-miss", "compile-attempt", "measurement-evaluation"]):
            fail(f"{field} is not an exact warm reuse event stream")
        if kinds != ["cache-hit", "publication"]:
            fail(f"{field} warm event order mismatch")
    elif counts["cache-miss"] != 1 or counts["cache-hit"]:
        fail(f"{field} is not a cold event stream")
    elif kinds != (["cache-miss"] + ["compile-attempt"] * counts["compile-attempt"]
                   + ["measurement-evaluation"] * counts["measurement-evaluation"]
                   + ["publication"]):
        fail(f"{field} cold event order mismatch")
    return counts


def schema9_check_tune_run(value, evidence_root, field, candidate_path, *, warm):
    exact_keys(value, {
        "decision", "outputs", "decisionDigest", "choiceIdentityDigest", "planDigest",
        "objectGraphDigest", "linkRecipeDigest", "outputContentDigest", "build",
        "cacheBefore", "cacheAfter", "eventLog", "eventDigest", "supervisorLog",
        "supervisorDigest", "compiledCandidates", "measuredCandidates", "wallMs",
        "peakRssBytes",
    }, field)
    decision_path = check_schema9_file(value["decision"], evidence_root,
                                       f"{field}.decision", "evidence")
    outputs = schema9_check_output_set(value["outputs"], evidence_root, f"{field}.outputs")
    summary, decision_counts = schema9_inspect_decision(candidate_path, decision_path, field)
    for key in ["decisionDigest", "choiceIdentityDigest", "planDigest", "objectGraphDigest",
                "linkRecipeDigest"]:
        if value[key] != summary[key]:
            fail(f"{field} {key} disagrees with the retained decision")
    records = {record["role"]: record for record in summary["outputRecords"]}
    for output in outputs:
        file = output["file"]
        record = records.get(output["role"])
        if (record is None or record["logicalName"] != pathlib.PurePosixPath(file["path"]).name
                or record["bytes"] != file["bytes"] or record["sha256"] != file["sha256"]):
            fail(f"{field} output disagrees with the retained decision")
    if value["outputContentDigest"] != schema9_output_digest(outputs):
        fail(f"{field} outputContentDigest mismatch")
    build_outputs, build_digest = schema9_check_build(value["build"], evidence_root,
                                                      f"{field}.build", tuned=True)
    if value["build"]["decision"] != value["decision"] or build_outputs != outputs:
        fail(f"{field} build/decision/output foreign key mismatch")
    schema9_check_cache(value["cacheBefore"], evidence_root, f"{field}.cacheBefore")
    schema9_check_cache(value["cacheAfter"], evidence_root, f"{field}.cacheAfter", current=True)
    counts = schema9_check_events(value["eventLog"], evidence_root, f"{field}.eventLog",
                                  value["planDigest"], warm=warm)
    expected_event_digest = schema9_digest(
        b"CK-V014-TUNE-EVENTS\0", schema9_file_value(value["eventLog"]))
    if value["eventDigest"] != expected_event_digest:
        fail(f"{field} eventDigest mismatch")
    wall, rss = schema9_check_supervisor(value["supervisorLog"], evidence_root,
                                         build_digest, f"{field}.supervisorLog")
    expected_supervisor = schema9_digest(
        b"CK-V014-TUNE-SUPERVISOR\0", schema9_file_value(value["supervisorLog"]))
    if (value["supervisorDigest"] != expected_supervisor or value["wallMs"] != wall
            or value["peakRssBytes"] != rss):
        fail(f"{field} supervisor-derived values mismatch")
    expected_compiled = 0 if warm else decision_counts["compiled"]
    expected_measured = 0 if warm else decision_counts["measured"]
    if (value["compiledCandidates"] != expected_compiled
            or value["measuredCandidates"] != expected_measured
            or counts["compile-attempt"] != expected_compiled
            or counts["measurement-evaluation"] != expected_measured):
        fail(f"{field} event/decision candidate counts mismatch")
    return summary, decision_counts


def schema9_check_compile_rows(rows, evidence_root, field, report, left, right):
    if (not isinstance(rows, list) or len(rows) != 7
            or [row.get("case") for row in rows] != sorted(SCHEMA9_CASES)):
        fail(f"{field} must contain seven case-name-sorted rows")
    medians = {left: [], right: []}
    for row in rows:
        case = row["case"]
        exact_keys(row, {"case", "warmupOrder", "sampleOrder", "samplesNs", "mediansNs", "commands"},
                   f"{field}.{case}")
        channels = [left, right]
        expected_orders = [channels[index % 2:] + channels[:index % 2] for index in range(18)]
        if row["warmupOrder"] != expected_orders[:3] or row["sampleOrder"] != expected_orders[3:]:
            fail(f"{field}.{case} alternating order mismatch")
        for key in ["samplesNs", "mediansNs", "commands"]:
            exact_keys(row[key], set(channels), f"{field}.{case}.{key}")
        for channel in channels:
            receipts = row["commands"][channel]
            if not isinstance(receipts, list) or len(receipts) != 18:
                fail(f"{field}.{case}.{channel} must retain 18 timed commands")
            elapsed = []
            output_paths, cache_paths = set(), set()
            for index, receipt in enumerate(receipts):
                exact_keys(receipt, {"command", "elapsedNs"},
                           f"{field}.{case}.{channel}.commands[{index}]")
                value = schema9_u64(receipt["elapsedNs"],
                                    f"{field}.{case}.{channel}.elapsedNs", positive_value=True)
                command = receipt["command"]
                schema9_check_command(command, evidence_root,
                                      f"{field}.{case}.{channel}.commands[{index}]")
                expected_executable = (report["v013ReplayBundle"]["compiler"]
                                       if channel == "v013Ordinary"
                                       else report["candidateBinary"])
                if command["executable"] != expected_executable:
                    fail(f"{field}.{case}.{channel} executable mismatch")
                schema9_check_ck_environment(
                    command, evidence_root, report,
                    f"{field}.{case}.{channel}.commands[{index}].environment")
                if "--out" not in command["argv"]:
                    fail(f"{field}.{case}.{channel} command omits --out")
                output_path = command["argv"][command["argv"].index("--out") + 1]
                try:
                    (REPO / output_path).resolve().relative_to(evidence_root.resolve())
                except ValueError:
                    fail(f"{field}.{case}.{channel} output is outside the evidence directory")
                cache_entries = [entry["value"] for entry in command["environment"]
                                 if entry["name"] in {"XDG_CACHE_HOME", "HOME", "LOCALAPPDATA"}]
                if len(cache_entries) != 1:
                    fail(f"{field}.{case}.{channel} command lacks one cache base")
                if output_path in output_paths or cache_entries[0] in cache_paths:
                    fail(f"{field}.{case}.{channel} reuses timed output/cache state")
                output_paths.add(output_path)
                cache_paths.add(cache_entries[0])
                if index >= 3:
                    elapsed.append(value)
                argv = command["argv"]
                table = schema9_case_table()
                source = next(item for item in report["workload"]["sources"]
                              if item["path"] == table[case]["source"])
                executable = schema9_file_argument(command["executable"], evidence_root)
                expected = [
                    executable, "build", schema9_file_argument(source, evidence_root),
                    "--out", output_path, "--kind", "dynamic", "--cpu", "native", "-O3",
                    "--overflow", "unchecked", "--bounds", "unchecked",
                ]
                if source not in command["inputs"]:
                    fail(f"{field}.{case}.{channel} compile command omits source input")
                if channel == "tuneUse":
                    decision = next(item["file"] for item in report["tuningDecisions"]
                                    if item["case"] == case)
                    expected.extend(["--tune-use", schema9_file_argument(decision, evidence_root)])
                    if decision not in command["inputs"]:
                        fail(f"{field}.{case} tuneUse command omits its decision input")
                if argv != expected:
                    fail(f"{field}.{case}.{channel} compile command template mismatch")
            if row["samplesNs"][channel] != elapsed:
                fail(f"{field}.{case}.{channel} samples do not equal retained receipts")
            if row["mediansNs"][channel] != sorted(elapsed)[7]:
                fail(f"{field}.{case}.{channel} compile median mismatch")
            medians[channel].append(row["mediansNs"][channel])
    return medians


def schema9_check_archive(value, evidence_root, report):
    exact_keys(value, {"candidate", "v013Replay", "producer", "command", "members"},
               "schema-9 archiveSize")
    candidate = check_schema9_file(value["candidate"], evidence_root,
                                   "schema-9 candidate archive", "evidence")
    if value["v013Replay"] != report["v013ReplayBundle"]["archive"]:
        fail("schema-9 archive replay foreign key mismatch")
    check_schema9_file(value["producer"], evidence_root, "schema-9 archive producer", "repository")
    if value["producer"]["path"] != "scripts/package-v014-performance-archive.py":
        fail("schema-9 archive producer path mismatch")
    schema9_check_command(value["command"], evidence_root, "schema-9 archive command")
    if value["command"]["executable"] != value["producer"] or value["command"]["environment"]:
        fail("schema-9 archive command producer/environment mismatch")
    expected_members = [
        ("ckc-v0.14/LICENSE", 0o644, next(item for item in value["command"]["inputs"]
                                          if item["path"] == "LICENSE")),
        ("ckc-v0.14/THIRD_PARTY_NOTICES.md", 0o644,
         next(item for item in value["command"]["inputs"]
              if item["path"] == "THIRD_PARTY_NOTICES.md")),
        ("ckc-v0.14/ckc", 0o755, report["candidateBinary"]),
    ]
    expected_inputs = sorted([item[2] for item in expected_members],
                             key=lambda item: (item["root"], item["path"].encode("utf-8")))
    if value["command"]["inputs"] != expected_inputs:
        fail("schema-9 archive command input set mismatch")
    expected_argv = [
        schema9_file_argument(value["producer"], evidence_root), "--compiler",
        schema9_file_argument(report["candidateBinary"], evidence_root), "--license", "LICENSE",
        "--notices", "THIRD_PARTY_NOTICES.md", "--out",
        schema9_file_argument(value["candidate"], evidence_root),
    ]
    if value["command"]["argv"] != expected_argv:
        fail("schema-9 archive command argv mismatch")
    if not isinstance(value["members"], list) or len(value["members"]) != 3:
        fail("schema-9 archive member cardinality mismatch")
    for actual, (name, mode, file) in zip(value["members"], expected_members, strict=True):
        exact_keys(actual, {"path", "mode", "file"}, "schema-9 archive member")
        if actual != {"path": name, "mode": mode, "file": file}:
            fail("schema-9 archive member identity mismatch")
    raw = candidate.read_bytes()
    if raw[:4] != b"\x1f\x8b\x08\x00" or raw[4:8] != b"\0\0\0\0":
        fail("schema-9 candidate archive gzip header is not deterministic")
    with tarfile.open(fileobj=io.BytesIO(gzip.decompress(raw)), mode="r:") as archive:
        records = archive.getmembers()
        if [record.name for record in records] != [item[0] for item in expected_members]:
            fail("schema-9 candidate archive record order mismatch")
        for record, (_, mode, file) in zip(records, expected_members, strict=True):
            if (not record.isfile() or record.mode != mode or record.mtime != 0
                    or record.uid or record.gid or record.uname or record.gname or record.pax_headers):
                fail("schema-9 candidate archive metadata mismatch")
            stream = archive.extractfile(record)
            source = (REPO if file["root"] == "repository" else evidence_root) / file["path"]
            if stream is None or stream.read() != source.read_bytes():
                fail("schema-9 candidate archive content mismatch")
    return value["candidate"]["bytes"], value["v013Replay"]["bytes"]


def schema9_check_tree(files, evidence_root, prefix, field):
    schema9_sorted_files(files, evidence_root, field, "evidence")
    prefix_path = evidence_root / prefix
    actual = []
    for entry in prefix_path.rglob("*"):
        if entry.is_symlink() or (not entry.is_dir() and not entry.is_file()):
            fail(f"{field} contains an unsafe entry")
        if entry.is_file():
            actual.append(entry.relative_to(evidence_root).as_posix())
    if sorted(actual) != [item["path"] for item in files]:
        fail(f"{field} is not the complete retained tree")


def schema9_check_evidence_closure(report, evidence_root):
    identities = {}

    def visit(value):
        if isinstance(value, dict):
            if set(value) == {"root", "path", "bytes", "sha256"}:
                if value["root"] == "evidence":
                    prior = identities.setdefault(value["path"], value)
                    if prior != value:
                        fail("schema-9 evidence path has conflicting identities")
                return
            for child in value.values():
                visit(child)
        elif isinstance(value, list):
            for child in value:
                visit(child)

    visit(report)
    actual = []
    for entry in evidence_root.rglob("*"):
        if entry.is_symlink() or (not entry.is_dir() and not entry.is_file()):
            fail("schema-9 evidence closure contains an unsafe entry")
        if entry.is_file():
            actual.append(entry.relative_to(evidence_root).as_posix())
    if sorted(actual) != sorted(identities):
        missing = sorted(set(identities) - set(actual))
        unknown = sorted(set(actual) - set(identities))
        fail(f"schema-9 evidence closure mismatch: missing={missing}, unknown={unknown}")


def schema9_check_replay(report, evidence_root):
    replay = report["v013ReplayBundle"]
    exact_keys(replay, {"commit", "manifest", "compiler", "archive", "schemaEight", "checker",
                        "evidenceFiles"}, "schema-9 v013ReplayBundle")
    if replay["commit"] != report["v013ReplayCommit"]:
        fail("schema-9 v0.13 replay commit mismatch")
    for key in ["manifest", "compiler", "archive", "schemaEight", "checker"]:
        check_schema9_file(replay[key], evidence_root, f"schema-9 replay {key}", "evidence")
    prefix = pathlib.PurePosixPath(replay["manifest"]["path"]).parts[0]
    schema9_check_tree(replay["evidenceFiles"], evidence_root, prefix,
                       "schema-9 v0.13 replay files")
    retained_manifest = evidence_root / replay["manifest"]["path"]
    if retained_manifest.read_bytes() != V013_REPLAY_MANIFEST.read_bytes():
        fail("schema-9 retained v0.13 manifest differs from the pinned candidate recipe")
    manifest = tomllib.loads(retained_manifest.read_text(encoding="utf-8"))
    if manifest.get("commit") != replay["commit"] or manifest.get("version") != "0.13.0":
        fail("schema-9 retained v0.13 manifest identity mismatch")
    source_checker = subprocess.run(
        ["git", "show", f"{replay['commit']}:scripts/check-native-performance.py"], cwd=REPO,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False,
    )
    if source_checker.returncode or source_checker.stdout != (evidence_root / replay["checker"]["path"]).read_bytes():
        fail("schema-9 retained v0.13 checker differs from its pinned commit")
    with tempfile.TemporaryDirectory(prefix="ckc-v013-schema9-check-") as temporary:
        checkout = pathlib.Path(temporary) / "checkout"
        clone = subprocess.run(
            ["git", "clone", "--quiet", "--shared", str(REPO), str(checkout)],
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, check=False,
        )
        if clone.returncode:
            fail(f"schema-9 v0.13 checker checkout failed: {clone.stdout[-2000:]}")
        checkout_result = subprocess.run(
            ["git", "checkout", "--quiet", "--detach", replay["commit"]], cwd=checkout,
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, check=False,
        )
        if checkout_result.returncode:
            fail(f"schema-9 v0.13 checker revision failed: {checkout_result.stdout[-2000:]}")
        historical = subprocess.run(
            [sys.executable, "-B", checkout / "scripts/check-native-performance.py",
             evidence_root / replay["schemaEight"]["path"]], cwd=checkout,
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, check=False,
        )
        if historical.returncode:
            fail(f"schema-9 retained v0.13 historical evidence failed: {historical.stdout[-3000:]}")
    historical_report_path = evidence_root / replay["schemaEight"]["path"]
    historical_report = json.loads(historical_report_path.read_text(encoding="utf-8"))
    if (historical_report.get("candidateVersion") != "0.13.0"
            or historical_report.get("candidateSha") != replay["commit"]):
        fail("schema-9 historical report is not the pinned v0.13 candidate")
    historical_directory = historical_report.get("evidenceDirectory")
    historical_binary = historical_report.get("candidateBinary")
    if (not isinstance(historical_directory, str) or not isinstance(historical_binary, dict)
            or set(historical_binary) != {"file", "bytes", "sha256"}):
        fail("schema-9 historical candidate binary identity is malformed")
    copied_binary = historical_report_path.parent / historical_directory / historical_binary["file"]
    verify_file(copied_binary, historical_binary["bytes"], historical_binary["sha256"],
                "schema-9 historical candidate binary")
    if (historical_binary["bytes"] != replay["compiler"]["bytes"]
            or historical_binary["sha256"] != replay["compiler"]["sha256"]):
        fail("schema-9 replay compiler differs from the accepted historical candidate")
    with tarfile.open(evidence_root / replay["archive"]["path"], mode="r:gz") as archive:
        compiler_members = [member for member in archive.getmembers()
                            if member.name == "ckc-v0.13/ckc"]
        if len(compiler_members) != 1 or not compiler_members[0].isfile():
            fail("schema-9 v0.13 archive lacks its unique compiler member")
        stream = archive.extractfile(compiler_members[0])
        if stream is None or stream.read() != (evidence_root / replay["compiler"]["path"]).read_bytes():
            fail("schema-9 v0.13 archive compiler differs from the replay compiler")
    cumulative = report["cumulativeSchemaEight"]
    exact_keys(cumulative, {"report", "files"}, "schema-9 cumulativeSchemaEight")
    cumulative_path = check_schema9_file(cumulative["report"], evidence_root,
                                         "schema-9 cumulative schema-8 report", "evidence")
    cumulative_prefix = pathlib.PurePosixPath(cumulative["report"]["path"]).parts[0]
    schema9_check_tree(cumulative["files"], evidence_root, cumulative_prefix,
                       "schema-9 cumulative schema-8 files")
    check(cumulative_path, DEFAULT_BASELINE_MANIFEST, schema=8,
          compat_candidate="0.14.0", candidate_sha=report["candidateSha"])
    return evidence_root / replay["compiler"]["path"]


def schema9_check_determinism_and_resources(report, evidence_root, candidate_path,
                                            decision_map, artifact_map, count_map):
    rows = report["determinism"]
    sessions = report["resourceUse"]["sessions"]
    if (not isinstance(rows, list) or len(rows) != 7
            or [row.get("case") for row in rows] != sorted(SCHEMA9_CASES)):
        fail("schema-9 determinism must contain seven sorted rows")
    if (not isinstance(sessions, list) or len(sessions) != 7
            or [row.get("case") for row in sessions] != sorted(SCHEMA9_CASES)):
        fail("schema-9 resourceUse sessions must contain seven sorted rows")
    session_map = {row["case"]: row for row in sessions}
    namespaces = set()
    for row in rows:
        exact_keys(row, {"case", "coldOne", "coldTwo", "warm"}, "schema-9 determinism row")
        case = row["case"]
        cold_one, cold_two, warm = row["coldOne"], row["coldTwo"], row["warm"]
        _, cold_counts = schema9_check_tune_run(
            cold_one, evidence_root, f"schema-9 determinism {case}.coldOne",
            candidate_path, warm=False)
        schema9_check_tune_run(cold_two, evidence_root,
                               f"schema-9 determinism {case}.coldTwo", candidate_path,
                               warm=False)
        schema9_check_tune_run(warm, evidence_root, f"schema-9 determinism {case}.warm",
                               candidate_path, warm=True)
        if cold_one["cacheBefore"]["files"] or cold_two["cacheBefore"]["files"]:
            fail(f"schema-9 determinism {case} cold cache was not empty")
        cold_names = [cold_one["cacheBefore"]["namespace"], cold_two["cacheBefore"]["namespace"]]
        if cold_names[0] == cold_names[1] or any(name in namespaces for name in cold_names):
            fail(f"schema-9 determinism {case} cold cache namespace was reused")
        namespaces.update(cold_names)
        if (warm["cacheBefore"] != cold_one["cacheAfter"]
                or warm["cacheAfter"] != warm["cacheBefore"]):
            fail(f"schema-9 determinism {case} warm cache continuity mismatch")
        cold_fields = ["choiceIdentityDigest", "planDigest", "objectGraphDigest",
                       "linkRecipeDigest", "outputContentDigest"]
        if any(cold_one[key] != cold_two[key] for key in cold_fields):
            fail(f"schema-9 determinism {case} independent cold results disagree")
        if any(cold_one[key] != warm[key] for key in ["decisionDigest", *cold_fields]):
            fail(f"schema-9 determinism {case} warm exact reuse disagrees")
        if (cold_one["decision"] != artifact_map[case]["decision"]
                or cold_one["outputs"] != artifact_map[case]["outputs"]
                or cold_one["decision"] != decision_map[case]["file"]):
            fail(f"schema-9 determinism {case} canonical cold-one foreign key mismatch")
        for role in {item["role"] for item in cold_one["outputs"]}:
            first = next(item["file"] for item in cold_one["outputs"] if item["role"] == role)
            reused = next(item["file"] for item in warm["outputs"] if item["role"] == role)
            if (first["bytes"], first["sha256"]) != (reused["bytes"], reused["sha256"]):
                fail(f"schema-9 determinism {case} warm output bytes disagree")
        session = session_map[case]
        exact_keys(session, {
            "case", "decision", "decisionDigest", "ordinaryBuild", "ordinarySupervisorLog",
            "ordinarySupervisorDigest", "budget", "wallMs", "peakRssBytes",
            "ordinaryPeakRssBytes", "expansions", "compileAttempts", "measuredFinalists",
            "validationEntrants", "cacheBytes",
        }, f"schema-9 resource session {case}")
        if (session["decision"] != decision_map[case]["file"]
                or session["decisionDigest"] != decision_map[case]["decisionDigest"]
                or session["budget"] != "standard"
                or session["wallMs"] != cold_one["wallMs"]
                or session["peakRssBytes"] != cold_one["peakRssBytes"]
                or session["expansions"] != cold_counts["expansions"]
                or session["compileAttempts"] != cold_one["compiledCandidates"]
                or session["measuredFinalists"] != cold_one["measuredCandidates"]
                or session["validationEntrants"] != cold_counts["validationEntrants"]
                or session["cacheBytes"] != sum(item["bytes"] for item in cold_one["cacheAfter"]["files"])):
            fail(f"schema-9 resource session {case} derived values mismatch")
        _, ordinary_digest = schema9_check_build(
            session["ordinaryBuild"], evidence_root,
            f"schema-9 resource session {case}.ordinaryBuild", tuned=False)
        ordinary_wall, ordinary_rss = schema9_check_supervisor(
            session["ordinarySupervisorLog"], evidence_root, ordinary_digest,
            f"schema-9 resource session {case}.ordinarySupervisorLog")
        if ordinary_wall <= 0 or session["ordinaryPeakRssBytes"] != ordinary_rss:
            fail(f"schema-9 resource session {case} ordinary supervisor mismatch")
        digest = schema9_digest(
            b"CK-V014-TUNE-SUPERVISOR\0",
            schema9_file_value(session["ordinarySupervisorLog"]))
        if session["ordinarySupervisorDigest"] != digest:
            fail(f"schema-9 resource session {case} ordinary supervisor digest mismatch")
    return session_map


def schema9_check_artifact_sizes(report, evidence_root, artifact_map):
    rows = report["artifactSize"]
    if (not isinstance(rows, list) or len(rows) != 7
            or [row.get("case") for row in rows] != sorted(SCHEMA9_CASES)):
        fail("schema-9 artifactSize must contain seven sorted rows")
    values = []
    for row in rows:
        exact_keys(row, {"case", "tunedPrimary", "baselinePrimary", "baselineBuild"},
                   "schema-9 artifact size row")
        case = row["case"]
        check_schema9_file(row["tunedPrimary"], evidence_root,
                           f"schema-9 artifact {case} tuned", "evidence")
        check_schema9_file(row["baselinePrimary"], evidence_root,
                           f"schema-9 artifact {case} baseline", "evidence")
        outputs, _ = schema9_check_build(row["baselineBuild"], evidence_root,
                                         f"schema-9 artifact {case}.baselineBuild", tuned=False)
        primary = next(item["file"] for item in outputs if item["role"] == "primary")
        tuned = next(item["file"] for item in artifact_map[case]["outputs"]
                     if item["role"] == "primary")
        if row["baselinePrimary"] != primary or row["tunedPrimary"] != tuned:
            fail(f"schema-9 artifact {case} foreign key mismatch")
        values.append((row["tunedPrimary"]["bytes"], row["baselinePrimary"]["bytes"]))
    return values, {row["case"]: row for row in rows}


def schema9_check_toolchain(report, evidence_root):
    toolchain = report["toolchain"]
    component = evidence_root / toolchain["componentManifest"]["path"]
    manifest = tomllib.loads(component.read_text(encoding="utf-8"))
    if (manifest.get("schema") != 1 or manifest.get("version") != "22.1.8"
            or manifest.get("static_only") is not True
            or not {"core", "native", "orcjit", "nativecodegen", "lto"}.issubset(
                set(manifest.get("components", [])))):
        fail("schema-9 retained LLVM component manifest is not the pinned static build")
    candidate = evidence_root / report["candidateBinary"]["path"]
    version = subprocess.run([candidate, "--version"], cwd=REPO, text=True,
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False)
    if version.returncode or not version.stdout.startswith("ckc 0.14.0"):
        fail("schema-9 retained candidate binary identity mismatch")
    clang_name = os.environ.get("CKC_CLANG_ORACLE")
    if not clang_name:
        fail("schema-9 full checking requires CKC_CLANG_ORACLE")
    clang = pathlib.Path(clang_name).resolve()
    if file_digest(clang) != report["toolchain"]["clangBinary"]["sha256"]:
        fail("schema-9 resolved Clang differs from the retained binary")
    clang_version = subprocess.run([clang, "--version"], text=True, stdout=subprocess.PIPE,
                                   stderr=subprocess.STDOUT, check=False)
    if clang_version.returncode or "clang version 22.1.8" not in clang_version.stdout.lower():
        fail("schema-9 resolved Clang is not 22.1.8")
    resource = subprocess.run([clang, "--print-resource-dir"], text=True,
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False)
    if resource.returncode:
        fail("schema-9 Clang resource directory query failed")
    root = pathlib.Path(resource.stdout.strip())
    matches = sorted(root.glob("lib/**/libclang_rt.profile*.a"))
    matches += sorted(root.glob("lib/**/clang_rt.profile*.lib"))
    if len(matches) != 1 or file_digest(matches[0]) != report["toolchain"]["clangProfileRuntime"]["sha256"]:
        fail("schema-9 Clang profile runtime identity mismatch")
    rustc = subprocess.run(["rustup", "which", "rustc", "--toolchain", "1.90.0"],
                           text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                           check=False)
    if rustc.returncode:
        fail("schema-9 rustc 1.90.0 resolution failed")
    rustc_path = pathlib.Path(rustc.stdout.strip()).resolve()
    if file_digest(rustc_path) != report["toolchain"]["rustCompiler"]["sha256"]:
        fail("schema-9 resolved Rust compiler differs from the retained binary")
    system_linker = pathlib.Path("/usr/bin/ld").resolve(strict=True)
    if (not system_linker.is_file() or system_linker.is_symlink()
            or file_digest(system_linker) != report["toolchain"]["systemLinker"]["sha256"]):
        fail("schema-9 resolved system linker differs from the retained binary")
    return {
        "clang": str(clang), "system": str(system_linker),
        "clangIdentity": report["toolchain"]["clangBinary"],
        "systemIdentity": report["toolchain"]["systemLinker"],
    }


def check_schema9(report, path, *, schema_only=False):
    contract_only, _, _ = check_schema9_schema_only(report, path)
    if schema_only:
        if not contract_only:
            fail("--schema-only accepts only the explicit non-accepting contract fixture")
        dynamic_empty = [
            "tuningDecisions", "tuningArtifacts", "cases", "validationCases",
            "domainCases", "tuneUseCompileTime", "ordinaryCompileRegression",
            "artifactSize", "determinism",
        ]
        if any(report[key] != [] for key in dynamic_empty):
            fail("schema-9 contract fixture contains measured evidence")
        exact_keys(report["resourceUse"], {"sessions", "cacheHardLimitBytes"},
                   "schema-9 contract resourceUse")
        if report["resourceUse"]["sessions"] != [] or report["archiveSize"] != {}:
            fail("schema-9 contract fixture must not claim resource/archive evidence")
        if report["v013ReplayBundle"] != {} or report["cumulativeSchemaEight"] != {}:
            fail("schema-9 contract fixture must not claim replay evidence")
        return
    if contract_only:
        fail("contract-only schema-9 output is not performance acceptance evidence")
    if not all(report["correctness"].values()):
        fail("schema-9 correctness evidence is incomplete")
    exact_keys(report["resourceUse"], {"sessions", "cacheHardLimitBytes"},
               "schema-9 resourceUse")
    if report["resourceUse"]["cacheHardLimitBytes"] != SCHEMA9_THRESHOLDS["cacheBytesMaximum"]:
        fail("schema-9 resourceUse cache hard limit mismatch")
    evidence_root = path.parent / report["evidenceDirectory"]
    table = schema9_case_table()
    oracle_linkers = schema9_check_toolchain(report, evidence_root)
    replay_compiler = schema9_check_replay(report, evidence_root)
    candidate_path = evidence_root / report["candidateBinary"]["path"]
    profiles = schema9_check_profiles(report["workload"], evidence_root, table,
                                      replay_compiler, report["hardware"]["target"])
    decision_map, artifact_map, count_map = schema9_check_decisions(
        report, evidence_root, candidate_path)

    sources = {
        case: next(item for item in report["workload"]["sources"]
                   if item["path"] == table[case]["source"])
        for case in SCHEMA9_CASES
    }
    release_expected = {row["case"]: row["digest"]
                        for row in report["workload"]["expectedResults"]}
    case_groups = [
        ("cases", SCHEMA9_MAIN_CASES, SCHEMA9_MAIN_CHANNELS,
         "rotating-six-channel-v1", "release-held-out", report["workload"]["releaseHeldOut"],
         True),
        ("validationCases", SCHEMA9_CASES, SCHEMA9_VALIDATION_CHANNELS,
         "rotating-three-channel-v1", "validation", report["workload"]["validation"], False),
        ("domainCases", SCHEMA9_DOMAIN_CASES, SCHEMA9_DOMAIN_CHANNELS,
         "rotating-three-channel-v1", "domain-release-held-out",
         report["workload"]["releaseHeldOut"], False),
    ]
    measured = {}
    for key, expected_cases, channels, protocol, split, input_file, eligible in case_groups:
        rows = report[key]
        if (not isinstance(rows, list) or len(rows) != len(expected_cases)
                or [row.get("case") for row in rows] != sorted(expected_cases)):
            fail(f"schema-9 {key} case set/order mismatch")
        for row in rows:
            case = row["case"]
            expected_digest = (table[case]["validationDigest"] if split == "validation"
                               else release_expected[case])
            schema9_case_record(
                row, evidence_root, f"schema-9 {key}.{case}", case=case,
                channels=channels, protocol=protocol, split=split,
                candidate_sha=report["candidateSha"], source=sources[case],
                input_file=input_file, decision=decision_map[case],
                tuned_artifacts=artifact_map[case], expected_digest=expected_digest,
                eligible=eligible,
            )
            measured[(key, case)] = row

    expected_executables = {
        "tuned": report["candidateBinary"], "v014Ordinary": report["candidateBinary"],
        "v013Ordinary": report["v013ReplayBundle"]["compiler"],
        "v013Pgo": report["v013ReplayBundle"]["compiler"],
        "cSimd": report["toolchain"]["clangBinary"],
        "rustSimd": report["toolchain"]["rustCompiler"],
        "genericC": report["toolchain"]["clangBinary"],
        "genericRust": report["toolchain"]["rustCompiler"],
    }
    with (REPO / report["workload"]["oracleManifest"]["path"]).open("rb") as source:
        oracle = tomllib.load(source)
    oracle_index = {row["name"]: row["oracle_case"] for row in oracle.get("case", [])}
    manifests = {
        case: next(item for item in report["workload"]["tuneManifests"]
                   if pathlib.PurePosixPath(item["path"]).name == table[case]["manifest"])
        for case in SCHEMA9_CASES
    }
    for row in [*report["cases"], *report["validationCases"], *report["domainCases"]]:
        for channel, build in row["buildCommands"].items():
            if build["command"]["executable"] != expected_executables[channel]:
                fail(f"schema-9 {row['case']} {channel} executable foreign key mismatch")
            argv = build["command"]["argv"]
            if channel in {"tuned", "v014Ordinary", "v013Ordinary", "v013Pgo"}:
                schema9_check_ck_environment(
                    build["command"], evidence_root, report,
                    f"schema-9 {row['case']}.{channel}.environment")
            elif build["command"]["environment"]:
                fail(f"schema-9 {row['case']} {channel} oracle environment is not empty")
            if channel == "v013Pgo":
                if "--pgo-use" not in argv or profiles[row["case"]]["file"] not in build["command"]["inputs"]:
                    fail(f"schema-9 {row['case']} v013Pgo profile foreign key mismatch")
            elif "--pgo-use" in argv:
                fail(f"schema-9 {row['case']} unexpected PGO input in {channel}")
            command_source = (report["workload"]["cOracle"] if channel in {"cSimd", "genericC"}
                              else report["workload"]["rustOracle"]
                              if channel in {"rustSimd", "genericRust"}
                              else sources[row["case"]])
            schema9_check_channel_template(
                build, evidence_root, f"schema-9 {row['case']}.{channel}", channel,
                command_source,
                manifest=(report["workload"]["oracleManifest"]
                          if channel in {"cSimd", "genericC", "rustSimd", "genericRust"}
                          else manifests[row["case"]]),
                profile=profiles[row["case"]]["file"], oracle=oracle,
                case_index=oracle_index.get(row["case"]), case_name=row["case"],
                oracle_linkers=oracle_linkers,
            )

    sessions = schema9_check_determinism_and_resources(
        report, evidence_root, candidate_path, decision_map, artifact_map, count_map)
    artifact_ratios, size_map = schema9_check_artifact_sizes(report, evidence_root, artifact_map)
    for case in SCHEMA9_CASES:
        if sessions[case]["ordinaryBuild"] != size_map[case]["baselineBuild"]:
            fail(f"schema-9 {case} ordinary resource/artifact build mismatch")
    tune_medians = schema9_check_compile_rows(
        report["tuneUseCompileTime"], evidence_root, "schema-9 tuneUseCompileTime",
        report, "tuneUse", "v014Ordinary")
    ordinary_medians = schema9_check_compile_rows(
        report["ordinaryCompileRegression"], evidence_root,
        "schema-9 ordinaryCompileRegression", report,
        "v014Ordinary", "v013Ordinary")
    archive_candidate, archive_replay = schema9_check_archive(report["archiveSize"], evidence_root,
                                                              report)

    thresholds = report["recipe"]["thresholds"]
    main = report["cases"]
    tuned_times = [row["mediansNs"]["tuned"] for row in main]
    baseline_times = [min(row["mediansNs"]["v013Ordinary"], row["mediansNs"]["v013Pgo"])
                      for row in main]
    if not schema9_ratio_le(tuned_times, baseline_times,
                            thresholds["heldOutGeomeanMaximumNum"],
                            thresholds["heldOutGeomeanMaximumDen"]):
        fail("schema-9 held-out geometric performance gate failed")
    for row, baseline in zip(main, baseline_times, strict=True):
        if not schema9_ratio_le([row["mediansNs"]["tuned"]], [baseline],
                                thresholds["validationOrHeldOutMaximumNum"],
                                thresholds["validationOrHeldOutMaximumDen"]):
            fail(f"schema-9 {row['case']} held-out slowdown gate failed")
        if decision_map[row["case"]]["selectionReason"] == "tuned" and not schema9_ratio_le(
                [row["mediansNs"]["tuned"]], [baseline],
                thresholds["selectedCaseMaximumNum"], thresholds["selectedCaseMaximumDen"]):
            fail(f"schema-9 {row['case']} selected-case gain gate failed")
    for row in report["validationCases"]:
        baseline = min(row["mediansNs"]["v013Ordinary"], row["mediansNs"]["v013Pgo"])
        if not schema9_ratio_le([row["mediansNs"]["tuned"]], [baseline],
                                thresholds["validationOrHeldOutMaximumNum"],
                                thresholds["validationOrHeldOutMaximumDen"]):
            fail(f"schema-9 {row['case']} validation slowdown gate failed")
    oracle_baselines = [min(row["mediansNs"]["cSimd"], row["mediansNs"]["rustSimd"])
                        for row in main]
    if not schema9_throughput_ge(
            tuned_times, oracle_baselines, thresholds["oracleGeomeanThroughputMinimumNum"],
            thresholds["oracleGeomeanThroughputMinimumDen"]):
        fail("schema-9 explicit-SIMD oracle geometric gate failed")
    for row, baseline in zip(main, oracle_baselines, strict=True):
        if not schema9_throughput_ge(
                [row["mediansNs"]["tuned"]], [baseline],
                thresholds["oracleCaseThroughputMinimumNum"],
                thresholds["oracleCaseThroughputMinimumDen"]):
            fail(f"schema-9 {row['case']} explicit-SIMD oracle gate failed")
    domain_tuned = [row["mediansNs"]["tuned"] for row in report["domainCases"]]
    domain_baselines = [min(row["mediansNs"]["genericC"], row["mediansNs"]["genericRust"])
                        for row in report["domainCases"]]
    if not schema9_throughput_ge(
            domain_tuned, domain_baselines, thresholds["domainThroughputMinimumNum"],
            thresholds["domainThroughputMinimumDen"], strict=True):
        fail("schema-9 domain throughput gate failed")
    if any(not schema9_ratio_le([left], [right], thresholds["artifactMaximumNum"],
                                thresholds["artifactMaximumDen"])
           for left, right in artifact_ratios):
        fail("schema-9 artifact size gate failed")
    if not schema9_ratio_le([archive_candidate], [archive_replay],
                            thresholds["archiveMaximumNum"], thresholds["archiveMaximumDen"]):
        fail("schema-9 archive size gate failed")
    compile_gates = [
        (tune_medians, "tuneUse", "v014Ordinary", "tuneUseCompileGeomeanMaximumNum",
         "tuneUseCompileGeomeanMaximumDen", "tuneUseCompileCaseMaximumNum",
         "tuneUseCompileCaseMaximumDen", "tune-use"),
        (ordinary_medians, "v014Ordinary", "v013Ordinary",
         "ordinaryCompileGeomeanMaximumNum", "ordinaryCompileGeomeanMaximumDen",
         "ordinaryCompileCaseMaximumNum", "ordinaryCompileCaseMaximumDen", "ordinary"),
    ]
    for medians, left, right, geo_num, geo_den, case_num, case_den, label in compile_gates:
        if not schema9_ratio_le(medians[left], medians[right], thresholds[geo_num], thresholds[geo_den]):
            fail(f"schema-9 {label} compile geometric gate failed")
        if any(not schema9_ratio_le([a], [b], thresholds[case_num], thresholds[case_den])
               for a, b in zip(medians[left], medians[right], strict=True)):
            fail(f"schema-9 {label} compile per-case gate failed")
    for case, session in sessions.items():
        if session["wallMs"] > thresholds["standardWallMsMaximum"]:
            fail(f"schema-9 {case} tuning wall budget failed")
        if not schema9_ratio_le(
                [session["peakRssBytes"]], [session["ordinaryPeakRssBytes"]],
                thresholds["peakRssMaximumNum"], thresholds["peakRssMaximumDen"]):
            fail(f"schema-9 {case} peak RSS gate failed")
        if session["cacheBytes"] > thresholds["cacheBytesMaximum"]:
            fail(f"schema-9 {case} cache size gate failed")

    audit = subprocess.run(
        [sys.executable, "-B", REPO / "scripts/audit-performance-oracles.py", "--tune",
         "--clang", os.environ["CKC_CLANG_ORACLE"]], cwd=REPO, env=os.environ.copy(),
        text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False,
    )
    if audit.returncode:
        fail(f"schema-9 independent oracle audit failed: {audit.stdout[-3000:]}")
    schema9_check_evidence_closure(report, evidence_root)


def check(path: pathlib.Path, baseline_manifest: pathlib.Path, *, schema_only=False,
          schema=None, compat_candidate=None, candidate_sha=None):
    raw = path.read_text(encoding="utf-8")
    report = json.loads(raw, object_pairs_hook=strict_json_object)
    actual_schema = report.get("schemaVersion")
    if schema is not None and actual_schema != schema:
        fail(f"requested schema {schema} does not match report schema {actual_schema}")
    if actual_schema == 9:
        canonical = json.dumps(report, sort_keys=True, separators=(",", ":")) + "\n"
        if raw != canonical:
            fail("schema-9 JSON is not canonical sorted UTF-8 with one trailing LF")
        check_schema9(report, path, schema_only=schema_only)
    elif schema_only:
        fail("--schema-only is defined only for schema 9")
    elif actual_schema == 8:
        check_schema8(
            report, path, baseline_manifest,
            candidate_version=compat_candidate or "0.13.0", candidate_sha=candidate_sha,
        )
    else:
        if compat_candidate or candidate_sha:
            fail("compatibility candidate overrides require schema 8")
        check_schema7(report, path, baseline_manifest)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--schema-only", action="store_true")
    parser.add_argument("--schema", type=int, choices=[7, 8, 9])
    parser.add_argument("--compat-candidate")
    parser.add_argument("--candidate-sha")
    parser.add_argument("results", type=pathlib.Path)
    parser.add_argument("baseline", nargs="?", type=pathlib.Path, default=DEFAULT_BASELINE_MANIFEST)
    args = parser.parse_args()
    if bool(args.compat_candidate) != bool(args.candidate_sha):
        parser.error("--compat-candidate and --candidate-sha must be supplied together")
    if args.compat_candidate and args.schema != 8:
        parser.error("compatibility overrides require --schema 8")
    try:
        check(
            args.results, args.baseline, schema_only=args.schema_only, schema=args.schema,
            compat_candidate=args.compat_candidate, candidate_sha=args.candidate_sha,
        )
    except (OSError, ValueError, KeyError, IndexError, StopIteration, TypeError,
            OverflowError, subprocess.SubprocessError, json.JSONDecodeError,
            tomllib.TOMLDecodeError) as error:
        print(f"native performance gate failed: {error}", file=sys.stderr)
        return 1
    print("native performance gate passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
