#!/usr/bin/env python3
"""Collect CK 0.14 schema-9 evidence or emit its non-accepting contract fixture."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import platform
import shutil
import struct
import subprocess
import sys
import time
import tomllib

REPO = pathlib.Path(__file__).resolve().parents[1]
CASE_TABLE = REPO / "benches/cases/tune-cases.tsv"
REPLAY_MANIFEST = REPO / "benches/baselines/v0_13_replay.toml"
MAIN_CHANNELS = [
    "tuned", "v014Ordinary", "v013Ordinary", "v013Pgo", "cSimd", "rustSimd",
]
VALIDATION_CHANNELS = ["tuned", "v013Ordinary", "v013Pgo"]
DOMAIN_CHANNELS = ["tuned", "genericC", "genericRust"]
TOP_LEVEL_KEYS = {
    "schemaVersion", "candidateVersion", "candidateSha", "v013ReplayCommit",
    "evidenceDirectory", "toolchain", "hardware", "recipe", "candidateBinary",
    "v013ReplayBundle", "cumulativeSchemaEight", "workload", "tuningDecisions",
    "tuningArtifacts", "sampling", "cases", "validationCases", "domainCases",
    "tuneUseCompileTime", "ordinaryCompileRegression", "artifactSize", "archiveSize",
    "resourceUse", "determinism", "correctness",
}
THRESHOLDS = {
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
RECIPE_FILES = [
    "benches/cases/tune-cases.tsv",
    *[f"benches/tune/workloads/{name}.cktune.toml" for name in [
        "branch-layout", "call-constant-length", "compute-bound",
        "contract-fixed-length", "contract-noalias", "memory-bound", "trip-unroll-simd",
    ]],
    "benches/tune/runner.rs",
    "benches/oracles/tune/manifest.toml",
    "benches/oracles/tune/c/tune_oracle.c",
    "benches/oracles/tune/rust/tune_oracle.rs",
    "benches/fixtures/pgo/branch_layout.ck",
    "benches/fixtures/pgo/call_constant_length.ck",
    "benches/oracles/fixtures/map_u32.ck",
    "benches/oracles/fixtures/zip_u32.ck",
    "benches/fixtures/pgo/compute_bound.ck",
    "benches/oracles/fixtures/contract_noalias.ck",
    "benches/oracles/fixtures/contract_fixed_length.ck",
    "benches/fixtures/pgo/training.tsv",
    "benches/fixtures/pgo/held-out.tsv",
    "benches/fixtures/pgo/adversarial.tsv",
    "benches/fixtures/tune/release-held-out.tsv",
    "benches/tune_perf.rs",
    "scripts/measure-v014-performance.py",
    "scripts/check-native-performance.py",
    "scripts/audit-performance-oracles.py",
    "scripts/package-v014-performance-archive.py",
    "LICENSE",
    "THIRD_PARTY_NOTICES.md",
    "benches/baselines/v0_13_replay.toml",
    "specs/0.14/performance-schema-9.md",
]


def fail(message: str):
    raise ValueError(message)


def sha256(path: pathlib.Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def checked_file(path: pathlib.Path) -> pathlib.Path:
    metadata = path.lstat()
    if path.is_symlink() or not path.is_file() or metadata.st_size <= 0:
        fail(f"required input is not a nonempty regular file: {path}")
    return path


def identity(path: pathlib.Path, root: str, relative: str) -> dict:
    checked_file(path)
    return {"root": root, "path": relative, "bytes": path.stat().st_size, "sha256": sha256(path)}


def repository_identity(relative: str) -> dict:
    return identity(REPO / relative, "repository", relative)


def evidence_identity(evidence: pathlib.Path, relative: str) -> dict:
    return identity(evidence / relative, "evidence", relative)


def text(value: str) -> bytes:
    encoded = value.encode("utf-8")
    return len(encoded).to_bytes(4, "big") + encoded


def file_value(value: dict) -> bytes:
    return ({"repository": 1, "evidence": 2}[value["root"]].to_bytes(1, "big")
            + text(value["path"]) + value["bytes"].to_bytes(8, "big")
            + bytes.fromhex(value["sha256"]))


def p(domain: bytes, *values: bytes) -> str:
    digest = hashlib.sha256(domain)
    for value in values:
        digest.update(value)
    return digest.hexdigest()


def list_value(values: list[bytes]) -> bytes:
    return len(values).to_bytes(4, "big") + b"".join(values)


def recipe(files: list[dict]) -> dict:
    threshold_values = [text(name) + value.to_bytes(8, "big")
                        for name, value in sorted(THRESHOLDS.items())]
    digest = p(
        b"CK-V014-PERF-RECIPE\0", (1).to_bytes(4, "big"),
        list_value([file_value(item) for item in files]), list_value(threshold_values),
    )
    return {"schema": 1, "files": files, "digest": digest, "thresholds": THRESHOLDS}


def git_sha() -> str:
    result = subprocess.run(["git", "rev-parse", "HEAD"], cwd=REPO, text=True,
                            capture_output=True, check=True).stdout.strip()
    if len(result) != 40:
        fail("candidate SHA is not a full Git object id")
    return result


def parse_cases() -> list[dict]:
    lines = CASE_TABLE.read_text(encoding="utf-8").splitlines()
    if not lines or lines[0] != "ckc-tune-cases\t1":
        fail("unsupported tune case table")
    rows = []
    keys = [
        "case", "source", "manifest", "searchRecord", "searchSeed", "searchDigest",
        "validationRecord", "validationSeed", "validationDigest", "releaseRecord",
        "releaseSeed", "releaseDigest", "partition",
    ]
    for line in lines[1:]:
        if not line or line.startswith("#"):
            continue
        fields = line.split("\t")
        if len(fields) != len(keys):
            fail("malformed tune case table row")
        rows.append(dict(zip(keys, fields, strict=True)))
    if len(rows) != 7 or len({row["case"] for row in rows}) != 7:
        fail("tune case table must contain seven unique cases")
    return sorted(rows, key=lambda row: row["case"])


def parse_release() -> dict[str, dict]:
    lines = (REPO / "benches/fixtures/tune/release-held-out.tsv").read_text().splitlines()
    if not lines or lines[0] != "ckc-tune-inputs\t1\trelease-held-out":
        fail("unsupported release-held-out partition")
    rows = {}
    for line in lines[1:]:
        if not line or line.startswith("#"):
            continue
        fields = line.split("\t")
        if len(fields) != 5 or fields[0] in rows:
            fail("malformed or duplicate release-held-out row")
        rows[fields[0]] = {
            "record": fields[1], "length": int(fields[2]), "seed": int(fields[3]),
            "parameter": fields[4],
        }
    return rows


def u32_input(length: int, seed: int) -> list[int]:
    return [((index + seed) * 2_654_435_761) % 1_000_002 + 1 for index in range(length)]


def release_result(case: str, record: dict) -> bytes:
    length, seed = record["length"], record["seed"]
    if case == "branch-layout":
        result, value, mask = seed, int(record["parameter"]), (1 << 64) - 1
        for _ in range(length):
            if value == 3:
                result = (result * 3 + value) & mask
            else:
                result = (((((result * 5 - value) & mask) * 7 + value) & mask) * 3 + 11) & mask
        return result.to_bytes(8, "little")
    if case == "call-constant-length":
        result, output, value, mask = 0, [], int(record["parameter"]), (1 << 32) - 1
        for _ in range(4_000):
            if value == 13:
                result = (result * 3 + value) & mask
            else:
                result = (((((result * 5 - value) & mask) * 7 + value) & mask) * 3 + 11) & mask
            output.append(result)
        return b"".join(value.to_bytes(4, "little") for value in output)
    actual = 16 if case == "contract-fixed-length" else length
    if case in {"trip-unroll-simd", "contract-noalias", "contract-fixed-length"}:
        addend = 7 if case == "trip-unroll-simd" else 17
        return b"".join(((value + addend) & 0xffff_ffff).to_bytes(4, "little")
                        for value in u32_input(actual, seed))
    if case == "memory-bound":
        left, right = u32_input(length, seed), u32_input(length, seed + 17)
        return b"".join(((a + b) & 0xffff_ffff).to_bytes(4, "little")
                        for a, b in zip(left, right, strict=True))
    if case == "compute-bound":
        factor, output = float(record["parameter"]), []
        for index in range(length):
            value = (index - length / 2 + seed) / 16.0 + 0.25
            x = value * factor
            x += value
            x *= factor
            x -= value
            x *= x
            x += factor
            x *= factor
            x -= value
            x *= x
            x += value
            x *= factor
            x -= value
            output.append(struct.pack("<d", x))
        return b"".join(output)
    fail(f"unknown tune case {case}")


def result_digest(case_id: str, raw: bytes) -> str:
    return p(b"CK-TUNE-RESULT\0", (1).to_bytes(4, "big"), text(case_id),
             len(raw).to_bytes(8, "big"), raw)


def hardware(candidate_sha: str) -> dict:
    system = platform.system().lower()
    arch = {"arm64": "aarch64", "amd64": "x86_64"}.get(
        platform.machine().lower(), platform.machine().lower())
    target = f"{system}-{arch}"
    features: list[str] = []
    tiers = ["contract-only"]
    values = [
        text(target), text(arch), text(system), text(platform.version()),
        text(platform.release()), text(platform.processor() or platform.machine()),
        (os.cpu_count() or 1).to_bytes(4, "big"), (os.cpu_count() or 1).to_bytes(4, "big"),
        (1).to_bytes(4, "big"), list_value([text(value) for value in features]),
        text("contract-only"), list_value([text(value) for value in tiers]),
        text(f"contract-only:{candidate_sha}"),
    ]
    return {
        "target": target, "arch": arch, "os": system, "osBuild": platform.version(),
        "kernel": platform.release(), "cpuModel": platform.processor() or platform.machine(),
        "logicalCpus": os.cpu_count() or 1, "physicalCpus": os.cpu_count() or 1,
        "numaNodes": 1, "features": features, "requiredTier": "contract-only",
        "availableTiers": tiers, "osState": f"contract-only:{candidate_sha}",
        "capabilityDigest": p(b"CK-V014-PERF-HARDWARE\0", *values),
    }


def retained_marker(evidence: pathlib.Path, relative: str, content: bytes) -> dict:
    destination = evidence / relative
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_bytes(content)
    return evidence_identity(evidence, relative)


def contract_report(output: pathlib.Path) -> dict:
    candidate_sha = git_sha()
    with REPLAY_MANIFEST.open("rb") as source:
        replay = tomllib.load(source)
    stamp = f"v014-measurement-{int(time.time())}-{os.getpid()}"
    evidence = output.parent / stamp
    evidence.mkdir(parents=True, exist_ok=False)
    cases = parse_cases()
    release = parse_release()
    expected = []
    for case in cases:
        raw = release_result(case["case"], release[case["case"]])
        relative = f"expected/{case['case']}.release.bin"
        path = evidence / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(raw)
        digest = result_digest(f"{case['case']}.release", raw)
        if digest != case["releaseDigest"]:
            fail(f"release result fixture mismatch for {case['case']}")
        expected.append({
            "case": case["case"], "split": "release-held-out",
            "input": repository_identity("benches/fixtures/tune/release-held-out.tsv"),
            "canonicalBytes": evidence_identity(evidence, relative), "digest": digest,
        })
    marker = f"CK 0.14 contract-only evidence for {candidate_sha}\n".encode()
    candidate = retained_marker(evidence, "contract/candidate.bin", marker)
    component = retained_marker(evidence, "toolchain/llvm-build.toml", b"contract-only\n")
    clang = retained_marker(evidence, "toolchain/clang.bin", b"clang 22.1.8 contract-only\n")
    clang_runtime = retained_marker(
        evidence, "toolchain/clang-profile-runtime.bin", b"profile runtime contract-only\n")
    rustc = retained_marker(evidence, "toolchain/rustc.bin", b"rustc 1.90.0 contract-only\n")
    runner = retained_marker(evidence, "workload/ckc-tune-runner", marker)
    recipe_files = sorted((repository_identity(name) for name in RECIPE_FILES),
                          key=lambda item: (item["root"], item["path"].encode()))
    sources = sorted((repository_identity(case["source"]) for case in cases),
                     key=lambda item: item["path"].encode())
    manifests = sorted((repository_identity(f"benches/tune/workloads/{case['manifest']}")
                        for case in cases), key=lambda item: item["path"].encode())
    report = {
        "schemaVersion": 9, "candidateVersion": "0.14.0", "candidateSha": candidate_sha,
        "v013ReplayCommit": replay["commit"], "evidenceDirectory": stamp,
        "toolchain": {
            "llvmVersion": "22.1.8", "clangVersion": "22.1.8", "rustVersion": "1.90.0",
            "componentManifest": component, "clangBinary": clang,
            "clangProfileRuntime": clang_runtime, "rustCompiler": rustc,
        },
        "hardware": hardware(candidate_sha), "recipe": recipe(recipe_files),
        "candidateBinary": candidate, "v013ReplayBundle": {}, "cumulativeSchemaEight": {},
        "workload": {
            "casesManifest": repository_identity("benches/cases/tune-cases.tsv"),
            "sources": sources,
            "search": repository_identity("benches/fixtures/pgo/training.tsv"),
            "validation": repository_identity("benches/fixtures/pgo/held-out.tsv"),
            "adversarial": repository_identity("benches/fixtures/pgo/adversarial.tsv"),
            "releaseHeldOut": repository_identity("benches/fixtures/tune/release-held-out.tsv"),
            "tuneManifests": manifests, "runner": runner,
            "oracleManifest": repository_identity("benches/oracles/tune/manifest.toml"),
            "cOracle": repository_identity("benches/oracles/tune/c/tune_oracle.c"),
            "rustOracle": repository_identity("benches/oracles/tune/rust/tune_oracle.rs"),
            "profiles": [], "expectedResults": expected,
        },
        "tuningDecisions": [], "tuningArtifacts": [],
        "sampling": {
            "mainProtocol": "rotating-six-channel-v1",
            "validationProtocol": "rotating-three-channel-v1",
            "domainProtocol": "rotating-three-channel-v1",
            "mainChannels": MAIN_CHANNELS, "validationChannels": VALIDATION_CHANNELS,
            "domainChannels": DOMAIN_CHANNELS, "warmupRows": 3, "sampleRows": 20,
            "callsPerSample": 7, "statistic": "minimum-then-upper-median",
            "stabilityPolicy": "at-least-80-percent-within-20-percent-of-upper-median",
            "rerunPolicy": "unstable-evidence-is-invalid-no-selective-rerun",
        },
        "cases": [], "validationCases": [], "domainCases": [],
        "tuneUseCompileTime": [], "ordinaryCompileRegression": [], "artifactSize": [],
        "archiveSize": {}, "resourceUse": {"sessions": [], "cacheHardLimitBytes": 4_294_967_296},
        "determinism": [],
        "correctness": {
            "search": False, "validation": False, "adversarial": False,
            "validationDifferential": False, "releaseHeldOutDifferential": False,
            "domainDifferential": False, "oracleUbAudit": False, "aliasAudit": False,
            "featureAudit": False,
        },
    }
    if set(report) != TOP_LEVEL_KEYS:
        fail("internal schema-9 top-level key drift")
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--contract-only", action="store_true")
    parser.add_argument("--task", choices=["collect"])
    parser.add_argument("--out", type=pathlib.Path, required=True)
    args = parser.parse_args()
    try:
        if not args.contract_only:
            fail("full schema-9 collection requires a stable Linux tier and is invoked by CI")
        output = args.out.resolve()
        output.parent.mkdir(parents=True, exist_ok=True)
        if output.exists():
            output.unlink()
        report = contract_report(output)
        output.write_text(json.dumps(report, sort_keys=True, separators=(",", ":")) + "\n",
                          encoding="utf-8")
    except (OSError, ValueError, subprocess.SubprocessError, tomllib.TOMLDecodeError) as error:
        parser.exit(1, f"schema-9 collection failed: {error}\n")
    print(f"schema-9 contract evidence written to {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
