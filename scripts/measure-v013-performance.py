#!/usr/bin/env python3
"""Collect real CK 0.13 schema-8 PGO and multiversion performance evidence."""

from __future__ import annotations

import argparse
import ctypes
import gzip
import hashlib
import io
import json
import os
import pathlib
import platform
import re
import shutil
import subprocess
import sys
import tarfile
import time
import tomllib

try:
    import resource
except ImportError:  # pragma: no cover - release performance workers are Unix
    resource = None

REPO = pathlib.Path(__file__).resolve().parents[1]
V012_COMMIT = "a49fa419669c400447dc13bcfa41ea464b3b040d"
LLVM_VERSION = "22.1.8"
RUST_VERSION = "1.90.0"
CHANNELS = [
    "ordinary", "replayV012", "pgo", "multiversion",
    "combined", "selectedDirect", "clangPgo", "rustPgo",
]
THRESHOLDS = (
    "ordinaryGeoSlowdown=1.02;ordinaryIndividualSlowdown=1.05;"
    "pgoGeoImprovement=1.05;pgoIndividualSlowdown=1.03;"
    "dispatchGeoImprovement=1.08;dispatchDirectGeoThroughput=0.98;"
    "combinedGeoSlowdown=1.02;oracleGeoThroughput=0.95;"
    "generationOverhead=5.0;archiveGrowth=1.15"
)
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
    "benches/baselines/v0_12_replay.toml",
    "benches/fixtures/pgo/training.tsv",
    "benches/fixtures/pgo/held-out.tsv",
    "benches/fixtures/pgo/adversarial.tsv",
    "benches/oracles/pgo/c/pgo_oracle.c",
    "benches/oracles/pgo/rust/pgo_oracle.rs",
]
SOURCE_PATHS = {
    "branch-layout": "benches/fixtures/pgo/branch_layout.ck",
    "call-constant-length": "benches/fixtures/pgo/call_constant_length.ck",
    "trip-unroll-simd": "benches/oracles/fixtures/map_u32.ck",
    "memory-bound": "benches/oracles/fixtures/zip_u32.ck",
    "compute-bound": "benches/fixtures/pgo/compute_bound.ck",
}


def fail(message: str):
    raise ValueError(message)


def sha256(path: pathlib.Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def identity(path: pathlib.Path, name: str | None = None) -> dict:
    metadata = path.lstat()
    if not path.is_file() or path.is_symlink() or metadata.st_size <= 0:
        fail(f"evidence input must be a nonempty regular file: {path}")
    return {"path": name or path.name, "bytes": metadata.st_size, "sha256": sha256(path)}


def retain_cumulative_schema_seven(source: pathlib.Path, destination: pathlib.Path) -> pathlib.Path:
    identity(source)
    report = json.loads(source.read_text(encoding="utf-8"))
    directory = report.get("evidenceDirectory") if isinstance(report, dict) else None
    if not isinstance(directory, str) or re.fullmatch(r"measurement-[0-9]+-[0-9]+", directory) is None:
        fail("cumulative schema-7 evidenceDirectory is invalid")
    source_evidence = source.parent / directory
    if source_evidence.is_symlink() or not source_evidence.is_dir():
        fail("cumulative schema-7 evidenceDirectory must be a real directory")
    shutil.copytree(source_evidence, destination / directory, symlinks=True)
    retained = destination / "results-schema7.json"
    shutil.copy2(source, retained)
    return retained


def artifact(path: pathlib.Path, case: str, role: str) -> dict:
    record = identity(path)
    return {"case": case, "role": role, "file": record["path"],
            "bytes": record["bytes"], "sha256": record["sha256"]}


def named_digest(names: list[str]) -> str:
    digest = hashlib.sha256()
    for name in sorted(names):
        digest.update(name.encode())
        digest.update(b"\0")
        digest.update(sha256(REPO / name).encode())
        digest.update(b"\n")
    return digest.hexdigest()


def canonical_digest(value) -> str:
    return hashlib.sha256(
        json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    ).hexdigest()


def command_output(command: list[object], *, env=None, cwd=REPO) -> str:
    command = [str(item) for item in command]
    result = subprocess.run(
        command, cwd=cwd, env=env, text=True,
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False,
    )
    if result.returncode:
        fail(f"command failed ({result.returncode}): {' '.join(command)}\n{result.stdout[-6000:]}")
    return result.stdout


def terminated_child_cpu_time_ns():
    if resource is None:
        fail("terminated-child CPU timing requires a Unix performance worker")
    usage = resource.getrusage(resource.RUSAGE_CHILDREN)
    return round((usage.ru_utime + usage.ru_stime) * 1_000_000_000)


def clang_profile_runtime(clang):
    resource = pathlib.Path(command_output([clang, "--print-resource-dir"]).strip())
    candidates = sorted(resource.glob("lib/**/libclang_rt.profile*.a"))
    candidates += sorted(resource.glob("lib/**/clang_rt.profile*.lib"))
    if len(candidates) != 1:
        fail(f"pinned Clang resource directory must contain one host profile runtime: {resource}")
    return candidates[0]


def host_target() -> tuple[str, str, str]:
    os_name = {"Linux": "linux", "Darwin": "macos"}.get(platform.system())
    arch = {
        "x86_64": "x86_64", "amd64": "x86_64",
        "arm64": "aarch64", "aarch64": "aarch64",
    }.get(platform.machine().lower())
    if os_name is None or arch is None:
        fail(f"unsupported performance host {platform.system()}/{platform.machine()}")
    return f"{os_name}-{arch}", os_name, arch


def dynamic_suffix() -> str:
    return ".dylib" if platform.system() == "Darwin" else ".so"


def parse_cases() -> list[dict]:
    lines = (REPO / "benches/cases/pgo-cases.tsv").read_text(encoding="utf-8").splitlines()
    if not lines or lines[0] != "ckc-pgo-cases\t1":
        fail("unsupported PGO case manifest")
    cases = []
    for line in lines[1:]:
        if not line or line.startswith("#"):
            continue
        fields = line.split("\t")
        if len(fields) != 8:
            fail("malformed PGO case record")
        name, source, abi, pgo, eligible, x86, arm, batch = fields
        if source != SOURCE_PATHS.get(name) or not batch.isdigit():
            fail(f"PGO case identity mismatch: {name}")
        cases.append({
            "name": name, "source": REPO / source, "sourceName": source, "abi": abi,
            "pgoSensitive": pgo == "true", "eligible": eligible == "true",
            "x86Tier": x86, "armTier": arm, "batchCalls": int(batch),
        })
    if {case["name"] for case in cases} != set(SOURCE_PATHS):
        fail("PGO case manifest is incomplete")
    return cases


def parse_split(name: str) -> dict[str, list[dict]]:
    file = REPO / f"benches/fixtures/pgo/{name}.tsv"
    lines = file.read_text(encoding="utf-8").splitlines()
    expected = "held-out" if name == "held-out" else name
    if not lines or lines[0] != f"ckc-pgo-inputs\t1\t{expected}":
        fail(f"unsupported {name} workload split")
    result = {}
    for line in lines[1:]:
        if not line or line.startswith("#"):
            continue
        fields = line.split("\t")
        if len(fields) != 5 or fields[0] not in SOURCE_PATHS:
            fail(f"malformed {name} workload record")
        case, record, length, salt, parameter = fields
        result.setdefault(case, []).append({
            "record": record, "length": int(length), "salt": int(salt),
            "parameter": float(parameter),
        })
    return result


def capability_manifest(allow_baseline_diagnostic=False) -> tuple[dict, str]:
    target, _, arch = host_target()
    text = ""
    cpu_model = platform.processor() or platform.machine()
    if platform.system() == "Linux":
        text = pathlib.Path("/proc/cpuinfo").read_text(encoding="utf-8", errors="replace").lower()
        match = re.search(r"(?:model name|hardware)\s*:\s*(.+)", text)
        if match:
            cpu_model = match.group(1).strip()
    elif platform.system() == "Darwin":
        text = command_output(["sysctl", "-a"]).lower()
        match = re.search(r"machdep\.cpu\.brand_string:\s*(.+)", text)
        if match:
            cpu_model = match.group(1).strip()
    features = sorted(set(re.findall(
        r"\b(?:avx512f|avx512bw|avx512dq|avx512vl|avx2|fma|bmi2|sve2|sve)\b", text
    )))
    tiers = ["baseline"]
    os_state = []
    if arch == "x86_64":
        if {"avx2", "fma", "bmi2"}.issubset(features):
            tiers.append("x86-64-v3")
            os_state.extend(["xsave", "ymm"])
        if {"avx512f", "avx512bw", "avx512dq", "avx512vl"}.issubset(features):
            tiers.append("x86-64-v4")
            os_state.append("zmm")
    else:
        if "sve" in features:
            tiers.append("aarch64-sve")
            os_state.append("sve-enabled")
        if "sve2" in features:
            tiers.append("aarch64-sve2")
            os_state.append("sve2-enabled")
    if len(tiers) < 2 and allow_baseline_diagnostic:
        tiers.append("diagnostic-baseline-only")
        os_state.append("not-release-evidence")
    if len(tiers) < 2:
        fail(f"required enhanced performance tier is unavailable on {target}")
    material = {
        "schema": 1, "targetSetSchema": 1, "requiredTier": tiers[-1],
        "availableTiers": tiers, "features": features, "osState": sorted(set(os_state)),
        "resolverPolicy": "resolve-once-before-timing",
    }
    return dict(material, digest=canonical_digest(material)), cpu_model


def output_artifact(base: pathlib.Path, kind: str) -> pathlib.Path:
    if kind == "dynamic":
        return base.with_suffix(dynamic_suffix())
    if kind == "static":
        return base.with_suffix(".a")
    fail(f"unknown artifact kind {kind}")


def build_ck(compiler: pathlib.Path, case: dict, base: pathlib.Path, *, cpu: str,
             kind="dynamic", profile=None, generate=None, cache_root=None):
    command = [compiler, "build", case["source"], "--kind", kind, "--out", base,
               "-O3", "--cpu", cpu, "--overflow", "unchecked", "--bounds", "unchecked"]
    if profile is not None:
        command.extend(["--pgo-use", profile])
    if generate is not None:
        command.extend(["--pgo-generate", generate])
    environment = os.environ.copy()
    if cache_root is not None:
        cache_root.mkdir(parents=True, exist_ok=False)
        environment["XDG_CACHE_HOME"] = str(cache_root)
    timer = terminated_child_cpu_time_ns()
    output = command_output(command, env=environment)
    elapsed = terminated_child_cpu_time_ns() - timer
    if elapsed < 0:
        fail("terminated-child CPU clock moved backwards")
    result = output_artifact(base, kind)
    identity(result)
    return result, max(1, elapsed), output


class Kernel:
    def __init__(self, library: pathlib.Path, case: dict, record: dict):
        self.library = ctypes.CDLL(str(library))
        self.function = self.library.kernel
        self.case = case
        self.record = record
        self.keepalive = []
        self.arguments = self._arguments()

    def _u32(self, length, salt):
        array = (ctypes.c_uint32 * max(1, length))()
        for index in range(length):
            array[index] = ((index + salt) * 2_654_435_761) % 1_000_002 + 1
        self.keepalive.append(array)
        return array

    def _f64(self, length, salt):
        array = (ctypes.c_double * max(1, length))()
        for index in range(length):
            array[index] = (index - length / 2 + salt) / 16.0 + 0.25
        self.keepalive.append(array)
        return array

    def _branch_u64(self, length, value):
        array = (ctypes.c_uint64 * max(1, length))()
        for index in range(length):
            array[index] = value
        self.keepalive.append(array)
        return array

    def _arguments(self):
        length, salt = self.record["length"], self.record["salt"]
        abi = self.case["abi"]
        if abi == "slice-branch-u64":
            items = self._branch_u64(length, int(self.record["parameter"]))
            self.function.argtypes = [
                ctypes.POINTER(ctypes.c_uint64), ctypes.c_uint32,
                ctypes.c_uint32, ctypes.c_uint64,
            ]
            self.function.restype = ctypes.c_uint64
            return items, length, length, salt
        if abi in {"slice-fixed-u32", "slice-map-u32"}:
            actual = 4000 if abi == "slice-fixed-u32" else length
            if abi == "slice-fixed-u32":
                a = (ctypes.c_uint32 * actual)(
                    *([int(self.record["parameter"])] * actual)
                )
                self.keepalive.append(a)
            else:
                a = self._u32(actual, salt)
            out = self._u32(actual, 0)
            self.function.argtypes = [
                ctypes.POINTER(ctypes.c_uint32), ctypes.c_uint32,
                ctypes.POINTER(ctypes.c_uint32), ctypes.c_uint32,
            ] + ([ctypes.c_uint32] if abi == "slice-map-u32" else [])
            arguments = [a, actual, out, actual]
            if abi == "slice-map-u32":
                arguments.append(length)
            return tuple(arguments)
        if abi == "slice-zip-u32":
            a, b, out = self._u32(length, salt), self._u32(length, salt + 17), self._u32(length, 0)
            self.function.argtypes = [
                ctypes.POINTER(ctypes.c_uint32), ctypes.c_uint32,
                ctypes.POINTER(ctypes.c_uint32), ctypes.c_uint32,
                ctypes.POINTER(ctypes.c_uint32), ctypes.c_uint32, ctypes.c_uint32,
            ]
            return a, length, b, length, out, length, length
        if abi == "slice-f64":
            a, out = self._f64(length, salt), self._f64(length, 0)
            self.function.argtypes = [ctypes.POINTER(ctypes.c_double), ctypes.c_uint32,
                                      ctypes.POINTER(ctypes.c_double), ctypes.c_uint32,
                                      ctypes.c_uint32, ctypes.c_double]
            return a, length, out, length, length, self.record["parameter"]
        fail(f"unsupported PGO ABI {abi}")

    def invoke(self):
        return self.function(*self.arguments)

    def run_batch(self, calls):
        for _ in range(calls):
            self.invoke()

    def result_digest(self):
        result = self.invoke()
        if self.case["abi"] == "slice-branch-u64":
            data = int(result).to_bytes(8, "little", signed=False)
        else:
            data = bytes(self.keepalive[-1])
        return hashlib.sha256(data).hexdigest()

    def write_profile(self):
        try:
            flush = getattr(self.library, "__llvm_profile_write_file")
        except AttributeError:
            return
        flush.restype = ctypes.c_int
        if flush() != 0:
            fail("oracle profile runtime failed to write its raw profile")


def upper_median(values):
    ordered = sorted(values)
    return ordered[len(ordered) // 2]


def rotating(width, row):
    return [(row + offset) % width for offset in range(width)]


def measure_kernel(kernel, batch, calls_per_sample):
    attempts = []
    for _ in range(calls_per_sample):
        start = time.perf_counter_ns()
        kernel.run_batch(batch)
        attempts.append(time.perf_counter_ns() - start)
    return min(attempts)


def flush_name(header):
    match = re.search(r"ck_profile_flush_[0-9a-f]{64}", header.read_text(encoding="utf-8"))
    if match is None:
        fail(f"profile generation header has no exact flush symbol: {header}")
    return match.group(0)


def profile_target_set(compiler, profile):
    parsed = json.loads(command_output([compiler, "pgo", "inspect", profile, "--json"]))
    value = parsed.get("identity", {}).get("target", {}).get("targetSet")
    if not isinstance(value, str) or re.fullmatch(r"[0-9a-f]{64}", value) is None:
        fail("profile inspection omitted the target-set digest")
    return value


def train_profile(compiler, case, record, policy, evidence, warmup, samples, calls_per_sample):
    shard_dir = evidence / f"{case['name']}-{policy}-shards"
    shard_dir.mkdir()
    base = evidence / f"{case['name']}-{policy}-generation"
    library, _, _ = build_ck(compiler, case, base, cpu=policy, generate=shard_dir)
    kernel = Kernel(library, case, record)
    for _ in range(warmup):
        kernel.run_batch(case["batchCalls"])
    generation_samples = [
        measure_kernel(kernel, case["batchCalls"], calls_per_sample) for _ in range(samples)
    ]
    kernel.run_batch(max(128, case["batchCalls"]))
    symbol = getattr(kernel.library, flush_name(base.with_suffix(".h")))
    symbol.restype = ctypes.c_int32
    if symbol() != 0:
        fail(f"CK profile flush failed for {case['name']}/{policy}")
    shards = [item for item in shard_dir.iterdir() if item.is_file() and not item.is_symlink()]
    if len(shards) != 1:
        fail(f"expected exactly one completed shard for {case['name']}/{policy}")
    shard_copy = evidence / f"{case['name']}-{policy}.ckprof-part"
    shutil.copy2(shards[0], shard_copy)
    profile = evidence / f"{case['name']}-{policy}.ckprof"
    command_output([compiler, "pgo", "merge", shard_copy, "--out", profile])
    return profile, artifact(shard_copy, case["name"], policy), \
        artifact(profile, case["name"], policy), generation_samples


def train_oracle_subprocess(library, case, record, calls):
    command_output([
        sys.executable, "-B", pathlib.Path(__file__), "--train-library", library,
        "--case", case["name"], "--record-json", json.dumps(record, sort_keys=True),
        "--calls", calls,
    ])


def oracle_link_flags(output):
    if platform.system() == "Darwin":
        sdk = command_output(["xcrun", "--show-sdk-path"]).strip()
        return ["-dynamiclib", "-fPIC", "-isysroot", sdk,
                "-Wl,-adhoc_codesign", "-o", output]
    return ["-shared", "-fPIC", "-o", output]


def compile_clang_pgo(clang, profdata, case, training, evidence):
    manifest = tomllib.loads((REPO / "benches/oracles/pgo/manifest.toml").read_text())
    number = next(row["oracle_case"] for row in manifest["case"] if row["name"] == case["name"])
    raw = evidence / f"{case['name']}-clang.profraw"
    profile = evidence / f"{case['name']}-clang.profdata"
    generated = evidence / f"{case['name']}-clang-generation{dynamic_suffix()}"
    source = REPO / "benches/oracles/pgo/c/pgo_oracle.c"
    common = [clang, "-std=c11", "-O3", "-march=native", "-fno-fast-math",
              "-ffp-contract=off", "-fno-builtin", f"-DCK_PGO_ORACLE_CASE={number}", source]
    resource_override = os.environ.get("CKC_CLANG_RESOURCE_DIR")
    if resource_override:
        common.extend(["-resource-dir", resource_override])
    command_output(common + [f"-fprofile-instr-generate={raw}"] + oracle_link_flags(generated))
    train_oracle_subprocess(generated, case, training, max(128, case["batchCalls"]))
    if not raw.is_file():
        fail(f"Clang did not write training profile for {case['name']}")
    command_output([profdata, "merge", "-o", profile, raw])
    final = evidence / f"{case['name']}-clang-pgo{dynamic_suffix()}"
    command_output(common + [f"-fprofile-instr-use={profile}"] + oracle_link_flags(final))
    return final


def compile_rust_pgo(profdata, case, training, evidence):
    raw_dir = evidence / f"{case['name']}-rust-raw"
    raw_dir.mkdir()
    profile = evidence / f"{case['name']}-rust.profdata"
    generated = evidence / f"{case['name']}-rust-generation{dynamic_suffix()}"
    source = REPO / "benches/oracles/pgo/rust/pgo_oracle.rs"
    common = ["rustc", "+1.90.0", "--edition", "2024", "--crate-type", "cdylib",
              "-Awarnings", "-C", "opt-level=3", "-C", "target-cpu=native",
              "-C", "panic=abort", "-C", "llvm-args=-fp-contract=off",
              "-C", "llvm-args=-enable-name-compression=false",
              "--cfg", f'oracle_case="{case["name"]}"', source]
    command_output(common + ["-C", f"profile-generate={raw_dir}", "-o", generated])
    train_oracle_subprocess(generated, case, training, max(128, case["batchCalls"]))
    raw_files = sorted(raw_dir.glob("*.profraw"))
    if not raw_files:
        fail(f"Rust did not write training profile for {case['name']}")
    command_output([profdata, "merge", "-o", profile, *raw_files])
    final = evidence / f"{case['name']}-rust-pgo{dynamic_suffix()}"
    command_output(common + ["-C", f"profile-use={profile}", "-o", final])
    return final


def parse_v012_bundle(bundle):
    text = (bundle / "replay.tsv").read_text(encoding="utf-8")
    lines = text.splitlines()
    if not lines or lines[0] != "ckc-v012-runtime-replay\t2":
        fail("exact v0.12 replay bundle has the wrong schema")
    metadata, archive = {}, None
    for line in lines[1:]:
        fields = line.split("\t")
        if len(fields) == 2:
            metadata[fields[0]] = fields[1]
        elif len(fields) == 4 and fields[0] == "distributionArchive":
            archive = {"file": fields[1], "bytes": int(fields[2]), "sha256": fields[3]}
        else:
            fail("exact v0.12 replay bundle contains an unknown record")
    compiler = bundle / "ckc-v012"
    compiler_record = {"file": "ckc-v012", "bytes": compiler.stat().st_size, "sha256": sha256(compiler)}
    if archive is None:
        fail("exact v0.12 replay bundle has no archive")
    return compiler, {"metadata": metadata, "manifestSha256": hashlib.sha256(text.encode()).hexdigest(),
                      "compiler": compiler_record, "archive": archive}


def deterministic_archive(output, compiler):
    entries = [
        ("ckc-v0.13/ckc", compiler, 0o755),
        ("ckc-v0.13/LICENSE", REPO / "LICENSE", 0o644),
        ("ckc-v0.13/THIRD_PARTY_NOTICES.md", REPO / "THIRD_PARTY_NOTICES.md", 0o644),
    ]
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w", format=tarfile.PAX_FORMAT) as archive:
        for name, source, mode in entries:
            data = source.read_bytes()
            item = tarfile.TarInfo(name)
            item.size, item.mtime, item.mode = len(data), 0, mode
            item.uid = item.gid = 0
            item.uname = item.gname = ""
            archive.addfile(item, io.BytesIO(data))
    with output.open("wb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
            compressed.write(buffer.getvalue())


def collect_compile_samples(candidate, case, profiles, evidence, warmup, samples):
    modes = ["ordinary", "pgo", "multiversion", "combined"]
    warmup_order, sample_order = [], []
    streams = {mode: [] for mode in modes}
    serial = 0

    def one(mode):
        nonlocal serial
        serial += 1
        base = evidence / f"compile-{case['name']}-{mode}-{serial}"
        cache = evidence / f"cache-{case['name']}-{mode}-{serial}"
        cpu = "multiversion" if mode in {"multiversion", "combined"} else "baseline"
        profile = profiles["baseline"] if mode == "pgo" else profiles["multiversion"] if mode == "combined" else None
        _, elapsed, _ = build_ck(candidate, case, base, cpu=cpu, kind="static",
                                 profile=profile, cache_root=cache)
        return elapsed

    for row in range(warmup):
        sequence = rotating(4, row)
        for channel in sequence:
            one(modes[channel])
        warmup_order.append(sequence)
    for row in range(samples):
        sequence = rotating(4, row)
        for channel in sequence:
            streams[modes[channel]].append(one(modes[channel]))
        sample_order.append(sequence)
    result = {"case": case["name"], "warmupOrder": warmup_order, "sampleOrder": sample_order}
    for mode in modes:
        result[mode + "SamplesNs"] = streams[mode]
        result[mode + "MedianNs"] = upper_median(streams[mode])
    return result


def collect(output, quick):
    candidate = pathlib.Path(os.environ.get("CKC_CANDIDATE_COMPILER", REPO / "target/release/ckc"))
    candidate = candidate if candidate.is_absolute() else (REPO / candidate).resolve()
    if not command_output([candidate, "--version"]).startswith("ckc 0.13.0"):
        fail("CKC_CANDIDATE_COMPILER must identify ckc 0.13.0")
    candidate_sha = command_output(["git", "rev-parse", "HEAD"]).strip()
    if not re.fullmatch(r"[0-9a-f]{40}", candidate_sha):
        fail("candidate SHA is not exact")
    if os.environ.get("GITHUB_SHA") not in {None, candidate_sha}:
        fail("GITHUB_SHA does not match the checked-out candidate")
    prefix_raw, clang_raw, replay_raw = (os.environ.get(name) for name in [
        "CKC_LLVM_PREFIX", "CKC_CLANG_ORACLE", "CKC_V012_RUNTIME_BUNDLE"
    ])
    if not prefix_raw or not clang_raw or not replay_raw:
        fail("CKC_LLVM_PREFIX, CKC_CLANG_ORACLE, and CKC_V012_RUNTIME_BUNDLE are required")
    prefix, clang, replay_bundle = pathlib.Path(prefix_raw), pathlib.Path(clang_raw), pathlib.Path(replay_raw)
    if LLVM_VERSION not in command_output([clang, "--version"]).splitlines()[0]:
        fail("CKC_CLANG_ORACLE must identify Clang 22.1.8")
    if not command_output(["rustc", "+1.90.0", "--version"]).startswith("rustc 1.90.0 "):
        fail("Rust oracle must identify rustc 1.90.0")
    profdata = clang.parent / ("llvm-profdata.exe" if os.name == "nt" else "llvm-profdata")
    if not profdata.is_file():
        fail("pinned Clang oracle prefix has no llvm-profdata")
    profile_runtime = clang_profile_runtime(clang) if not quick else None
    replay_compiler, replay_report = parse_v012_bundle(replay_bundle)
    if replay_report["metadata"].get("commit") != V012_COMMIT:
        fail("replay bundle is not exact CK 0.12")
    output = output.absolute()
    output.parent.mkdir(parents=True, exist_ok=True)
    evidence = output.parent / f"v013-measurement-{int(time.time())}-{os.getpid()}"
    evidence.mkdir()
    warmup, samples, calls_per_sample = (1, 5, 3) if quick else (3, 20, 7)
    compile_warmup, compile_count = (1, 3) if quick else (3, 15)
    cases = parse_cases()
    splits = {name: parse_split(name) for name in ["training", "held-out", "adversarial"]}
    capability, cpu_model = capability_manifest(quick)
    command_output([sys.executable, "-B", REPO / "scripts/audit-performance-oracles.py",
                    "--pgo", "--clang", clang])

    candidate_copy = evidence / "ckc-v013"
    shutil.copy2(candidate, candidate_copy)
    candidate_binary = {"file": candidate_copy.name, "bytes": candidate_copy.stat().st_size,
                        "sha256": sha256(candidate_copy)}
    cumulative_source = REPO / "target/ckc-perf/results-baseline.json"
    if not cumulative_source.is_file():
        cumulative_source = REPO / "target/ckc-perf/results.json"
    cumulative = retain_cumulative_schema_seven(cumulative_source, evidence)

    training_records, profile_records, target_sets = [], [], []
    profiles_by_case, generation_by_case = {}, {}
    for case in cases:
        profiles_by_case[case["name"]] = {}
        training_record = splits["training"][case["name"]][0]
        for policy in ["baseline", "multiversion"]:
            profile, shard_record, profile_record, generation = train_profile(
                candidate, case, training_record, policy, evidence,
                warmup, samples, calls_per_sample,
            )
            profiles_by_case[case["name"]][policy] = profile
            training_records.append(shard_record)
            profile_records.append(profile_record)
            target_sets.append({
                "case": case["name"], "policy": policy, "schema": 1,
                "digest": profile_target_set(candidate, profile),
                "tiers": ["baseline"] if policy == "baseline" else capability["availableTiers"],
            })
            if policy == "baseline":
                generation_by_case[case["name"]] = generation

    case_rows, compile_rows, size_rows, variant_records = [], [], [], []
    role_names = {
        "ordinary": "ordinary", "pgo": "pgo", "multiversion": "multiversion",
        "combined": "combined", "selectedDirect": "selected-direct",
        "clangPgo": "clang-pgo", "rustPgo": "rust-pgo",
    }
    for case in cases:
        name, profiles = case["name"], profiles_by_case[case["name"]]
        artifacts = {}
        for role, compiler, cpu, profile in [
            ("ordinary", candidate, "baseline", None),
            ("replayV012", replay_compiler, "baseline", None),
            ("pgo", candidate, "baseline", profiles["baseline"]),
            ("multiversion", candidate, "multiversion", None),
            ("combined", candidate, "multiversion", profiles["multiversion"]),
            ("selectedDirect", candidate, "native", None),
        ]:
            artifacts[role], _, _ = build_ck(
                compiler, case, evidence / f"{name}-{role}", cpu=cpu, profile=profile
            )
        training_record = splits["training"][name][0]
        artifacts["clangPgo"] = compile_clang_pgo(clang, profdata, case, training_record, evidence)
        artifacts["rustPgo"] = compile_rust_pgo(profdata, case, training_record, evidence)
        for channel, role in role_names.items():
            variant_records.append(artifact(artifacts[channel], name, role))

        for split_name in ["training", "held-out", "adversarial"]:
            for record in splits[split_name].get(name, []):
                digests = {channel: Kernel(artifacts[channel], case, record).result_digest()
                           for channel in CHANNELS}
                if len(set(digests.values())) != 1:
                    fail(f"differential result mismatch for {name}/{split_name}: {digests}")

        held = splits["held-out"][name][0]
        kernels = [Kernel(artifacts[channel], case, held) for channel in CHANNELS]
        resolver_calls = 0
        if case["eligible"]:
            kernels[CHANNELS.index("multiversion")].invoke()
            resolver_calls = 1
        warmup_order, sample_order = [], []
        streams = [[] for _ in CHANNELS]
        for row in range(warmup):
            sequence = rotating(len(CHANNELS), row)
            for channel in sequence:
                kernels[channel].run_batch(case["batchCalls"])
            warmup_order.append(sequence)
        for row in range(samples):
            sequence = rotating(len(CHANNELS), row)
            for channel in sequence:
                streams[channel].append(
                    measure_kernel(kernels[channel], case["batchCalls"], calls_per_sample)
                )
            sample_order.append(sequence)
        row = {
            "name": name, "pgoSensitive": case["pgoSensitive"],
            "multiversionEligible": case["eligible"], "heldOutOnly": True,
            "referenceEquivalent": True, "batchCalls": case["batchCalls"],
            "resultDigest": kernels[0].result_digest(), "resolverCalls": resolver_calls,
            "warmupOrder": warmup_order, "sampleOrder": sample_order,
            "generationSamplesNs": generation_by_case[name],
            "generationMedianNs": upper_median(generation_by_case[name]),
        }
        for index, channel in enumerate(CHANNELS):
            row[channel + "SamplesNs"] = streams[index]
            row[channel + "MedianNs"] = upper_median(streams[index])
        case_rows.append(row)
        compile_rows.append(collect_compile_samples(
            candidate, case, profiles, evidence, compile_warmup, compile_count
        ))
        size_rows.append({
            "case": name, "ordinaryBytes": artifacts["ordinary"].stat().st_size,
            "pgoBytes": artifacts["pgo"].stat().st_size,
            "multiversionBytes": artifacts["multiversion"].stat().st_size,
            "combinedBytes": artifacts["combined"].stat().st_size,
        })

    archive_path = evidence / "ckc-v013-distribution.tar.gz"
    deterministic_archive(archive_path, candidate)
    replay_archive = replay_report["archive"]
    report = {
        "schemaVersion": 8, "candidateVersion": "0.13.0", "candidateSha": candidate_sha,
        "replayCommit": V012_COMMIT, "evidenceDirectory": evidence.name,
        "toolchain": {
            "llvmVersion": LLVM_VERSION, "clangVersion": LLVM_VERSION,
            "rustVersion": RUST_VERSION,
            "componentManifestSha256": sha256(prefix / "share/ckc/llvm-build.toml"),
            "clangProfileRuntimeSha256": sha256(profile_runtime) if profile_runtime else "0" * 64,
        },
        "hardware": {
            "target": host_target()[0], "arch": host_target()[2], "os": host_target()[1],
            "cpuModel": cpu_model, "logicalCpus": os.cpu_count() or 1,
        },
        "capabilityManifest": capability,
        "recipe": {
            "schema": 1, "files": [identity(REPO / name, name) for name in RECIPE_FILES],
            "digest": named_digest(RECIPE_FILES), "thresholds": THRESHOLDS,
        },
        "workload": {
            "manifest": identity(REPO / "benches/cases/pgo-cases.tsv", "benches/cases/pgo-cases.tsv"),
            "sources": [identity(REPO / name, name) for name in SOURCE_PATHS.values()],
            "training": identity(REPO / "benches/fixtures/pgo/training.tsv", "benches/fixtures/pgo/training.tsv"),
            "heldOut": identity(REPO / "benches/fixtures/pgo/held-out.tsv", "benches/fixtures/pgo/held-out.tsv"),
            "adversarial": identity(REPO / "benches/fixtures/pgo/adversarial.tsv", "benches/fixtures/pgo/adversarial.tsv"),
        },
        "candidateBinary": candidate_binary, "replayBundle": replay_report,
        "cumulativeSchemaSeven": {"file": cumulative.name, "bytes": cumulative.stat().st_size,
                                  "sha256": sha256(cumulative)},
        "trainingShards": training_records, "finalProfiles": profile_records,
        "targetSets": target_sets, "variantObjects": variant_records,
        "sampling": {
            "protocol": "rotating-eight-channel-v1", "warmupRows": warmup,
            "sampleRows": samples, "callsPerSample": calls_per_sample,
            "channelNames": CHANNELS,
            "stabilityPolicy": "at-least-80-percent-within-25-percent-of-median",
            "rerunPolicy": "unstable-evidence-is-invalid-no-selective-rerun",
        },
        "cases": case_rows, "compileTime": compile_rows, "artifactSize": size_rows,
        "archiveSize": {
            "candidateFile": archive_path.name, "candidateBytes": archive_path.stat().st_size,
            "candidateSha256": sha256(archive_path), "replayFile": replay_archive["file"],
            "replayBytes": replay_archive["bytes"], "replaySha256": replay_archive["sha256"],
        },
        "correctness": {
            "training": True, "heldOut": True, "adversarial": True,
            "differential": True, "ubAudit": True, "featureAudit": True,
        },
    }
    output.write_text(json.dumps(report, sort_keys=True, separators=(",", ":")) + "\n",
                      encoding="utf-8")
    print(f"wrote schema-8 raw performance evidence: {output}")


def main() -> int:
    if "--train-library" in sys.argv:
        parser = argparse.ArgumentParser(add_help=False)
        parser.add_argument("--train-library", type=pathlib.Path, required=True)
        parser.add_argument("--case", required=True)
        parser.add_argument("--record-json", required=True)
        parser.add_argument("--calls", type=int, required=True)
        args = parser.parse_args()
        try:
            case = next(item for item in parse_cases() if item["name"] == args.case)
            kernel = Kernel(args.train_library, case, json.loads(args.record_json))
            kernel.run_batch(args.calls)
            kernel.write_profile()
        except (OSError, ValueError, StopIteration, json.JSONDecodeError) as error:
            print(f"oracle training child failed: {error}", file=sys.stderr)
            return 1
        return 0
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--task", choices=("collect",), required=True)
    parser.add_argument("--out", type=pathlib.Path, required=True)
    parser.add_argument("--quick", action="store_true")
    args = parser.parse_args()
    try:
        collect(args.out, args.quick)
    except (OSError, ValueError, subprocess.SubprocessError, json.JSONDecodeError) as error:
        print(f"schema-8 measurement failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
