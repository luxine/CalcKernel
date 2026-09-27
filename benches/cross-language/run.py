#!/usr/bin/env python3
"""Build and compare the checked-in CalcKernel benchmark kernels."""

from __future__ import annotations

import argparse
import array
import ctypes
import datetime as dt
import hashlib
import importlib.util
import json
import os
import platform
import random
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from pathlib import PureWindowsPath
from statistics import median
from typing import Any, Callable


ROOT = Path(__file__).resolve().parents[2]
BENCH_ROOT = Path(__file__).resolve().parent
KERNEL_ROOT = BENCH_ROOT / "kernels"
SIZE = {"matmul": 256, "convolve": 1024}
LANGUAGES = ("ck", "cpp", "rust", "js", "java", "numpy")
MODES = ("ordinary", "tuned")
THREAD_ENV = (
    "VECLIB_MAXIMUM_THREADS",
    "OMP_NUM_THREADS",
    "OPENBLAS_NUM_THREADS",
    "MKL_NUM_THREADS",
    "BLIS_NUM_THREADS",
    "NUMEXPR_NUM_THREADS",
)
MATRIX_DESCRIPTION = "256x256 row-major f64 matrix multiplication with fixed dyadic inputs"
CONVOLVE_DESCRIPTION = "1024x1024 f64 3x3 Gaussian stencil; output border is zero"


def set_single_thread_environment() -> None:
    for name in THREAD_ENV:
        os.environ[name] = "1"


def run_command(command: list[str], *, cwd: Path = ROOT) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(command, cwd=cwd, text=True, capture_output=True)
    if result.returncode != 0:
        raise RuntimeError(
            f"command failed ({result.returncode}): {json.dumps(command)}\n"
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        )
    return result


def command_version(command: str) -> dict[str, str]:
    path = shutil.which(command)
    if path is None:
        raise RuntimeError(f"required command not found on PATH: {command}")
    result = subprocess.run([path, "--version"], text=True, capture_output=True)
    output = (result.stdout or result.stderr).strip()
    if result.returncode != 0:
        raise RuntimeError(f"cannot read version for {path}: {output}")
    return {"binary": Path(path).name, "version": output.splitlines()[0] if output else "unknown"}


def resolve_program(explicit: str | None, candidates: tuple[str, ...], label: str) -> str:
    if explicit:
        found = shutil.which(explicit) or (explicit if Path(explicit).is_file() else None)
        if not found:
            raise RuntimeError(f"{label} not found: {explicit}")
        return str(Path(found).absolute())
    for candidate in candidates:
        found = shutil.which(candidate)
        if found:
            return str(Path(found).absolute())
    raise RuntimeError(f"could not find {label}; tried {', '.join(candidates)}")


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    return sha256_bytes(path.read_bytes())


def source_hashes() -> dict[str, str]:
    source_suffixes = {".ck", ".cpp", ".rs", ".mjs", ".java", ".py"}
    paths = sorted(
        path
        for path in KERNEL_ROOT.rglob("*")
        if path.is_file() and path.suffix in source_suffixes and "__pycache__" not in path.parts
    )
    hashes = {path.relative_to(ROOT).as_posix(): sha256_file(path) for path in paths}
    hashes[Path(__file__).resolve().relative_to(ROOT).as_posix()] = sha256_file(Path(__file__).resolve())
    return hashes


def dynamic_suffix() -> str:
    if sys.platform == "darwin":
        return ".dylib"
    if sys.platform.startswith("linux"):
        return ".so"
    raise RuntimeError("this runner currently supports macOS and Linux shared libraries")


def baseline_flags() -> tuple[list[str], str]:
    machine = platform.machine().lower()
    if machine in ("x86_64", "amd64"):
        return ["-march=x86-64", "-mtune=generic"], "x86-64"
    if machine in ("arm64", "aarch64"):
        return ["-march=armv8-a"], "armv8-a"
    return [], "compiler-default"


def build_batcher(cxx: str, build_dir: Path, commands: list[dict[str, Any]]) -> ctypes.CDLL:
    source = build_dir / "batcher.c"
    source.write_text(
        """#include <stdint.h>
typedef void (*matmul_fn)(const double*, const double*, double*, int32_t);
typedef void (*convolve_fn)(const double*, double*, int32_t);
void bench_matmul_batch(matmul_fn f, const double* a, const double* b, double* out, int32_t n, int32_t repeat) {
  for (int32_t i = 0; i < repeat; ++i) f(a, b, out, n);
}
void bench_convolve_batch(convolve_fn f, const double* input, double* out, int32_t n, int32_t repeat) {
  for (int32_t i = 0; i < repeat; ++i) f(input, out, n);
}
"""
    )
    out = build_dir / f"libck_benchmark_batcher{dynamic_suffix()}"
    link_kind = "-dynamiclib" if sys.platform == "darwin" else "-shared"
    command = [cxx, "-O2", "-fno-fast-math", "-ffp-contract=off", link_kind, "-fPIC", str(source), "-o", str(out)]
    result = run_command(command)
    commands.append(command_record("benchmark-batcher", command, result))
    library = ctypes.CDLL(str(out))
    library.bench_matmul_batch.argtypes = [
        ctypes.c_void_p,
        ctypes.POINTER(ctypes.c_double),
        ctypes.POINTER(ctypes.c_double),
        ctypes.POINTER(ctypes.c_double),
        ctypes.c_int32,
        ctypes.c_int32,
    ]
    library.bench_matmul_batch.restype = None
    library.bench_convolve_batch.argtypes = [
        ctypes.c_void_p,
        ctypes.POINTER(ctypes.c_double),
        ctypes.POINTER(ctypes.c_double),
        ctypes.c_int32,
        ctypes.c_int32,
    ]
    library.bench_convolve_batch.restype = None
    return library


def command_record(
    name: str,
    command: list[str],
    result: subprocess.CompletedProcess[str],
    *,
    build_dir: Path | None = None,
) -> dict[str, Any]:
    normalized: list[str] = []
    for index, argument in enumerate(command):
        if index == 0:
            normalized.append(Path(argument).name)
            continue
        candidate = Path(argument)
        if candidate.is_absolute():
            if build_dir is not None:
                try:
                    relative_to_build = candidate.resolve().relative_to(build_dir.resolve())
                    normalized.append("<build-dir>" if str(relative_to_build) == "." else f"<build-dir>/{relative_to_build.as_posix()}")
                    continue
                except ValueError:
                    pass
            try:
                normalized.append(candidate.resolve().relative_to(ROOT).as_posix())
            except ValueError:
                normalized.append(f"<build-dir>/{candidate.name}")
        else:
            normalized.append(argument)
    return {
        "name": name,
        "command": normalized,
        "cwd": ".",
        "exitCode": result.returncode,
    }


ABSOLUTE_PATH_RE = re.compile(r"(?<![A-Za-z0-9.])/(?:[^\s\"'`,;)}]+)|[A-Za-z]:[\\](?:[^\s\"'`,;)}]+)")
QUOTED_PATH_RE = re.compile(r"(?P<quote>[\"'])(?P<path>(?:/(?!/)|[A-Za-z]:[\\])[^\"'\r\n]*)(?P=quote)")
UNQUOTED_PATH_RE = re.compile(r"(?<![:/])/(?:[^\s,;)}\]\"']+)(?:\s+[^\r\n,;)}\]\"']+)*")
WINDOWS_PATH_RE = re.compile(r"(?<![A-Za-z0-9])[A-Za-z]:[\\][^\r\n,;)}\]\"']+")


def redact_absolute_paths(value: str) -> str:
    if Path(value).is_absolute() or PureWindowsPath(value).is_absolute():
        return "<absolute-path>"

    value = QUOTED_PATH_RE.sub(lambda match: f"{match.group('quote')}<absolute-path>{match.group('quote')}", value)
    value = UNQUOTED_PATH_RE.sub("<absolute-path>", value)
    value = WINDOWS_PATH_RE.sub("<absolute-path>", value)
    value = ABSOLUTE_PATH_RE.sub("<absolute-path>", value)
    return value


def sanitize_numpy_config(config: str) -> str:
    """Remove host-specific paths from both JSON and legacy text output."""
    try:
        parsed = json.loads(config)
    except json.JSONDecodeError:
        return redact_absolute_paths(config)

    def sanitize(value: Any) -> Any:
        if isinstance(value, str):
            return redact_absolute_paths(value)
        if isinstance(value, list):
            return [sanitize(item) for item in value]
        if isinstance(value, dict):
            return {redact_absolute_paths(str(key)): sanitize(item) for key, item in value.items()}
        return value

    return json.dumps(sanitize(parsed), indent=2, sort_keys=True)


def compile_library(
    name: str,
    command: list[str],
    output: Path,
    commands: list[dict[str, Any]],
) -> Path:
    result = run_command(command)
    commands.append(command_record(name, command, result))
    if not output.exists():
        raise RuntimeError(f"compiler succeeded but did not create expected library: {output}")
    return output


def numpy_worker_source() -> str:
    return r'''from __future__ import annotations
import contextlib, hashlib, importlib.util, json, os, sys, time
for name in ("VECLIB_MAXIMUM_THREADS", "OMP_NUM_THREADS", "OPENBLAS_NUM_THREADS", "MKL_NUM_THREADS", "BLIS_NUM_THREADS", "NUMEXPR_NUM_THREADS"):
    os.environ[name] = "1"
import numpy as np

module_path = sys.argv[2] if len(sys.argv) > 2 and sys.argv[1] == "--_numpy-worker" else sys.argv[1]
spec = importlib.util.spec_from_file_location("ck_benchmark_numpy", module_path)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
active = None

def canonical_hash(values):
    return hashlib.sha256(np.asarray(values, dtype="<f8", order="C").tobytes(order="C")).hexdigest()

def make_case(case, n):
    if case == "matmul":
        rows = np.arange(n, dtype=np.int64)[:, None]
        cols = np.arange(n, dtype=np.int64)[None, :]
        a = ((((rows * 17 + cols * 13) % 31) - 15) / 32.0).astype(np.float64, copy=False)
        b = ((((rows * 11 + cols * 7) % 29) - 14) / 32.0).astype(np.float64, copy=False)
        ref = np.matmul(a, b)
        return np.ascontiguousarray(a), np.ascontiguousarray(b), np.zeros((n, n), dtype=np.float64), np.ascontiguousarray(ref)
    index = np.arange(n * n, dtype=np.int64).reshape((n, n))
    source = ((((index * 17) % 37) - 18) / 32.0).astype(np.float64, copy=False)
    ref = np.zeros((n, n), dtype=np.float64)
    target = ref[1:-1, 1:-1]
    windows = (
        (source[:-2, :-2], 1.0), (source[:-2, 1:-1], 2.0), (source[:-2, 2:], 1.0),
        (source[1:-1, :-2], 2.0), (source[1:-1, 1:-1], 4.0), (source[1:-1, 2:], 2.0),
        (source[2:, :-2], 1.0), (source[2:, 1:-1], 2.0), (source[2:, 2:], 1.0),
    )
    for window, weight in windows:
        np.add(target, window * weight, out=target)
    np.divide(target, 16.0, out=target)
    return np.ascontiguousarray(source), None, np.zeros((n, n), dtype=np.float64), ref

def handle(command):
    global active
    if command.get("cmd") == "init":
        case, n = command["case"], int(command["size"])
        a, b, out, reference = make_case(case, n)
        active = (case, n, a, b, out, reference)
        # Warm each route and allocate any reusable NumPy scratch before calibration.
        for mode in ("ordinary", "tuned"):
            fn = getattr(module, case + "_" + mode)
            if case == "matmul": fn(a, b, out, n)
            else: fn(a, out, n)
        config = None
        stream = __import__("io").StringIO()
        with contextlib.redirect_stdout(stream):
            np.__config__.show()
        config = stream.getvalue().strip()
        return {"ready": True, "referenceHash": canonical_hash(reference), "numpyVersion": np.__version__, "numpyConfig": config}
    if command.get("cmd") == "run":
        if active is None: raise RuntimeError("send init before run")
        case, n, a, b, out, _ = active
        mode, repeat = command["mode"], int(command["repeat"])
        fn = getattr(module, case + "_" + mode)
        started = time.perf_counter_ns()
        for _ in range(repeat):
            if case == "matmul": fn(a, b, out, n)
            else: fn(a, out, n)
        elapsed = time.perf_counter_ns() - started
        return {"durationNs": elapsed, "sha256": canonical_hash(out)}
    raise RuntimeError("cmd must be init or run")

for line in sys.stdin:
    try:
        print(json.dumps(handle(json.loads(line)), separators=(",", ":")), flush=True)
    except Exception as error:
        print(json.dumps({"error": str(error)}), flush=True)
'''


class JsonLineWorker:
    def __init__(self, command: list[str], env: dict[str, str]):
        self.process = subprocess.Popen(
            command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
            env=env,
        )
        self.command = command

    def request(self, payload: dict[str, Any]) -> dict[str, Any]:
        if self.process.poll() is not None:
            stderr = self.process.stderr.read() if self.process.stderr else ""
            raise RuntimeError(f"worker exited ({self.process.returncode}): {stderr}")
        assert self.process.stdin is not None and self.process.stdout is not None
        self.process.stdin.write(json.dumps(payload, separators=(",", ":")) + "\n")
        self.process.stdin.flush()
        line = self.process.stdout.readline()
        if not line:
            stderr = self.process.stderr.read() if self.process.stderr else ""
            raise RuntimeError(f"worker closed stdout: {stderr}")
        response = json.loads(line)
        if "error" in response:
            raise RuntimeError(f"worker error: {response['error']}")
        return response

    def close(self) -> None:
        if self.process.stdin:
            self.process.stdin.close()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.terminate()
            self.process.wait(timeout=5)


def python_numpy_interpreter(requested: str | None) -> tuple[str, str]:
    candidates = [requested] if requested else [sys.executable, "python3", "python"]
    seen: set[str] = set()
    for candidate in candidates:
        if not candidate:
            continue
        found = shutil.which(candidate) or (candidate if Path(candidate).is_file() else None)
        if not found:
            continue
        path = str(Path(found).resolve())
        if path in seen:
            continue
        seen.add(path)
        probe = subprocess.run(
            [path, "-c", "import numpy; print(numpy.__version__)"],
            text=True,
            capture_output=True,
            env=os.environ.copy(),
        )
        if probe.returncode == 0:
            return path, probe.stdout.strip().splitlines()[-1]
    raise RuntimeError("NumPy is unavailable; pass --python with an interpreter that can import numpy")


def numeric_java_version(value: str) -> tuple[int, ...]:
    match = re.search(r"(?<![A-Za-z0-9])([0-9]+(?:\.[0-9]+){0,3})(?![0-9])", value)
    if not match:
        raise RuntimeError(f"cannot parse Java version: {value}")
    return tuple(int(part) for part in match.group(1).split("."))


def java_toolchain(java_requested: str | None, javac_requested: str | None) -> tuple[str, str, dict[str, str], dict[str, str]]:
    java = resolve_program(java_requested, ("java",), "Java runtime")
    javac = resolve_program(javac_requested, ("javac",), "Java compiler")
    java_info = command_version(java)
    javac_info = command_version(javac)
    java_version = numeric_java_version(java_info["version"])
    javac_version = numeric_java_version(javac_info["version"])
    if java_version[0] != 21 or javac_version[0] != 21:
        raise RuntimeError(
            "the benchmark requires a matching OpenJDK 21 runtime and compiler; "
            f"found java {java_info['version']!r} and javac {javac_info['version']!r}"
        )
    if java_version != javac_version:
        raise RuntimeError(
            "java and javac must come from the same OpenJDK 21 release; "
            f"found java {java_info['version']!r} and javac {javac_info['version']!r}"
        )
    return java, javac, java_info, javac_info


def native_input(case: str, n: int) -> tuple[array.array, array.array | None]:
    if array.array("d").itemsize != ctypes.sizeof(ctypes.c_double):
        raise RuntimeError("array('d') is not the host C double size")
    if case == "matmul":
        a = array.array("d", ((((row * 17 + col * 13) % 31) - 15) / 32.0 for row in range(n) for col in range(n)))
        b = array.array("d", ((((row * 11 + col * 7) % 29) - 14) / 32.0 for row in range(n) for col in range(n)))
        return a, b
    values = array.array("d", ((((index * 17) % 37) - 18) / 32.0 for index in range(n * n)))
    return values, None


def as_double_pointer(buffer: array.array) -> ctypes.POINTER(ctypes.c_double):
    return ctypes.cast(buffer.buffer_info()[0], ctypes.POINTER(ctypes.c_double))


def digest_native_output(output: array.array) -> str:
    if sys.byteorder == "little":
        return sha256_bytes(output.tobytes())
    copy = array.array("d", output)
    copy.byteswap()
    return sha256_bytes(copy.tobytes())


def native_reference_sha(worker: JsonLineWorker, case: str, n: int) -> str:
    response = worker.request({"cmd": "init", "case": case, "size": n})
    return response["referenceHash"]


def get_host() -> dict[str, Any]:
    uname = platform.uname()
    cpu_model = platform.processor() or None
    commands = []
    if sys.platform == "darwin":
        commands = [["sysctl", "-n", "machdep.cpu.brand_string"], ["sysctl", "-n", "hw.model"], ["sysctl", "-n", "hw.memsize"]]
    elif Path("/proc/cpuinfo").exists():
        for line in Path("/proc/cpuinfo").read_text(errors="replace").splitlines():
            if line.lower().startswith(("model name", "hardware")):
                cpu_model = line.split(":", 1)[-1].strip()
                break
    for command in commands:
        result = subprocess.run(command, text=True, capture_output=True)
        if result.returncode == 0 and result.stdout.strip():
            value = result.stdout.strip()
            if command[-1] == "machdep.cpu.brand_string":
                cpu_model = value
            elif command[-1] == "hw.model":
                model_id = value
            elif command[-1] == "hw.memsize":
                memory_bytes = int(value)
    result = {
        "system": uname.system,
        "release": uname.release,
        "machine": uname.machine,
        "processor": cpu_model,
        "logicalCpuCount": os.cpu_count(),
    }
    if sys.platform == "darwin":
        if "model_id" in locals():
            result["modelId"] = model_id
        if "memory_bytes" in locals():
            result["memoryBytes"] = memory_bytes
    return result


def build_specs(args: argparse.Namespace, build_dir: Path, commands: list[dict[str, Any]]) -> dict[tuple[str, str], Path]:
    ckc = resolve_program(args.ckc or os.environ.get("CKC"), ("ckc",), "ckc compiler")
    cxx = resolve_program(args.cxx or os.environ.get("CXX"), ("clang++", "g++", "c++"), "C++ compiler")
    rustc = resolve_program(args.rustc, ("rustc",), "rustc")
    clang = resolve_program(None, ("clang",), "Clang linker")
    node = resolve_program(args.node, ("node",), "Node.js")
    java, javac, java_info, javac_info = java_toolchain(args.java, args.javac)
    ext = dynamic_suffix()
    flags, baseline_name = baseline_flags()
    outputs: dict[tuple[str, str], Path] = {}

    # Record tool versions through the commands actually selected.
    versions = {
        "ckc": command_version(ckc),
        "cpp": command_version(cxx),
        "rust": command_version(rustc),
        "clang": command_version(clang),
        "node": command_version(node),
        "java": java_info,
        "javac": javac_info,
    }
    args._resolved = {"ckc": ckc, "cxx": cxx, "rustc": rustc, "clang": clang, "node": node, "java": java, "javac": javac, "versions": versions, "baseline": baseline_name}

    for mode in MODES:
        source = KERNEL_ROOT / "ck" / f"{mode}.ck"
        out = build_dir / f"libck_{mode}{ext}"
        object_out = build_dir / f"ck_{mode}.o"
        cpu = "baseline" if mode == "ordinary" else "native"
        object_command = [ckc, "build", str(source), "--kind", "object", "--cpu", cpu, "-O3", "--out", str(object_out)]
        result = run_command(object_command)
        commands.append(command_record(f"ck-{mode}-object", object_command, result))
        if not object_out.exists():
            raise RuntimeError(f"ckc succeeded but did not create expected object: {object_out}")
        link_kind = "-dynamiclib" if sys.platform == "darwin" else "-shared"
        link_command = [clang, link_kind, str(object_out), "-o", str(out)]
        outputs[("ck", mode)] = compile_library(f"ck-{mode}-link", link_command, out, commands)

    for mode in MODES:
        source = KERNEL_ROOT / "cpp" / f"{mode}.cpp"
        out = build_dir / f"libcpp_{mode}{ext}"
        cpu_flags = flags if mode == "ordinary" else (["-march=native"] if baseline_name == "x86-64" else (["-mcpu=native"] if baseline_name == "armv8-a" else []))
        command = [cxx, "-std=c++20", "-O3", "-fno-fast-math", "-ffp-contract=off", *cpu_flags, "-shared", "-fPIC", str(source), "-o", str(out)]
        outputs[("cpp", mode)] = compile_library(f"cpp-{mode}", command, out, commands)

    for mode in MODES:
        source = KERNEL_ROOT / "rust" / f"{mode}.rs"
        out = build_dir / f"librust_{mode}{ext}"
        cpu = ("x86-64" if baseline_name == "x86-64" else "generic") if mode == "ordinary" else "native"
        command = [rustc, "--crate-type", "cdylib", "--edition", "2021", "-C", "opt-level=3", "-C", f"target-cpu={cpu}", str(source), "-o", str(out)]
        outputs[("rust", mode)] = compile_library(f"rust-{mode}", command, out, commands)

    java_source = KERNEL_ROOT / "java" / "BenchmarkWorker.java"
    java_command = [javac, "-d", str(build_dir), str(java_source)]
    java_result = run_command(java_command)
    commands.append(command_record("java-worker", java_command, java_result, build_dir=build_dir))
    if not (build_dir / "BenchmarkWorker.class").exists():
        raise RuntimeError(f"javac succeeded but did not create expected worker class: {build_dir / 'BenchmarkWorker.class'}")
    for mode in MODES:
        outputs[("java", mode)] = java_source

    outputs[("js", "ordinary")] = KERNEL_ROOT / "javascript" / "worker.mjs"
    outputs[("js", "tuned")] = outputs[("js", "ordinary")]
    outputs[("numpy", "ordinary")] = KERNEL_ROOT / "python" / "numpy_impl.py"
    outputs[("numpy", "tuned")] = outputs[("numpy", "ordinary")]
    batcher = build_batcher(clang, build_dir, commands)
    args._batcher = batcher
    return outputs


class NativeCase:
    def __init__(self, library_path: Path, batcher: ctypes.CDLL, case: str, n: int, inputs: array.array, second: array.array | None):
        self.library = ctypes.CDLL(str(library_path))
        self.batcher = batcher
        self.case = case
        self.n = n
        self.output = array.array("d", [0.0]) * (n * n)
        self.input_ptr = as_double_pointer(inputs)
        self.second_ptr = as_double_pointer(second) if second is not None else None
        self.output_ptr = as_double_pointer(self.output)
        if case == "matmul":
            self.function = getattr(self.library, "matmul")
            self.function.argtypes = [ctypes.POINTER(ctypes.c_double), ctypes.POINTER(ctypes.c_double), ctypes.POINTER(ctypes.c_double), ctypes.c_int32]
            self.function.restype = None
            self.function_ptr = ctypes.cast(self.function, ctypes.c_void_p)
        else:
            self.function = getattr(self.library, "convolve")
            self.function.argtypes = [ctypes.POINTER(ctypes.c_double), ctypes.POINTER(ctypes.c_double), ctypes.c_int32]
            self.function.restype = None
            self.function_ptr = ctypes.cast(self.function, ctypes.c_void_p)

    def batch(self, repeat: int) -> int:
        started = time.perf_counter_ns()
        if self.case == "matmul":
            self.batcher.bench_matmul_batch(self.function_ptr, self.input_ptr, self.second_ptr, self.output_ptr, self.n, repeat)
        else:
            self.batcher.bench_convolve_batch(self.function_ptr, self.input_ptr, self.output_ptr, self.n, repeat)
        return time.perf_counter_ns() - started

    def digest(self) -> str:
        return digest_native_output(self.output)


class WorkerCase:
    def __init__(self, worker: JsonLineWorker, case: str, n: int, mode: str):
        self.worker = worker
        self.case = case
        self.n = n
        self.mode = mode

    def batch(self, repeat: int) -> int:
        result = self.worker.request({"cmd": "run", "mode": self.mode, "repeat": repeat})
        self.last_digest = result["sha256"]
        return int(result["durationNs"])

    def digest(self) -> str:
        return self.last_digest


def calibrate(item: Any, target_ns: int, max_repeat: int) -> int:
    repeat = 1
    while True:
        elapsed = item.batch(repeat)
        if elapsed >= target_ns or repeat >= max_repeat:
            return repeat
        repeat = min(repeat * 2, max_repeat)


def median_ms(samples_ns: list[int]) -> float:
    return median(samples_ns) / 1_000_000.0


def check_digest(item: Any, expected: str, label: str) -> str:
    actual = item.digest()
    if actual != expected:
        raise RuntimeError(f"output mismatch for {label}: expected {expected}, got {actual}")
    return actual


def benchmark_case(
    case: str,
    native_items: dict[tuple[str, str], NativeCase],
    worker_items: dict[tuple[str, str], WorkerCase],
    expected: str,
    *,
    rounds: int,
    target_ns: int,
    max_repeat: int,
    rng: random.Random,
) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    items: dict[tuple[str, str], Any] = {}
    for language in LANGUAGES:
        for mode in MODES:
            key = (language, mode)
            items[key] = native_items[key] if language in ("ck", "cpp", "rust") else worker_items[key]
    repeats: dict[tuple[str, str], int] = {}
    outputs: dict[tuple[str, str], str] = {}
    for key, item in items.items():
        label = f"{key[0]}-{key[1]}:{case}"
        # Untimed warm-up, then exact output validation before collecting data.
        item.batch(1)
        outputs[key] = check_digest(item, expected, label)
        repeats[key] = calibrate(item, target_ns, max_repeat)
        item.batch(repeats[key])
        check_digest(item, expected, label)

    samples: dict[tuple[str, str], list[int]] = {key: [] for key in items}
    sample_order: list[str] = []
    for round_index in range(rounds):
        order = list(items.keys())
        rng.shuffle(order)
        for key in order:
            item = items[key]
            duration_ns = item.batch(repeats[key])
            check_digest(item, expected, f"{key[0]}-{key[1]}:{case}:round-{round_index + 1}")
            ns_per_call = max(1, round(duration_ns / repeats[key]))
            samples[key].append(ns_per_call)
            sample_order.append(f"{round_index + 1}:{key[0]}-{key[1]}")

    results: list[dict[str, Any]] = []
    for language in LANGUAGES:
        for mode in MODES:
            key = (language, mode)
            values = samples[key]
            median_ns = median(values)
            result = {
                "key": f"{language}-{mode}",
                "language": language,
                "mode": mode,
                "medianMs": median_ns / 1_000_000.0,
                "samplesMs": [value / 1_000_000.0 for value in values],
                "medianNsPerCall": median_ns,
                "samplesNsPerCall": values,
                "batchRepeats": repeats[key],
                "outputHash": outputs[key],
            }
            results.append(result)
    return results, {"roundOrder": sample_order, "calibratedRepeats": {f"{k[0]}-{k[1]}": v for k, v in repeats.items()}}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ckc", help="release ckc executable; also read from CKC")
    parser.add_argument("--cxx", help="C++ compiler (default: clang++, g++, or c++)")
    parser.add_argument("--rustc", help="Rust compiler (default: rustc on PATH)")
    parser.add_argument("--node", help="Node.js executable (default: node on PATH)")
    parser.add_argument("--java", help="OpenJDK 21 runtime (default: java on PATH)")
    parser.add_argument("--javac", help="matching OpenJDK 21 compiler (default: javac on PATH)")
    parser.add_argument("--python", help="Python interpreter with NumPy (default: current interpreter, then python3/python)")
    parser.add_argument("--rounds", type=int, default=7, help="interleaved samples per variant; minimum 7 (default: 7)")
    parser.add_argument("--target-ms", type=float, default=75.0, help="approximate duration of a calibrated batch (default: 75 ms)")
    parser.add_argument("--max-repeat", type=int, default=1_048_576, help="calibration repetition ceiling")
    parser.add_argument("--seed", type=int, default=140926, help="seed used to shuffle each interleaved round")
    parser.add_argument("--output", type=Path, help="result JSON path; default: results/<UTC-date>-<host>.json")
    return parser.parse_args()


def run_benchmarks(args: argparse.Namespace) -> Path:
    set_single_thread_environment()
    if args.rounds < 7:
        raise RuntimeError("--rounds must be at least 7")
    if args.target_ms <= 0 or args.max_repeat < 1:
        raise RuntimeError("--target-ms and --max-repeat must be positive")

    python_path, numpy_version = python_numpy_interpreter(args.python)
    python_version = run_command([python_path, "--version"]).stdout.strip()
    if not python_version:
        python_version = run_command([python_path, "--version"]).stderr.strip()
    node = resolve_program(args.node, ("node",), "Node.js")
    rustc = resolve_program(args.rustc, ("rustc",), "rustc")
    cxx = resolve_program(args.cxx or os.environ.get("CXX"), ("clang++", "g++", "c++"), "C++ compiler")
    ckc = resolve_program(args.ckc or os.environ.get("CKC"), ("ckc",), "ckc compiler")

    host = get_host()
    hostname = platform.node().lower().replace(" ", "-") or "host"
    now = dt.datetime.now(dt.timezone.utc)
    result_path = args.output or (BENCH_ROOT / "results" / f"{now:%Y-%m-%d}-{hostname}.json")
    if not result_path.is_absolute():
        result_path = (ROOT / result_path).resolve()

    env = os.environ.copy()
    for name in THREAD_ENV:
        env[name] = "1"

    commands: list[dict[str, Any]] = []
    with tempfile.TemporaryDirectory(prefix="ck-bench-") as temp_name:
        build_dir = Path(temp_name)
        outputs = build_specs(args, build_dir, commands)
        batcher = args._batcher

        worker_script = outputs[("js", "ordinary")]
        js_worker = JsonLineWorker([node, str(worker_script)], env)
        numpy_worker_command = [python_path, str(Path(__file__).resolve()), "--_numpy-worker", str(outputs[("numpy", "ordinary")])]
        numpy_worker = JsonLineWorker(numpy_worker_command, env)
        java_worker_command = [args._resolved["java"], "-cp", str(build_dir), "BenchmarkWorker"]
        java_worker = JsonLineWorker(java_worker_command, env)
        try:
            cases: list[dict[str, Any]] = []
            sample_metadata: dict[str, Any] = {}
            java_warmup_metadata: dict[str, Any] = {}
            rng = random.Random(args.seed)
            for case, n in SIZE.items():
                js_setup = js_worker.request({"cmd": "init", "case": case, "size": n})
                numpy_setup = numpy_worker.request({"cmd": "init", "case": case, "size": n})
                java_setup = java_worker.request({"cmd": "init", "case": case, "size": n})
                if numpy_setup["numpyVersion"] != numpy_version:
                    raise RuntimeError("NumPy version changed between interpreter probe and worker")
                expected = numpy_setup["referenceHash"]
                if not java_setup.get("ready") or java_setup.get("case") != case or int(java_setup.get("size", -1)) != n:
                    raise RuntimeError(f"Java worker returned an invalid init response for {case}: {java_setup}")
                runtime_version = numeric_java_version(str(java_setup.get("javaVersion", "")))
                if runtime_version != numeric_java_version(args._resolved["versions"]["java"]["version"]):
                    raise RuntimeError("Java worker runtime version changed between probe and worker startup")
                java_warmup_metadata[case] = {
                    "workerReportedWarmupMs": java_setup.get("warmupMs"),
                    "variants": list(MODES),
                }
                native_in, native_second = native_input(case, n)
                native_items: dict[tuple[str, str], NativeCase] = {}
                for language in ("ck", "cpp", "rust"):
                    for mode in MODES:
                        native_items[(language, mode)] = NativeCase(outputs[(language, mode)], batcher, case, n, native_in, native_second)
                worker_items = {
                    ("js", mode): WorkerCase(js_worker, case, n, mode) for mode in MODES
                } | {
                    ("java", mode): WorkerCase(java_worker, case, n, mode) for mode in MODES
                } | {
                    ("numpy", mode): WorkerCase(numpy_worker, case, n, mode) for mode in MODES
                }
                results, meta = benchmark_case(
                    case,
                    native_items,
                    worker_items,
                    expected,
                    rounds=args.rounds,
                    target_ns=round(args.target_ms * 1_000_000),
                    max_repeat=args.max_repeat,
                    rng=rng,
                )
                cases.append({
                    "key": case,
                    "n": n,
                    "description": MATRIX_DESCRIPTION if case == "matmul" else CONVOLVE_DESCRIPTION,
                    "referenceHash": expected,
                    "jsInputContract": js_setup,
                    "javaInputContract": java_setup,
                    "results": results,
                })
                sample_metadata[case] = meta

            ckc_version_result = subprocess.run([ckc, "--version"], text=True, capture_output=True)
            ckc_version = (ckc_version_result.stdout or ckc_version_result.stderr).strip()
            host_tool_versions = {
                "ckc": {"binary": Path(ckc).name, "version": ckc_version},
                "clang": command_version(args._resolved["clang"]),
                "cpp": command_version(cxx),
                "rust": command_version(rustc),
                "node": command_version(node),
                "java": args._resolved["versions"]["java"],
                "javac": args._resolved["versions"]["javac"],
                "python": {"binary": Path(python_path).name, "version": python_version},
                "numpy": {"version": numpy_version, "config": sanitize_numpy_config(numpy_setup.get("numpyConfig", ""))},
            }
            flags, baseline_name = baseline_flags()
            result = {
                "schemaVersion": 1,
                "date": now.isoformat().replace("+00:00", "Z"),
                "host": host,
                "hardware": host,
                "toolchains": host_tool_versions,
                "executionCommands": {
                    "javascriptWorker": [Path(node).name, "benches/cross-language/kernels/javascript/worker.mjs"],
                    "javaWorker": [
                        Path(args._resolved["java"]).name,
                        "-cp",
                        "<build-dir>",
                        "BenchmarkWorker",
                    ],
                    "numpyWorker": [Path(python_path).name, "benches/cross-language/run.py", "--_numpy-worker", "benches/cross-language/kernels/python/numpy_impl.py"],
                },
                "warmup": {
                    "java": {
                        "protocol": "per-case init warms ordinary and tuned before calibration",
                        "reportedByWorker": java_warmup_metadata,
                    }
                },
                "modes": {
                    "ordinary": {
                        "description": "Reference loop order and compiler O3; target uses the named portable baseline where applicable.",
                        "cpuPolicy": "baseline",
                    },
                    "tuned": {
                        "description": "Cache-friendlier/manual API variant and compiler O3 with native CPU tuning where available.",
                        "cpuPolicy": "native",
                    },
                    "nativeBaseline": baseline_name,
                    "cppStrictFloatingPoint": ["-fno-fast-math", "-ffp-contract=off"],
                    "numpyThreadEnvironment": {name: "1" for name in THREAD_ENV},
                    "timer": "batch body only; compilation, process startup, inputs, and SHA-256 excluded",
                    "timedCallOverhead": {
                        "ckCppRust": "one C batch-shim call surrounds repeated native calls; no Python-to-C FFI call per operation",
                        "javascript": "the worker-side JavaScript repeat loop and function dispatch are included",
                        "java": "the persistent JVM worker's repeat loop and function dispatch are included",
                        "numpy": "the worker-side Python repeat loop and function-call dispatch are included",
                    },
                    "targetBatchMs": args.target_ms,
                    "minimumInterleavedRounds": 7,
                    "seed": args.seed,
                },
                "sourceHashes": source_hashes(),
                "buildCommands": commands,
                "sampleProtocol": sample_metadata,
                "cases": cases,
            }
        finally:
            js_worker.close()
            java_worker.close()
            numpy_worker.close()

    result_path.parent.mkdir(parents=True, exist_ok=True)
    result_path.write_text(json.dumps(result, indent=2, sort_keys=False) + "\n")
    return result_path


def numpy_worker_main(module_path: str) -> int:
    # Importing NumPy here happens only after the caller has set every common
    # BLAS/Accelerate thread limit to one.
    scope: dict[str, Any] = {}
    exec(numpy_worker_source(), scope)
    return 0


def main() -> int:
    if len(sys.argv) >= 3 and sys.argv[1] == "--_numpy-worker":
        # Worker mode's protocol is embedded as source so the benchmark remains
        # one checked-in harness with no extra worker file to drift.
        numpy_worker_main(sys.argv[2])
        return 0
    args = parse_args()
    try:
        result_path = run_benchmarks(args)
    except Exception as error:
        print(f"benchmark failed: {error}", file=sys.stderr)
        return 1
    print(result_path)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
