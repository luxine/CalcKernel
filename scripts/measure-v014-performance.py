#!/usr/bin/env python3
"""Collect CK 0.14 schema-9 evidence or emit its non-accepting contract fixture."""

from __future__ import annotations

import argparse
import ctypes
import hashlib
import json
import os
import pathlib
import platform
import re
import shutil
import struct
import subprocess
import sys
import tarfile
import time
import tomllib

try:
    import resource
except ImportError:  # pragma: no cover - full performance workers are Unix
    resource = None

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
ABI_BY_CASE = {
    "branch-layout": "slice-branch-u64",
    "call-constant-length": "slice-fixed-u32",
    "trip-unroll-simd": "slice-map-u32",
    "memory-bound": "slice-zip-u32",
    "compute-bound": "slice-f64",
    "contract-noalias": "slice-map-u32-domain",
    "contract-fixed-length": "slice-fixed-length-u32-domain",
}
ORACLE_CASE = {name: index for index, name in enumerate([
    "branch-layout", "call-constant-length", "trip-unroll-simd", "memory-bound",
    "compute-bound", "contract-noalias", "contract-fixed-length",
], 1)}
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


def parse_pgo_split(name: str) -> dict[str, dict]:
    path = REPO / f"benches/fixtures/pgo/{name}.tsv"
    lines = path.read_text(encoding="utf-8").splitlines()
    expected = "held-out" if name == "held-out" else name
    if not lines or lines[0] != f"ckc-pgo-inputs\t1\t{expected}":
        fail(f"unsupported {name} partition")
    rows = {}
    for line in lines[1:]:
        if not line or line.startswith("#"):
            continue
        fields = line.split("\t")
        if len(fields) != 5:
            fail(f"malformed {name} partition row")
        case, record, length, seed, parameter = fields
        rows[(case, record)] = {
            "record": record, "length": int(length), "seed": int(seed),
            "parameter": parameter,
        }
    return rows


def provenance_case(case: str) -> str:
    return {
        "contract-noalias": "memory-bound",
        "contract-fixed-length": "call-constant-length",
    }.get(case, case)


def case_record(row: dict, split: str, partitions: dict) -> dict:
    if split == "release-held-out":
        return partitions[split][row["case"]]
    record_name = row["searchRecord"] if split == "training" else row["validationRecord"]
    key = (provenance_case(row["case"]), record_name)
    try:
        return partitions[split][key]
    except KeyError:
        fail(f"missing {split} record for {row['case']}")


class ExternalKernel:
    """Loads one retained dynamic library and exposes the frozen corpus ABI."""

    def __init__(self, library: pathlib.Path, case: str, record: dict):
        self.library = ctypes.CDLL(str(library))
        self.function = self.library.kernel
        self.case = case
        self.record = record
        self.keepalive = []
        self.output = None
        self.arguments = self._arguments()

    def _u32(self, length: int, seed: int):
        values = (ctypes.c_uint32 * max(1, length))()
        for index in range(length):
            values[index] = ((index + seed) * 2_654_435_761) % 1_000_002 + 1
        self.keepalive.append(values)
        return values

    def _f64(self, length: int, seed: int):
        values = (ctypes.c_double * max(1, length))()
        for index in range(length):
            values[index] = (index - length / 2 + seed) / 16.0 + 0.25
        self.keepalive.append(values)
        return values

    def _arguments(self):
        length, seed = self.record["length"], self.record["seed"]
        abi = ABI_BY_CASE[self.case]
        if abi == "slice-branch-u64":
            value = int(self.record["parameter"])
            items = (ctypes.c_uint64 * max(1, length))(*([value] * length))
            self.keepalive.append(items)
            self.function.argtypes = [
                ctypes.POINTER(ctypes.c_uint64), ctypes.c_uint32,
                ctypes.c_uint32, ctypes.c_uint64,
            ]
            self.function.restype = ctypes.c_uint64
            return items, length, length, seed
        if abi == "slice-fixed-u32":
            actual = 4_000
            value = int(self.record["parameter"])
            source = (ctypes.c_uint32 * actual)(*([value] * actual))
            output = (ctypes.c_uint32 * actual)()
            self.keepalive.extend([source, output])
            self.output = output
            self.function.argtypes = [
                ctypes.POINTER(ctypes.c_uint32), ctypes.c_uint32,
                ctypes.POINTER(ctypes.c_uint32), ctypes.c_uint32,
            ]
            self.function.restype = None
            return source, actual, output, actual
        if abi in {"slice-map-u32", "slice-map-u32-domain", "slice-fixed-length-u32-domain"}:
            actual = 16 if abi == "slice-fixed-length-u32-domain" else length
            source, output = self._u32(actual, seed), (ctypes.c_uint32 * max(1, actual))()
            self.keepalive.append(output)
            self.output = output
            self.function.argtypes = [
                ctypes.POINTER(ctypes.c_uint32), ctypes.c_uint32,
                ctypes.POINTER(ctypes.c_uint32), ctypes.c_uint32, ctypes.c_uint32,
            ]
            self.function.restype = None
            return source, actual, output, actual, actual
        if abi == "slice-zip-u32":
            left = self._u32(length, seed)
            right = self._u32(length, seed + 17)
            output = (ctypes.c_uint32 * max(1, length))()
            self.keepalive.append(output)
            self.output = output
            self.function.argtypes = [
                ctypes.POINTER(ctypes.c_uint32), ctypes.c_uint32,
                ctypes.POINTER(ctypes.c_uint32), ctypes.c_uint32,
                ctypes.POINTER(ctypes.c_uint32), ctypes.c_uint32, ctypes.c_uint32,
            ]
            self.function.restype = None
            return left, length, right, length, output, length, length
        if abi == "slice-f64":
            source, output = self._f64(length, seed), (ctypes.c_double * max(1, length))()
            self.keepalive.append(output)
            self.output = output
            self.function.argtypes = [
                ctypes.POINTER(ctypes.c_double), ctypes.c_uint32,
                ctypes.POINTER(ctypes.c_double), ctypes.c_uint32,
                ctypes.c_uint32, ctypes.c_double,
            ]
            self.function.restype = None
            return source, length, output, length, length, float(self.record["parameter"])
        fail(f"unsupported tuning ABI {abi}")

    def invoke(self):
        return self.function(*self.arguments)

    def run(self, iterations: int):
        result = None
        for _ in range(iterations):
            result = self.invoke()
        return result

    def result_bytes(self) -> bytes:
        result = self.invoke()
        if ABI_BY_CASE[self.case] == "slice-branch-u64":
            return int(result).to_bytes(8, "little")
        return bytes(self.output)

    def correctness_digest(self, case_id: str) -> str:
        return result_digest(case_id, self.result_bytes())


class ExternalPerformanceKernel:
    """Runs a native iteration batch with setup and harness I/O outside its timer."""

    def __init__(self, runner: pathlib.Path, library: pathlib.Path, case: str, record: dict):
        self.runner = runner
        self.library = library
        self.case = case
        self.record = record

    def call(self, iterations: int, case_id: str) -> dict:
        result = subprocess.run(
            [
                self.runner, "--ck-perf", self.library, self.case, case_id,
                str(self.record["length"]), str(self.record["seed"]),
                str(self.record["parameter"]), str(iterations),
            ],
            cwd=REPO, env={}, text=True, stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT, check=False,
        )
        framing = (result.stdout.endswith("\n") and "\n" not in result.stdout[:-1]
                   and "\r" not in result.stdout)
        fields = result.stdout[:-1].split(" ") if framing else []
        expected = ["CKPERF/1", case_id, str(self.record["seed"]), str(iterations)]
        if (result.returncode != 0 or len(fields) != 7 or fields[:4] != expected
                or any(not field for field in fields)
                or fields[4] != str(iterations)
                or re.fullmatch(r"[0-9]+", fields[5]) is None
                or not 0 < int(fields[5]) <= 0xffff_ffff_ffff_ffff
                or re.fullmatch(r"[0-9a-f]{64}", fields[6]) is None):
            fail(f"{self.case} native performance runner returned a malformed receipt")
        return {
            "elapsedNs": int(fields[5]), "iterations": iterations,
            "completed": iterations, "correctnessDigest": fields[6],
        }

    def correctness_digest(self, case_id: str) -> str:
        return self.call(1, case_id)["correctnessDigest"]


def timed_call(kernel: ExternalPerformanceKernel, iterations: int, digest: str,
               case_id: str) -> dict:
    receipt = kernel.call(iterations, case_id)
    if receipt["correctnessDigest"] != digest:
        fail(f"{kernel.case} produced an incorrect result after a timed call")
    return receipt


def calibrate(kernel: ExternalPerformanceKernel, channel: str, digest: str,
              case_id: str) -> dict:
    attempts = []
    iterations = 1
    for _ in range(32):
        receipt = timed_call(kernel, iterations, digest, case_id)
        attempts.append(receipt)
        if receipt["elapsedNs"] >= 50_000_000:
            return {
                "channel": channel, "attempts": attempts,
                "selectedIterationsPerCall": iterations,
                "confirmation": timed_call(kernel, iterations, digest, case_id),
            }
        iterations = iterations * 2
        if iterations > 0xffff_ffff_ffff_ffff:
            fail("external calibration iteration overflow")
    fail("external calibration did not reach 50 ms in 32 doublings")


def order(candidate_sha: str, protocol: str, split: str, case: str,
          phase: int, row: int, channels: list[str]) -> list[str]:
    material = (b"CK-V014-PERF-ORDER\0" + text(candidate_sha) + text(protocol)
                + text(split) + text(case) + phase.to_bytes(1, "big")
                + row.to_bytes(4, "big"))
    rotation = int.from_bytes(hashlib.sha256(material).digest()[:8], "big") % len(channels)
    return channels[rotation:] + channels[:rotation]


def upper_median(values: list[int]) -> int:
    return sorted(values)[len(values) // 2]


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


def copy_retained(source: pathlib.Path, evidence: pathlib.Path, relative: str,
                  *, executable: bool = False) -> dict:
    checked_file(source)
    destination = evidence / relative
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists():
        fail(f"retained destination already exists: {relative}")
    shutil.copyfile(source, destination)
    destination.chmod(0o755 if executable else 0o644)
    return evidence_identity(evidence, relative)


def evidence_relative(evidence: pathlib.Path, path: pathlib.Path) -> str:
    try:
        return path.resolve().relative_to(evidence.resolve()).as_posix()
    except ValueError:
        fail(f"path is outside the evidence root: {path}")


def repository_relative(path: pathlib.Path) -> str:
    try:
        return path.resolve().relative_to(REPO.resolve()).as_posix()
    except ValueError:
        fail(f"full performance command path is outside the repository: {path}")


def file_argument(file: dict, evidence: pathlib.Path) -> str:
    return (file["path"] if file["root"] == "repository"
            else repository_relative(evidence / file["path"]))


def environment_entry(name: str, value: str, references: list[dict]) -> dict:
    return {"name": name, "value": value, "references": sorted(
        references, key=lambda item: (item["root"], item["path"].encode("utf-8")))}


def environment_digest(entries: list[dict]) -> str:
    encoded = []
    for entry in entries:
        encoded.append(text(entry["name"]) + text(entry["value"])
                       + list_value([file_value(item) for item in entry["references"]]))
    return p(b"CK-V014-PERF-COMMAND-ENV\0", list_value(encoded))


def command_record(argv: list[str], executable: dict, inputs: list[dict],
                   environment: list[dict]) -> dict:
    environment = sorted(environment, key=lambda item: item["name"].encode("utf-8"))
    return {
        "argv": argv, "workingDirectory": "repository", "executable": executable,
        "inputs": sorted(inputs, key=lambda item: (item["root"], item["path"].encode("utf-8"))),
        "environment": environment, "environmentDigest": environment_digest(environment),
    }


def command_digest(command: dict) -> str:
    return p(
        b"CK-V014-PERF-COMMAND\0",
        list_value([text(item) for item in command["argv"]]),
        text(command["workingDirectory"]), file_value(command["executable"]),
        list_value([file_value(item) for item in command["inputs"]]),
        bytes.fromhex(command["environmentDigest"]),
    )


def child_environment(entries: list[dict]) -> dict[str, str]:
    return {entry["name"]: entry["value"] for entry in entries}


def actual_argv(command: dict, evidence: pathlib.Path) -> list[str]:
    executable = ((evidence if command["executable"]["root"] == "evidence" else REPO)
                  / command["executable"]["path"])
    if not executable.is_file() or executable.is_symlink():
        fail("recorded command executable is missing or indirect")
    return list(command["argv"])


def run_command(command: dict, evidence: pathlib.Path,
                executable_override: pathlib.Path | None = None) -> tuple[int, str]:
    start = time.perf_counter_ns()
    result = subprocess.run(
        actual_argv(command, evidence), cwd=REPO,
        env=child_environment(command["environment"]),
        text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False,
        executable=str(executable_override) if executable_override is not None else None,
    )
    elapsed = max(1, time.perf_counter_ns() - start)
    if result.returncode:
        fail(f"command failed ({result.returncode}): {' '.join(command['argv'])}\n{result.stdout[-6000:]}")
    return elapsed, result.stdout


def terminated_child_cpu_time_ns():
    if resource is None:
        fail("terminated-child CPU timing requires a Unix performance worker")
    usage = resource.getrusage(resource.RUSAGE_CHILDREN)
    return round((usage.ru_utime + usage.ru_stime) * 1_000_000_000)


def run_compile_command(command: dict, evidence: pathlib.Path,
                        executable_override: pathlib.Path | None = None) -> tuple[int, str]:
    start = terminated_child_cpu_time_ns()
    result = subprocess.run(
        actual_argv(command, evidence), cwd=REPO,
        env=child_environment(command["environment"]),
        text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False,
        executable=str(executable_override) if executable_override is not None else None,
    )
    elapsed = terminated_child_cpu_time_ns() - start
    if elapsed < 0:
        fail("terminated-child CPU clock moved backwards")
    if result.returncode:
        fail(f"command failed ({result.returncode}): {' '.join(command['argv'])}\n{result.stdout[-6000:]}")
    return max(1, elapsed), result.stdout


def run_supervised(command: dict, evidence: pathlib.Path, log_relative: str) -> tuple[dict, int, int, str]:
    if platform.system() != "Linux" or not hasattr(os, "wait4"):
        fail("full schema-9 supervision requires Linux wait4")
    output_path = evidence / f"{log_relative}.output"
    output_path.parent.mkdir(parents=True, exist_ok=True)
    start = time.clock_gettime_ns(time.CLOCK_MONOTONIC_RAW)
    pid = os.fork()
    if pid == 0:
        try:
            descriptor = os.open(output_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            os.dup2(descriptor, 1)
            os.dup2(descriptor, 2)
            if descriptor > 2:
                os.close(descriptor)
            os.chdir(REPO)
            argv = actual_argv(command, evidence)
            os.execve(argv[0], argv, child_environment(command["environment"]))
        except BaseException:
            os._exit(127)
    waited, status, usage = os.wait4(pid, 0)
    end = time.clock_gettime_ns(time.CLOCK_MONOTONIC_RAW)
    if waited != pid or status != 0:
        output = output_path.read_text(encoding="utf-8", errors="replace") if output_path.exists() else ""
        fail(f"supervised command failed status={status}: {output[-6000:]}")
    digest = command_digest(command)
    supervisor = (
        f"CK-TUNE-SUPERVISOR\t1\nstart\t{digest}\t{start}\n"
        f"wait4\t{end}\t{status}\t{usage.ru_maxrss}\n"
    ).encode("utf-8")
    log = retained_marker(evidence, log_relative, supervisor)
    wall_ms = max(1, (end - start + 999_999) // 1_000_000)
    peak = max(1, usage.ru_maxrss * 1024)
    output = output_path.read_text(encoding="utf-8", errors="replace")
    output_path.unlink()
    return log, wall_ms, peak, output


def dynamic_outputs(evidence: pathlib.Path, base_relative: str) -> list[dict]:
    suffix = ".so" if platform.system() == "Linux" else ".dylib"
    base = evidence / base_relative
    paths = [("primary", base.with_suffix(suffix)), ("header", base.with_suffix(".h"))]
    return [{"role": role, "file": evidence_identity(evidence, evidence_relative(evidence, path))}
            for role, path in paths]


def primary_output(outputs: list[dict]) -> dict:
    return next(item["file"] for item in outputs if item["role"] == "primary")


def build_record(command: dict, decision: dict | None, outputs: list[dict]) -> dict:
    return {"command": command, "decision": decision, "outputs": outputs}


def copy_tree(source: pathlib.Path, evidence: pathlib.Path, prefix: str) -> list[dict]:
    if not source.is_dir() or source.is_symlink():
        fail(f"retained tree is missing or indirect: {source}")
    output = []
    for entry in sorted(source.rglob("*")):
        if entry.is_symlink():
            fail(f"retained tree contains a symlink: {entry}")
        if entry.is_dir():
            continue
        if not entry.is_file():
            fail(f"retained tree contains a special entry: {entry}")
        relative = f"{prefix}/{entry.relative_to(source).as_posix()}"
        output.append(copy_retained(entry, evidence, relative, executable=os.access(entry, os.X_OK)))
    return sorted(output, key=lambda item: (item["root"], item["path"].encode("utf-8")))


def remove_owned_tree(evidence: pathlib.Path, path: pathlib.Path) -> None:
    evidence_relative(evidence, path)
    if path.is_symlink() or not path.is_dir():
        fail(f"owned scratch tree is missing or indirect: {path}")
    shutil.rmtree(path)


def remove_publication_locks(evidence: pathlib.Path, directory: pathlib.Path) -> None:
    evidence_relative(evidence, directory)
    for entry in directory.glob(".ckc-tune-dest-*.lock"):
        if (re.fullmatch(r"\.ckc-tune-dest-[0-9a-f]{64}\.lock", entry.name) is None
                or entry.is_symlink() or not entry.is_file()):
            fail(f"unexpected tuning publication lock entry: {entry}")
        entry.unlink()


def snapshot_cache(evidence: pathlib.Path, namespace: pathlib.Path) -> dict:
    namespace.mkdir(parents=True, exist_ok=True)
    relative = evidence_relative(evidence, namespace)
    files = []
    for entry in sorted(namespace.rglob("*")):
        if entry.is_symlink() or (not entry.is_dir() and not entry.is_file()):
            fail(f"cache namespace contains an unsafe entry: {entry}")
        if entry.is_file():
            files.append(evidence_identity(evidence, evidence_relative(evidence, entry)))
    files.sort(key=lambda item: (item["root"], item["path"].encode("utf-8")))
    digest = p(b"CK-V014-CACHE-SNAPSHOT\0", text(relative),
               list_value([file_value(item) for item in files]))
    return {"namespace": relative, "files": files, "digest": digest}


def tagged(nodes: list[dict], tag: int):
    matches = [node["value"] for node in nodes if node.get("tag") == tag]
    if len(matches) != 1:
        fail(f"inspection tree is missing unique tag {tag}")
    return matches[0]


def inspect_decision(candidate: pathlib.Path, decision: pathlib.Path) -> tuple[dict, dict]:
    result = subprocess.run(
        [candidate, "tune", "inspect", decision, "--json"], cwd=REPO,
        text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False,
    )
    if result.returncode:
        fail(f"decision inspection failed: {result.stdout[-6000:]}")
    inspection = json.loads(result.stdout)
    records = inspection.get("records")
    if not isinstance(records, list) or len(records) != 8:
        fail("decision inspection has an invalid top-level tree")
    selection = tagged(records, 7)
    replay = tagged(records, 8)
    candidates = tagged(records, 6)
    frontier = tagged(records, 5)
    reason = tagged(selection, 4)
    certificate = tagged(selection, 5)
    outputs = []
    for item in tagged(replay, 6):
        outputs.append({
            "role": tagged(item, 1), "logicalName": tagged(item, 2),
            "sha256": tagged(item, 3), "bytes": int(tagged(item, 4)),
        })
    certificate_digest = None
    if certificate is not None:
        certificate_digest = p(
            b"CK-V014-TUNE-CERTIFICATE\0",
            *(bytes.fromhex(tagged(certificate, tag)) for tag in range(1, 9)),
        )
    summary = {
        "decisionDigest": inspection["decisionDigest"],
        "choiceIdentityDigest": tagged(replay, 10),
        "selectionReason": reason, "planDigest": tagged(selection, 3),
        "objectGraphDigest": tagged(replay, 4),
        "linkRecipeDigest": tagged(replay, 5),
        "certificateDigest": certificate_digest, "outputRecords": outputs,
    }
    trial_nodes = tagged(candidates, 2)
    measured = sum(bool(tagged(item, 9)) for item in trial_nodes)
    round_one = tagged(selection, 1)
    return summary, {
        "compiled": len(trial_nodes), "measured": measured,
        "expansions": len(tagged(frontier, 4)),
        "validationEntrants": len(tagged(round_one, 2)),
    }


def output_content_digest(outputs: list[dict]) -> str:
    encoded = []
    for output in outputs:
        file = output["file"]
        encoded.append(text(output["role"]) + file["bytes"].to_bytes(8, "big")
                       + bytes.fromhex(file["sha256"]))
    return p(b"CK-V014-PERF-OUTPUT-CONTENT\0", list_value(encoded))


def derived_event_log(evidence: pathlib.Path, relative: str, summary: dict,
                      counts: dict, warm: bool) -> tuple[dict, str]:
    lines = ["CK-TUNE-EVENTS\t1"]
    events = []
    if warm:
        events.append(("cache-hit", "-", "-", "-", 0))
    else:
        events.append(("cache-miss", "-", "-", "-", 0))
        events.extend(("compile-attempt", "-", "-", "-", 0)
                      for _ in range(counts["compiled"]))
        events.extend(("measurement-evaluation", summary["planDigest"], "search", "-", 1)
                      for _ in range(counts["measured"]))
    events.append(("publication", summary["planDigest"], "-", "-", 0))
    for ordinal, event in enumerate(events):
        lines.append(f"{ordinal}\t{event[0]}\t{event[1]}\t{event[2]}\t{event[3]}\t{event[4]}")
    file = retained_marker(evidence, relative, ("\n".join(lines) + "\n").encode("utf-8"))
    return file, p(b"CK-V014-TUNE-EVENTS\0", file_value(file))


def compiler_environment(evidence: pathlib.Path, cache_base: pathlib.Path,
                         retained: dict) -> list[dict]:
    cache_base.mkdir(parents=True, exist_ok=True)
    return [
        environment_entry("CKC_CANDIDATE_COMPILER",
                          str(evidence / retained["candidate"]["path"]),
                          [retained["candidate"]]),
        environment_entry("CKC_CLANG_ORACLE", retained["clangOriginal"],
                          [retained["clang"]]),
        environment_entry("CKC_LLVM_PREFIX", retained["llvmPrefix"],
                          [retained["component"], retained["clang"], retained["clangRuntime"]]),
        environment_entry("CKC_V013_REPLAY_BUNDLE", retained["replayOriginal"],
                          retained["replayReferences"]),
        environment_entry("XDG_CACHE_HOME", str(cache_base), []),
    ]


def ck_build_command(evidence: pathlib.Path, compiler: dict, source: dict,
                     base_relative: str, environment: list[dict], *, decision: dict | None = None,
                     profile: dict | None = None, tune_manifest: dict | None = None,
                     tune_out_relative: str | None = None, budget: str | None = None) -> dict:
    if tune_manifest is not None:
        argv = [
            file_argument(compiler, evidence), "tune", "build", file_argument(source, evidence),
            "--config", file_argument(tune_manifest, evidence), "--out", base_relative,
            "--kind", "dynamic",
            "--cpu", "native", "-O3", "--overflow", "unchecked", "--bounds",
            "unchecked", "--budget", budget or "standard", "--tune-out",
            tune_out_relative or f"{base_relative}.cktune",
        ]
        inputs = [source, tune_manifest]
    else:
        argv = [
            file_argument(compiler, evidence), "build", file_argument(source, evidence),
            "--out", base_relative,
            "--kind", "dynamic", "--cpu", "native", "-O3", "--overflow",
            "unchecked", "--bounds", "unchecked",
        ]
        inputs = [source]
        if profile is not None:
            argv.extend(["--pgo-use", file_argument(profile, evidence)])
            inputs.append(profile)
        if decision is not None:
            argv.extend(["--tune-use", file_argument(decision, evidence)])
            inputs.append(decision)
    return command_record(argv, compiler, inputs, environment)


def run_tune(candidate_path: pathlib.Path, candidate: dict, case: dict,
             source: dict, manifest: dict, evidence: pathlib.Path,
             retained: dict, run_name: str, cache_base: pathlib.Path,
             *, warm: bool = False) -> tuple[dict, dict, dict]:
    base_relative = f"runs/{case['case']}/{run_name}/artifact"
    decision_relative = f"runs/{case['case']}/{run_name}/decision.cktune"
    base_argument = repository_relative(evidence / base_relative)
    decision_argument = repository_relative(evidence / decision_relative)
    environment = compiler_environment(evidence, cache_base, retained)
    namespace = cache_base / "ckc"
    cache_before = snapshot_cache(evidence, namespace)
    command = ck_build_command(
        evidence, candidate, source, base_argument, environment, tune_manifest=manifest,
        tune_out_relative=decision_argument, budget="standard",
    )
    supervisor, wall_ms, peak, output = run_supervised(
        command, evidence, f"runs/{case['case']}/{run_name}/supervisor.tsv")
    expected_phrase = "warm exact reuse" if warm else "fresh session"
    if expected_phrase not in output:
        fail(f"{case['case']} {run_name} did not report {expected_phrase}")
    outputs = dynamic_outputs(evidence, base_relative)
    decision_file = evidence_identity(evidence, decision_relative)
    summary, counts = inspect_decision(candidate_path, evidence / decision_relative)
    records = {item["role"]: item for item in summary["outputRecords"]}
    for artifact in outputs:
        record = records.get(artifact["role"])
        file = artifact["file"]
        if record is None or (record["logicalName"], record["bytes"], record["sha256"]) != (
                pathlib.PurePosixPath(file["path"]).name, file["bytes"], file["sha256"]):
            fail(f"{case['case']} decision output record mismatch")
    cache_after = snapshot_cache(evidence, namespace)
    event, event_digest = derived_event_log(
        evidence, f"runs/{case['case']}/{run_name}/events.tsv", summary, counts, warm)
    build = build_record(command, decision_file, outputs)
    tune_run = {
        "decision": decision_file, "outputs": outputs,
        "decisionDigest": summary["decisionDigest"],
        "choiceIdentityDigest": summary["choiceIdentityDigest"],
        "planDigest": summary["planDigest"],
        "objectGraphDigest": summary["objectGraphDigest"],
        "linkRecipeDigest": summary["linkRecipeDigest"],
        "outputContentDigest": output_content_digest(outputs), "build": build,
        "cacheBefore": cache_before, "cacheAfter": cache_after,
        "eventLog": event, "eventDigest": event_digest,
        "supervisorLog": supervisor,
        "supervisorDigest": p(b"CK-V014-TUNE-SUPERVISOR\0", file_value(supervisor)),
        "compiledCandidates": 0 if warm else counts["compiled"],
        "measuredCandidates": 0 if warm else counts["measured"],
        "wallMs": wall_ms, "peakRssBytes": peak,
    }
    decision_row = {"case": case["case"], "file": decision_file, **summary}
    artifact_row = {"case": case["case"], "decision": decision_file, "outputs": outputs}
    remove_publication_locks(evidence, (evidence / base_relative).parent)
    return tune_run, decision_row, artifact_row


def artifact_handle(evidence: pathlib.Path, build: dict) -> dict:
    value = primary_output(build["outputs"])
    return {"identity": value, "_absolute": str(evidence / value["path"])}


def expected_results(evidence: pathlib.Path, cases: list[dict], release: dict,
                     release_input: dict) -> list[dict]:
    rows = []
    for case in cases:
        raw = release_result(case["case"], release[case["case"]])
        relative = f"expected/{case['case']}.release.bin"
        target = evidence / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(raw)
        digest = result_digest(f"{case['case']}.release", raw)
        if digest != case["releaseDigest"]:
            fail(f"release result fixture mismatch for {case['case']}")
        rows.append({
            "case": case["case"], "split": "release-held-out", "input": release_input,
            "canonicalBytes": evidence_identity(evidence, relative), "digest": digest,
        })
    return rows


def run_ordinary(compiler_path: pathlib.Path, compiler: dict, case: dict, source: dict,
                 evidence: pathlib.Path, retained: dict, name: str,
                 cache_base: pathlib.Path, *, profile: dict | None = None,
                 supervised: bool = False) -> tuple[dict, int, int | None, dict | None]:
    base_relative = f"builds/{case['case']}/{name}/artifact"
    base_argument = repository_relative(evidence / base_relative)
    environment = compiler_environment(evidence, cache_base, retained)
    command = ck_build_command(
        evidence, compiler, source, base_argument, environment, profile=profile)
    if supervised:
        supervisor, wall_ms, peak, _ = run_supervised(
            command, evidence, f"builds/{case['case']}/{name}/supervisor.tsv")
        elapsed = wall_ms * 1_000_000
    else:
        elapsed, _ = run_command(command, evidence, compiler_path)
        supervisor, peak = None, None
    outputs = dynamic_outputs(evidence, base_relative)
    remove_owned_tree(evidence, cache_base)
    return build_record(command, None, outputs), elapsed, peak, supervisor


def inspected_profile_compiler_source(inspection: dict) -> str:
    identity_value = inspection.get("identity")
    value = identity_value.get("compilerSource") if isinstance(identity_value, dict) else None
    if (inspection.get("schema") != 1 or inspection.get("format") != "CKPROF01"
            or not isinstance(value, str) or re.fullmatch(r"[0-9a-f]{64}", value) is None):
        fail("v0.13 profile inspection omitted its compiler source identity")
    return value


def train_v013_profile(compiler_path: pathlib.Path, compiler: dict, case: dict,
                       source: dict, record: dict, evidence: pathlib.Path,
                       retained: dict) -> tuple[dict, str]:
    base_relative = f"profiles/{case['case']}/generation"
    shard_relative = f"profiles/{case['case']}/shards"
    shard_dir = evidence / shard_relative
    shard_dir.mkdir(parents=True)
    cache = evidence / f"profiles/{case['case']}/cache"
    environment = compiler_environment(evidence, cache, retained)
    argv = [
        file_argument(compiler, evidence), "build", file_argument(source, evidence),
        "--out", repository_relative(evidence / base_relative),
        "--kind", "dynamic", "--cpu", "native", "-O3", "--overflow", "unchecked",
        "--bounds", "unchecked", "--pgo-generate",
        repository_relative(evidence / shard_relative),
    ]
    command = command_record(argv, compiler, [source], environment)
    run_command(command, evidence, compiler_path)
    outputs = dynamic_outputs(evidence, base_relative)
    kernel = ExternalKernel(evidence / primary_output(outputs)["path"], case["case"], record)
    if kernel.correctness_digest(f"{case['case']}.search") != case["searchDigest"]:
        fail(f"{case['case']} v0.13 profile training artifact is incorrect")
    kernel.run(256)
    header = evidence / next(item["file"]["path"] for item in outputs if item["role"] == "header")
    match = re.search(r"ck_profile_flush_[0-9a-f]{64}", header.read_text(encoding="utf-8"))
    if match is None:
        fail(f"{case['case']} profile generation header lacks the flush symbol")
    flush = getattr(kernel.library, match.group(0))
    flush.restype = ctypes.c_int32
    if flush() != 0:
        fail(f"{case['case']} v0.13 profile flush failed")
    shards = [item for item in shard_dir.iterdir() if item.is_file() and not item.is_symlink()]
    if len(shards) != 1:
        fail(f"{case['case']} did not produce exactly one v0.13 shard")
    profile_relative = f"profiles/{case['case']}/profile.ckprof"
    merge = command_record(
        [file_argument(compiler, evidence), "pgo", "merge", repository_relative(shards[0]),
         "--out", repository_relative(evidence / profile_relative)], compiler,
        [evidence_identity(evidence, evidence_relative(evidence, shards[0]))], environment,
    )
    run_command(merge, evidence, compiler_path)
    profile = evidence_identity(evidence, profile_relative)
    inspected = json.loads(subprocess.run(
        [compiler_path, "pgo", "inspect", evidence / profile_relative, "--json"], cwd=REPO,
        text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True, env={},
    ).stdout)
    compiler_source = inspected_profile_compiler_source(inspected)
    for output in outputs:
        (evidence / output["file"]["path"]).unlink()
    for shard in shards:
        shard.unlink()
    remove_owned_tree(evidence, cache)
    return profile, compiler_source


def build_oracle(kind: str, flavor: str, case: dict, evidence: pathlib.Path,
                 retained: dict, manifest: dict) -> dict:
    if kind not in {"c", "rust"} or flavor not in {"simd", "generic"}:
        fail("invalid oracle build request")
    compiler = retained["clang"] if kind == "c" else retained["rustc"]
    original = pathlib.Path(retained["clangOriginal"] if kind == "c" else retained["rustcOriginal"])
    source = retained["cOracle"] if kind == "c" else retained["rustOracle"]
    relative = f"oracles/{case['case']}/{kind}-{flavor}.so"
    (evidence / relative).parent.mkdir(parents=True, exist_ok=True)
    args = list(manifest[kind][f"{flavor}_args"])
    system_linker = retained["systemLinkerOriginal"]
    if kind == "c":
        argv = [file_argument(compiler, evidence), *args, f"--ld-path={system_linker}",
                f"-DCK_TUNE_ORACLE_CASE={ORACLE_CASE[case['case']]}",
                file_argument(source, evidence), "-o", repository_relative(evidence / relative)]
        inputs = [source, retained["oracleManifest"], retained["systemLinker"]]
    else:
        argv = [file_argument(compiler, evidence), *args,
                "-C", f"linker={retained['clangOriginal']}",
                "-C", f"link-arg=--ld-path={system_linker}", "--cfg",
                f'tune_case="{case["case"]}"', file_argument(source, evidence),
                "-o", repository_relative(evidence / relative)]
        inputs = [source, retained["oracleManifest"], retained["clang"],
                  retained["systemLinker"]]
    command = command_record(argv, compiler, inputs, [])
    run_command(command, evidence, original)
    file = evidence_identity(evidence, relative)
    return build_record(command, None, [{"role": "primary", "file": file}])


def measure_channels(candidate_sha: str, case: dict, split: str, input_identity: dict,
                     artifacts: dict[str, dict], builds: dict[str, dict],
                     record: dict, channels: list[str], protocol: str,
                     calibration_channel: str, runner: pathlib.Path) -> dict:
    case_id = (f"{case['case']}.release" if split != "validation"
               else f"{case['case']}.validation")
    expected = (case["releaseDigest"] if split != "validation" else case["validationDigest"])
    kernels = {
        channel: ExternalPerformanceKernel(
            runner, pathlib.Path(artifacts[channel]["_absolute"]), case["case"], record)
        for channel in channels
    }
    digests = {channel: kernel.correctness_digest(case_id) for channel, kernel in kernels.items()}
    if set(digests.values()) != {expected}:
        fail(f"{case['case']} {split} differential mismatch: {digests}")
    calibration = calibrate(kernels[calibration_channel], calibration_channel, expected, case_id)
    iterations = calibration["selectedIterationsPerCall"]
    warmup_order, sample_order = [], []
    warmups = {channel: [] for channel in channels}
    calls = {channel: [] for channel in channels}
    for row in range(3):
        sequence = order(candidate_sha, protocol, split, case["case"], 1, row, channels)
        warmup_order.append(sequence)
        for channel in sequence:
            warmups[channel].append([
                timed_call(kernels[channel], iterations, expected, case_id) for _ in range(7)
            ])
    for row in range(20):
        sequence = order(candidate_sha, protocol, split, case["case"], 2, row, channels)
        sample_order.append(sequence)
        for channel in sequence:
            calls[channel].append([
                timed_call(kernels[channel], iterations, expected, case_id) for _ in range(7)
            ])
    calls_ns = {
        channel: [[receipt["elapsedNs"] for receipt in row] for row in rows]
        for channel, rows in calls.items()
    }
    samples = {channel: [min(row) for row in rows] for channel, rows in calls_ns.items()}
    medians = {channel: upper_median(values) for channel, values in samples.items()}
    return {
        "case": case["case"], "source": case["sourceIdentity"], "input": input_identity,
        "decisionDigest": case["decisionDigest"], "correctnessDigest": expected,
        "correctnessDigests": digests,
        "artifacts": {channel: artifacts[channel]["identity"] for channel in channels},
        "buildCommands": {channel: builds[channel] for channel in channels},
        "calibration": calibration, "warmupOrder": warmup_order,
        "sampleOrder": sample_order, "warmupReceipts": warmups,
        "callReceipts": calls, "callsNs": calls_ns, "samplesNs": samples,
        "mediansNs": medians,
    }


def compile_comparison(case: dict, evidence: pathlib.Path, retained: dict,
                       left: str, right: str, serial_prefix: str,
                       command_factory) -> dict:
    channels = [left, right]
    orders = [[], []]
    commands = {channel: [] for channel in channels}
    samples = {channel: [] for channel in channels}
    for row in range(18):
        sequence = channels[row % 2:] + channels[:row % 2]
        orders[0 if row < 3 else 1].append(sequence)
        for channel in sequence:
            invocation = len(commands[channel])
            command = command_factory(channel, invocation)
            output_index = command["argv"].index("--out") + 1
            (REPO / command["argv"][output_index]).parent.mkdir(parents=True, exist_ok=True)
            elapsed, _ = run_compile_command(command, evidence)
            commands[channel].append({"command": command, "elapsedNs": elapsed})
            transient_outputs = dynamic_outputs(
                evidence, evidence_relative(evidence, REPO / command["argv"][output_index]))
            for output in transient_outputs:
                (evidence / output["file"]["path"]).unlink()
            remove_publication_locks(
                evidence, (REPO / command["argv"][output_index]).parent)
            cache = pathlib.Path(next(
                entry["value"] for entry in command["environment"]
                if entry["name"] == "XDG_CACHE_HOME"
            ))
            remove_owned_tree(evidence, cache)
            if row >= 3:
                samples[channel].append(elapsed)
    return {
        "case": case["case"], "warmupOrder": orders[0], "sampleOrder": orders[1],
        "samplesNs": samples,
        "mediansNs": {channel: upper_median(values) for channel, values in samples.items()},
        "commands": commands,
    }


def full_hardware(candidate_sha: str, target: str) -> dict:
    if platform.system() != "Linux":
        fail("full schema-9 collection requires Linux")
    machine = platform.machine().lower()
    arch = {"amd64": "x86_64", "x86_64": "x86_64",
            "arm64": "aarch64", "aarch64": "aarch64"}.get(machine)
    if arch is None:
        fail(f"unsupported performance architecture {machine}")
    cpuinfo = pathlib.Path("/proc/cpuinfo").read_text(encoding="utf-8", errors="replace").lower()
    model_match = re.search(r"(?:model name|hardware)\s*:\s*(.+)", cpuinfo)
    cpu_model = model_match.group(1).strip() if model_match else platform.processor() or machine
    feature_match = re.search(r"(?:flags|features)\s*:\s*(.+)", cpuinfo)
    all_features = set(feature_match.group(1).split()) if feature_match else set()
    interesting = {
        "avx2", "fma", "bmi2", "avx512f", "avx512bw", "avx512cd", "avx512dq", "avx512vl",
        "sve", "sve2",
    }
    features = sorted(all_features & interesting)
    if arch == "x86_64":
        required = "x86-64-v4"
        needed = {"avx512f", "avx512bw", "avx512cd", "avx512dq", "avx512vl"}
        tiers = ["baseline"]
        if {"avx2", "fma", "bmi2"}.issubset(all_features):
            tiers.append("x86-64-v3")
        if needed.issubset(all_features):
            tiers.append(required)
    else:
        required = "aarch64-sve2"
        needed = {"sve", "sve2"}
        tiers = ["baseline"]
        if "sve" in all_features:
            tiers.append("aarch64-sve")
        if needed.issubset(all_features):
            tiers.append(required)
    if not needed.issubset(all_features):
        fail(f"required stable performance tier {required} is unavailable")
    tiers = sorted(tiers)
    logical = os.cpu_count() or 1
    physical_pairs = set(re.findall(r"physical id\s*:\s*(\d+).*?core id\s*:\s*(\d+)",
                                    cpuinfo, flags=re.DOTALL))
    physical = len(physical_pairs) or logical
    numa_paths = list(pathlib.Path("/sys/devices/system/node").glob("node[0-9]*"))
    numa = len(numa_paths) or 1
    os_build = platform.version()
    kernel = platform.release()
    os_state = "linux-required-tier-active"
    values = [
        text(target), text(arch), text("linux"), text(os_build), text(kernel), text(cpu_model),
        logical.to_bytes(4, "big"), physical.to_bytes(4, "big"), numa.to_bytes(4, "big"),
        list_value([text(value) for value in features]), text(required),
        list_value([text(value) for value in tiers]), text(os_state),
    ]
    return {
        "target": target, "arch": arch, "os": "linux", "osBuild": os_build,
        "kernel": kernel, "cpuModel": cpu_model, "logicalCpus": logical,
        "physicalCpus": physical, "numaNodes": numa, "features": features,
        "requiredTier": required, "availableTiers": tiers, "osState": os_state,
        "capabilityDigest": p(b"CK-V014-PERF-HARDWARE\0", *values),
    }


def output_text(command: list[object]) -> str:
    result = subprocess.run([str(item) for item in command], cwd=REPO, text=True,
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False)
    if result.returncode:
        fail(f"command failed ({result.returncode}): {result.stdout[-6000:]}")
    return result.stdout


def clang_profile_runtime(clang: pathlib.Path) -> pathlib.Path:
    root = pathlib.Path(output_text([clang, "--print-resource-dir"]).strip())
    matches = sorted(root.glob("lib/**/libclang_rt.profile*.a"))
    matches += sorted(root.glob("lib/**/clang_rt.profile*.lib"))
    if len(matches) != 1:
        fail("Clang resource directory does not contain exactly one profile runtime")
    return matches[0]


def prepare_full_retained(evidence: pathlib.Path) -> dict:
    required = [
        "CKC_CANDIDATE_COMPILER", "CKC_LLVM_PREFIX", "CKC_CLANG_ORACLE",
        "CKC_V013_RUNTIME_BUNDLE", "CKC_V014_SCHEMA8_REPORT",
    ]
    missing = [name for name in required if not os.environ.get(name)]
    if missing:
        fail("full schema-9 collection requires " + ", ".join(missing))
    candidate_original = pathlib.Path(os.environ["CKC_CANDIDATE_COMPILER"]).resolve()
    prefix = pathlib.Path(os.environ["CKC_LLVM_PREFIX"]).resolve()
    clang_original = pathlib.Path(os.environ["CKC_CLANG_ORACLE"]).resolve()
    replay_original = pathlib.Path(os.environ["CKC_V013_RUNTIME_BUNDLE"]).resolve()
    schema8_original = pathlib.Path(os.environ["CKC_V014_SCHEMA8_REPORT"]).resolve()
    if not output_text([candidate_original, "--version"]).startswith("ckc 0.14.0"):
        fail("candidate compiler does not identify CK 0.14.0")
    verbose = output_text([candidate_original, "--version", "--verbose"])
    target_match = re.search(r"^Target: (.+)$", verbose, re.MULTILINE)
    if target_match is None:
        fail("candidate compiler verbose identity omitted Target")
    if "clang version 22.1.8" not in output_text([clang_original, "--version"]).lower():
        fail("CKC_CLANG_ORACLE is not Clang 22.1.8")
    rustc_original = pathlib.Path(output_text(
        ["rustup", "which", "rustc", "--toolchain", "1.90.0"]
    ).strip()).resolve()
    if not output_text([rustc_original, "--version"]).startswith("rustc 1.90.0 "):
        fail("resolved Rust compiler is not 1.90.0")
    component_original = prefix / "share/ckc/llvm-build.toml"
    clang_runtime_original = clang_profile_runtime(clang_original)
    system_linker_original = pathlib.Path("/usr/bin/ld").resolve(strict=True)
    checked_file(system_linker_original)
    candidate = copy_retained(candidate_original, evidence, "toolchain/ckc-v014", executable=True)
    component = copy_retained(component_original, evidence, "toolchain/llvm-build.toml")
    clang = copy_retained(clang_original, evidence, "toolchain/clang.bin", executable=True)
    clang_runtime = copy_retained(
        clang_runtime_original, evidence, "toolchain/clang-profile-runtime.bin")
    rustc = copy_retained(rustc_original, evidence, "toolchain/rustc.bin", executable=True)
    system_linker = copy_retained(
        system_linker_original, evidence, "toolchain/system-linker.bin", executable=True)
    runner = copy_retained(REPO / "target/release/ckc-tune-runner", evidence,
                           "workload/ckc-tune-runner", executable=True)
    replay_files = copy_tree(replay_original, evidence, "replay-v013")
    replay_manifest = copy_retained(REPLAY_MANIFEST, evidence,
                                    "replay-v013/v0_13_replay.toml")
    replay_files.append(replay_manifest)
    replay_files.sort(key=lambda item: (item["root"], item["path"].encode("utf-8")))
    by_suffix = lambda suffix: next(
        item for item in replay_files if item["path"].endswith(suffix))
    replay = {
        "commit": tomllib.loads(REPLAY_MANIFEST.read_text(encoding="utf-8"))["commit"],
        "manifest": replay_manifest, "compiler": by_suffix("/ckc-v013"),
        "archive": by_suffix("/ckc-v013-distribution.tar.gz"),
        "schemaEight": by_suffix("/schema8/v0.13-results.json"),
        "checker": by_suffix("/check-native-performance-v013.py"),
        "evidenceFiles": replay_files,
    }
    schema8_report = json.loads(schema8_original.read_text(encoding="utf-8"))
    schema8_dir = schema8_original.parent / schema8_report.get("evidenceDirectory", "")
    cumulative_files = [copy_retained(schema8_original, evidence,
                                      "compat-schema8/results-schema8-v014-compat.json")]
    cumulative_files.extend(copy_tree(schema8_dir, evidence,
                                      f"compat-schema8/{schema8_dir.name}"))
    cumulative_files.sort(key=lambda item: (item["root"], item["path"].encode("utf-8")))
    cumulative = {"report": cumulative_files[0], "files": cumulative_files}
    oracle_manifest = repository_identity("benches/oracles/tune/manifest.toml")
    c_oracle = repository_identity("benches/oracles/tune/c/tune_oracle.c")
    rust_oracle = repository_identity("benches/oracles/tune/rust/tune_oracle.rs")
    return {
        "candidateOriginal": candidate_original, "candidate": candidate,
        "llvmPrefix": str(prefix), "component": component,
        "clangOriginal": str(clang_original), "clang": clang,
        "clangRuntime": clang_runtime, "rustcOriginal": str(rustc_original), "rustc": rustc,
        "systemLinkerOriginal": str(system_linker_original), "systemLinker": system_linker,
        "runner": runner, "replayOriginal": str(replay_original), "replay": replay,
        "replayReferences": [replay_manifest, replay["compiler"], replay["archive"],
                             replay["schemaEight"], replay["checker"]],
        "cumulative": cumulative, "target": target_match.group(1),
        "oracleManifest": oracle_manifest, "cOracle": c_oracle, "rustOracle": rust_oracle,
    }


def compile_command_factory(evidence: pathlib.Path, retained: dict, case: dict,
                            source: dict, decision: dict, profile: dict,
                            comparison: str):
    compilers = {
        "tuneUse": retained["candidate"],
        "v014Ordinary": retained["candidate"],
        "v013Ordinary": retained["replay"]["compiler"],
    }

    def factory(channel: str, invocation: int) -> dict:
        compiler = compilers[channel]
        base = f"compile/{case['case']}/{comparison}/{channel}-{invocation}/artifact"
        cache = evidence / f"compile-cache/{case['case']}/{comparison}/{channel}-{invocation}"
        environment = compiler_environment(evidence, cache, retained)
        return ck_build_command(
            evidence, compiler, source, repository_relative(evidence / base), environment,
            decision=decision if channel == "tuneUse" else None,
            profile=profile if channel == "v013Pgo" else None,
        )

    return factory


def build_archive(evidence: pathlib.Path, retained: dict) -> dict:
    producer = repository_identity("scripts/package-v014-performance-archive.py")
    license_file = repository_identity("LICENSE")
    notices = repository_identity("THIRD_PARTY_NOTICES.md")
    relative = "archive/ckc-v014-distribution.tar.gz"
    candidate_archive = evidence / relative
    command = command_record(
        [
            file_argument(producer, evidence), "--compiler",
            file_argument(retained["candidate"], evidence), "--license",
            file_argument(license_file, evidence), "--notices", file_argument(notices, evidence),
            "--out", repository_relative(candidate_archive),
        ],
        producer, [retained["candidate"], license_file, notices], [],
    )
    run_command(command, evidence)
    candidate = evidence_identity(evidence, relative)
    expected = {
        "ckc-v0.14/LICENSE": (0o644, license_file),
        "ckc-v0.14/THIRD_PARTY_NOTICES.md": (0o644, notices),
        "ckc-v0.14/ckc": (0o755, retained["candidate"]),
    }
    members = []
    with tarfile.open(candidate_archive, "r:gz") as archive:
        records = archive.getmembers()
        if [record.name for record in records] != sorted(expected):
            fail("candidate archive member set/order mismatch")
        for record in records:
            mode, file = expected[record.name]
            if (not record.isfile() or record.mode != mode or record.mtime != 0
                    or record.uid != 0 or record.gid != 0 or record.uname or record.gname
                    or record.pax_headers):
                fail(f"candidate archive metadata mismatch: {record.name}")
            stream = archive.extractfile(record)
            if stream is None or stream.read() != ((REPO if file["root"] == "repository" else evidence)
                                                    / file["path"]).read_bytes():
                fail(f"candidate archive content mismatch: {record.name}")
            members.append({"path": record.name, "mode": mode, "file": file})
    return {
        "candidate": candidate, "v013Replay": retained["replay"]["archive"],
        "producer": producer, "command": command, "members": members,
    }


def full_report(output: pathlib.Path) -> dict:
    if platform.system() != "Linux":
        fail("full schema-9 collection requires a stable Linux performance host")
    candidate_sha = git_sha()
    stamp = f"v014-measurement-{int(time.time())}-{os.getpid()}"
    evidence = output.parent / stamp
    evidence.mkdir(parents=True, exist_ok=False)
    retained = prepare_full_retained(evidence)
    candidate_path = evidence / retained["candidate"]["path"]
    replay_path = evidence / retained["replay"]["compiler"]["path"]
    cases = parse_cases()
    release = parse_release()
    partitions = {
        "training": parse_pgo_split("training"),
        "validation": parse_pgo_split("held-out"),
        "release-held-out": release,
    }
    sources = {
        case["case"]: repository_identity(case["source"])
        for case in cases
    }
    manifests = {
        case["case"]: repository_identity(f"benches/tune/workloads/{case['manifest']}")
        for case in cases
    }
    search_input = repository_identity("benches/fixtures/pgo/training.tsv")
    validation_input = repository_identity("benches/fixtures/pgo/held-out.tsv")
    release_input = repository_identity("benches/fixtures/tune/release-held-out.tsv")
    expected = expected_results(evidence, cases, release, release_input)
    with (REPO / "benches/oracles/tune/manifest.toml").open("rb") as source:
        oracle_manifest = tomllib.load(source)

    profiles = {}
    profile_rows = []
    for case in cases:
        record = case_record(case, "training", partitions)
        profile, compiler_source = train_v013_profile(
            replay_path, retained["replay"]["compiler"], case, sources[case["case"]],
            record, evidence, retained,
        )
        profiles[case["case"]] = profile
        profile_rows.append({
            "case": case["case"], "file": profile, "compilerSource": compiler_source,
            "source": sources[case["case"]], "trainingInput": search_input,
        })

    decisions = []
    artifacts = []
    determinism = []
    resource_sessions = []
    main_rows = []
    validation_rows = []
    domain_rows = []
    tune_compile = []
    ordinary_compile = []
    size_rows = []

    for case in cases:
        name = case["case"]
        source = sources[name]
        manifest = manifests[name]
        cold_one, decision, artifact = run_tune(
            candidate_path, retained["candidate"], case, source, manifest, evidence, retained,
            "cold-one", evidence / f"cache/{name}/cold-one",
        )
        cold_two, _, _ = run_tune(
            candidate_path, retained["candidate"], case, source, manifest, evidence, retained,
            "cold-two", evidence / f"cache/{name}/cold-two",
        )
        warm, _, _ = run_tune(
            candidate_path, retained["candidate"], case, source, manifest, evidence, retained,
            "warm", evidence / f"cache/{name}/cold-one", warm=True,
        )
        cold_keys = [
            "choiceIdentityDigest", "planDigest", "objectGraphDigest",
            "linkRecipeDigest", "outputContentDigest",
        ]
        if any(cold_one[key] != cold_two[key] for key in cold_keys):
            fail(f"{name} independent cold tuning is not deterministic")
        warm_keys = ["decisionDigest", *cold_keys]
        if any(cold_one[key] != warm[key] for key in warm_keys):
            fail(f"{name} warm tuning did not exactly reuse cold one")
        if warm["cacheBefore"] != cold_one["cacheAfter"]:
            fail(f"{name} warm cache pre-state does not equal cold-one post-state")
        decisions.append(decision)
        artifacts.append(artifact)
        determinism.append({
            "case": name, "coldOne": cold_one, "coldTwo": cold_two, "warm": warm,
        })
        case["sourceIdentity"] = source
        case["decisionDigest"] = decision["decisionDigest"]

        v014, _, ordinary_peak, ordinary_log = run_ordinary(
            candidate_path, retained["candidate"], case, source, evidence, retained,
            "v014-ordinary", evidence / f"cache/{name}/v014-ordinary", supervised=True,
        )
        v013, _, _, _ = run_ordinary(
            replay_path, retained["replay"]["compiler"], case, source, evidence, retained,
            "v013-ordinary", evidence / f"cache/{name}/v013-ordinary",
        )
        v013_pgo, _, _, _ = run_ordinary(
            replay_path, retained["replay"]["compiler"], case, source, evidence, retained,
            "v013-pgo", evidence / f"cache/{name}/v013-pgo", profile=profiles[name],
        )
        c_simd = build_oracle("c", "simd", case, evidence, retained, oracle_manifest)
        rust_simd = build_oracle("rust", "simd", case, evidence, retained, oracle_manifest)
        builds = {
            "tuned": cold_one["build"], "v014Ordinary": v014,
            "v013Ordinary": v013, "v013Pgo": v013_pgo,
            "cSimd": c_simd, "rustSimd": rust_simd,
        }
        handles = {channel: artifact_handle(evidence, build) for channel, build in builds.items()}

        validation_rows.append(measure_channels(
            candidate_sha, case, "validation", validation_input, handles, builds,
            case_record(case, "validation", partitions), VALIDATION_CHANNELS,
            "rotating-three-channel-v1", "v013Ordinary",
            evidence / retained["runner"]["path"],
        ))
        if case["partition"] == "eligible":
            row = measure_channels(
                candidate_sha, case, "release-held-out", release_input, handles, builds,
                case_record(case, "release-held-out", partitions), MAIN_CHANNELS,
                "rotating-six-channel-v1", "v014Ordinary",
                evidence / retained["runner"]["path"],
            )
            row["eligible"] = True
            main_rows.append(row)
        else:
            generic_c = build_oracle("c", "generic", case, evidence, retained, oracle_manifest)
            generic_rust = build_oracle("rust", "generic", case, evidence, retained, oracle_manifest)
            domain_builds = {
                "tuned": cold_one["build"], "genericC": generic_c, "genericRust": generic_rust,
            }
            domain_handles = {
                channel: artifact_handle(evidence, build)
                for channel, build in domain_builds.items()
            }
            domain_rows.append(measure_channels(
                candidate_sha, case, "domain-release-held-out", release_input,
                domain_handles, domain_builds,
                case_record(case, "release-held-out", partitions), DOMAIN_CHANNELS,
                "rotating-three-channel-v1", "genericC",
                evidence / retained["runner"]["path"],
            ))

        tune_compile.append(compile_comparison(
            case, evidence, retained, "tuneUse", "v014Ordinary", "tune-use",
            compile_command_factory(
                evidence, retained, case, source, decision["file"], profiles[name], "tune-use"),
        ))
        ordinary_compile.append(compile_comparison(
            case, evidence, retained, "v014Ordinary", "v013Ordinary", "ordinary",
            compile_command_factory(
                evidence, retained, case, source, decision["file"], profiles[name], "ordinary"),
        ))
        tuned_primary = primary_output(cold_one["outputs"])
        baseline_primary = primary_output(v014["outputs"])
        size_rows.append({
            "case": name, "tunedPrimary": tuned_primary,
            "baselinePrimary": baseline_primary, "baselineBuild": v014,
        })
        _, counts = inspect_decision(candidate_path, evidence / decision["file"]["path"])
        if ordinary_log is None or ordinary_peak is None:
            fail("ordinary resource supervision is missing")
        resource_sessions.append({
            "case": name, "decision": decision["file"],
            "decisionDigest": decision["decisionDigest"], "ordinaryBuild": v014,
            "ordinarySupervisorLog": ordinary_log,
            "ordinarySupervisorDigest": p(
                b"CK-V014-TUNE-SUPERVISOR\0", file_value(ordinary_log)),
            "budget": "standard", "wallMs": cold_one["wallMs"],
            "peakRssBytes": cold_one["peakRssBytes"],
            "ordinaryPeakRssBytes": ordinary_peak, "expansions": counts["expansions"],
            "compileAttempts": cold_one["compiledCandidates"],
            "measuredFinalists": cold_one["measuredCandidates"],
            "validationEntrants": counts["validationEntrants"],
            "cacheBytes": sum(item["bytes"] for item in cold_one["cacheAfter"]["files"]),
        })

    recipe_files = sorted(
        (repository_identity(name) for name in RECIPE_FILES),
        key=lambda item: (item["root"], item["path"].encode("utf-8")),
    )
    archive = build_archive(evidence, retained)
    report = {
        "schemaVersion": 9, "candidateVersion": "0.14.0", "candidateSha": candidate_sha,
        "v013ReplayCommit": retained["replay"]["commit"], "evidenceDirectory": stamp,
        "toolchain": {
            "llvmVersion": "22.1.8", "clangVersion": "22.1.8",
            "rustVersion": "1.90.0", "componentManifest": retained["component"],
            "clangBinary": retained["clang"], "clangProfileRuntime": retained["clangRuntime"],
            "rustCompiler": retained["rustc"], "systemLinker": retained["systemLinker"],
        },
        "hardware": full_hardware(candidate_sha, retained["target"]),
        "recipe": recipe(recipe_files), "candidateBinary": retained["candidate"],
        "v013ReplayBundle": retained["replay"],
        "cumulativeSchemaEight": retained["cumulative"],
        "workload": {
            "casesManifest": repository_identity("benches/cases/tune-cases.tsv"),
            "sources": sorted(sources.values(), key=lambda item: item["path"].encode("utf-8")),
            "search": search_input, "validation": validation_input,
            "adversarial": repository_identity("benches/fixtures/pgo/adversarial.tsv"),
            "releaseHeldOut": release_input,
            "tuneManifests": sorted(manifests.values(), key=lambda item: item["path"].encode("utf-8")),
            "runner": retained["runner"], "oracleManifest": retained["oracleManifest"],
            "cOracle": retained["cOracle"], "rustOracle": retained["rustOracle"],
            "profiles": profile_rows, "expectedResults": expected,
        },
        "tuningDecisions": decisions, "tuningArtifacts": artifacts,
        "sampling": {
            "mainProtocol": "rotating-six-channel-v1",
            "validationProtocol": "rotating-three-channel-v1",
            "domainProtocol": "rotating-three-channel-v1", "mainChannels": MAIN_CHANNELS,
            "validationChannels": VALIDATION_CHANNELS, "domainChannels": DOMAIN_CHANNELS,
            "warmupRows": 3, "sampleRows": 20, "callsPerSample": 7,
            "statistic": "minimum-then-upper-median",
            "stabilityPolicy": "at-least-80-percent-within-20-percent-of-upper-median",
            "rerunPolicy": "unstable-evidence-is-invalid-no-selective-rerun",
        },
        "cases": main_rows, "validationCases": validation_rows, "domainCases": domain_rows,
        "tuneUseCompileTime": tune_compile,
        "ordinaryCompileRegression": ordinary_compile, "artifactSize": size_rows,
        "archiveSize": archive,
        "resourceUse": {"sessions": resource_sessions, "cacheHardLimitBytes": 4_294_967_296},
        "determinism": determinism,
        "correctness": {
            "search": True, "validation": True, "adversarial": True,
            "validationDifferential": True, "releaseHeldOutDifferential": True,
            "domainDifferential": True, "oracleUbAudit": True, "aliasAudit": True,
            "featureAudit": True,
        },
    }
    if set(report) != TOP_LEVEL_KEYS:
        fail("internal full schema-9 top-level key drift")
    return report


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
    system_linker = retained_marker(
        evidence, "toolchain/system-linker.bin", b"system linker contract-only\n")
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
            "systemLinker": system_linker,
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
        output = args.out.resolve()
        output.parent.mkdir(parents=True, exist_ok=True)
        if output.exists():
            output.unlink()
        report = contract_report(output) if args.contract_only else full_report(output)
        output.write_text(json.dumps(report, sort_keys=True, separators=(",", ":")) + "\n",
                          encoding="utf-8")
    except (OSError, ValueError, KeyError, IndexError, StopIteration, TypeError,
            OverflowError, subprocess.SubprocessError, tomllib.TOMLDecodeError) as error:
        parser.exit(1, f"schema-9 collection failed: {error}\n")
    kind = "contract" if args.contract_only else "full"
    print(f"schema-9 {kind} evidence written to {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
