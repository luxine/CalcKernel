#!/usr/bin/env python3
"""Compile and differentially/UB-audit the pinned CK 0.12 C/Rust oracles."""

from __future__ import annotations

import argparse
import ctypes
import hashlib
import importlib.util
import os
import pathlib
import platform
import subprocess
import sys
import tempfile
import tomllib

REPO = pathlib.Path(__file__).resolve().parents[1]
MANIFEST = REPO / "benches/oracles/manifest.toml"
PGO_MANIFEST = REPO / "benches/oracles/pgo/manifest.toml"


def digest(path: pathlib.Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def run(command: list[str]) -> None:
    completed = subprocess.run(command, cwd=REPO, text=True, capture_output=True, check=False)
    if completed.returncode:
        raise ValueError(
            f"oracle command failed ({completed.returncode}): {' '.join(command)}\n{completed.stdout}{completed.stderr}"
        )


def suffix() -> str:
    return {"Darwin": ".dylib", "Windows": ".dll"}.get(platform.system(), ".so")


def compile_c(clang: str, case: int, checked: bool, output: pathlib.Path, ubsan: bool) -> None:
    command = [
        clang, "-std=c11", "-O3", "-fno-fast-math", "-ffp-contract=off", "-fno-builtin", "-fuse-ld=lld",
        f"-DORACLE_CASE={case}", f"-DORACLE_CHECKED={int(checked)}",
    ]
    if ubsan:
        command += [
            "-fsanitize=undefined", "-fno-sanitize-recover=all",
            "-fsanitize-trap=undefined",
        ]
    if platform.system() == "Darwin":
        command += [
            "-dynamiclib", "-nostdlib", "-Wl,-platform_version,macos,11.0,11.0",
            "-Wl,-adhoc_codesign",
        ]
    elif platform.system() == "Windows":
        command += ["-shared", "-Wl,/noentry", "-nostdlib"]
    else:
        command += ["-shared", "-fPIC", "-nostdlib", "-Wl,--no-undefined"]
    command += [str(REPO / "benches/oracles/c/vector_oracle.c"), "-o", str(output)]
    run(command)


def compile_rust(case: str, checked: bool, output: pathlib.Path) -> None:
    command = [
        "rustc", "--edition", "2021", "--crate-type", "cdylib", "-Awarnings",
        "-C", "opt-level=3", "-C", "target-cpu=generic", "-C", "panic=abort",
        "--cfg", f'oracle_case="{case}"',
    ]
    if checked:
        command += ["--cfg", "oracle_checked"]
    command += [str(REPO / "benches/oracles/rust/vector_oracle.rs"), "-o", str(output)]
    run(command)


def u32_values(n: int, salt: int) -> ctypes.Array:
    array = (ctypes.c_uint32 * n)()
    for index in range(n):
        array[index] = ((index + salt) * 2_654_435_761) % 1_000_002 + 1
    return array


def f64_values(n: int) -> ctypes.Array:
    array = (ctypes.c_double * n)()
    for index in range(n):
        array[index] = (index - 2048) / 16.0 + 0.25
    return array


def invoke(library: pathlib.Path, name: str, checked: bool) -> bytes:
    n = 4 if name == "slp_quad" else 16 if name in {"contract_noalias", "contract_fixed_length"} else 4000
    dylib = ctypes.CDLL(str(library))
    kernel = dylib.ck_oracle_kernel
    status_type = ctypes.c_int32 if checked else None
    kernel.restype = status_type
    a = u32_values(n, 7)
    b = u32_values(n, 19)
    if name == "integer_cast":
        for index, value in enumerate((0, 2_147_483_647, 2_147_483_648, 4_294_967_295)):
            a[index] = value
    out = (ctypes.c_uint32 * n)()
    if name == "zip_u32":
        args = (a, n, b, n, out, n, n)
    elif name == "strict_f64":
        f64 = f64_values(n)
        f64_out = (ctypes.c_double * n)()
        args = (f64, n, f64_out, n, n, ctypes.c_double(1.0009765625))
        result_buffer = f64_out
    elif name == "integer_cast":
        f64_out = (ctypes.c_double * n)()
        args = (a, n, f64_out, n, n)
        result_buffer = f64_out
    elif name == "modular_reduction":
        if checked:
            result = ctypes.c_uint32()
            args = (a, n, n, ctypes.byref(result))
            result_buffer = result
        else:
            kernel.restype = ctypes.c_uint32
            args = (a, n, n)
            result_buffer = None
    elif name == "slp_quad":
        args = (a, n, b, n, out, n)
        result_buffer = out
    elif name == "specialized_length":
        args = (a, n, out, n)
    else:
        args = (a, n, out, n, n)

    returned = kernel(*args)
    if checked and returned != 0:
        raise ValueError(f"{name} checked oracle returned status {returned}")
    if name == "modular_reduction" and not checked:
        return int(returned).to_bytes(4, "little")
    if name == "modular_reduction":
        return int(result_buffer.value).to_bytes(4, "little")
    return bytes(result_buffer if name in {"strict_f64", "integer_cast"} else out)


def compare_kernel(c: pathlib.Path, rust: pathlib.Path, ubsan: pathlib.Path,
                   name: str, checked: bool) -> None:
    c_result = invoke(c, name, checked)
    rust_result = invoke(rust, name, checked)
    ubsan_result = invoke(ubsan, name, checked)
    if c_result != rust_result or c_result != ubsan_result:
        raise ValueError(f"oracle differential mismatch for {name}/{'checked' if checked else 'unchecked'}")


def audit(clang: str) -> None:
    clang_version = subprocess.run([clang, "--version"], text=True, capture_output=True, check=True).stdout
    rust_version = subprocess.run(["rustc", "--version"], text=True, capture_output=True, check=True).stdout
    if "clang version 22.1.8" not in clang_version:
        raise ValueError("CKC_CLANG_ORACLE must be Clang 22.1.8")
    if not rust_version.startswith("rustc 1.90.0 "):
        raise ValueError(f"oracle compiler must be rustc 1.90.0, got {rust_version.strip()}")
    with MANIFEST.open("rb") as source:
        manifest = tomllib.load(source)
    for entry in manifest["source"]:
        if digest(REPO / entry["source"]) != entry["sha256"]:
            raise ValueError(f"oracle source digest mismatch: {entry['source']}")
    for kernel in manifest["kernel"]:
        if digest(REPO / kernel["ck_source"]) != kernel["ck_sha256"]:
            raise ValueError(f"CK fixture digest mismatch: {kernel['name']}")

    with tempfile.TemporaryDirectory(prefix="ckc-oracle-audit-") as temporary:
        root = pathlib.Path(temporary)
        for kernel in manifest["kernel"]:
            for checked in (False, True):
                mode = "checked" if checked else "unchecked"
                c = root / f"{kernel['name']}-{mode}-c{suffix()}"
                rust = root / f"{kernel['name']}-{mode}-rust{suffix()}"
                ubsan = root / f"{kernel['name']}-{mode}-ubsan{suffix()}"
                compile_c(clang, kernel["oracle_case"], checked, c, False)
                compile_rust(kernel["name"], checked, rust)
                compile_c(clang, kernel["oracle_case"], checked, ubsan, True)
                compare_kernel(c, rust, ubsan, kernel["name"], checked)


def load_pgo_measurement_module():
    specification = importlib.util.spec_from_file_location(
        "ckc_pgo_measurement", REPO / "scripts/measure-v013-performance.py"
    )
    module = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(module)
    return module


def audit_pgo(clang: str) -> None:
    measurement = load_pgo_measurement_module()
    clang_version = subprocess.run([clang, "--version"], text=True, capture_output=True, check=True).stdout
    rust_version = subprocess.run(["rustc", "+1.90.0", "--version"], text=True, capture_output=True, check=True).stdout
    if "clang version 22.1.8" not in clang_version:
        raise ValueError("CKC_CLANG_ORACLE must be Clang 22.1.8")
    if not rust_version.startswith("rustc 1.90.0 "):
        raise ValueError(f"PGO oracle compiler must be rustc 1.90.0, got {rust_version.strip()}")
    with PGO_MANIFEST.open("rb") as source:
        manifest = tomllib.load(source)
    if (
        manifest.get("schema_version") != 1
        or manifest.get("clang_version") != "22.1.8"
        or manifest.get("rust_version") != "1.90.0"
        or manifest.get("fast_math") is not False
        or manifest.get("contraction") is not False
    ):
        raise ValueError("PGO oracle manifest identity is not closed")
    for entry in manifest.get("source", []):
        if digest(REPO / entry["path"]) != entry["sha256"]:
            raise ValueError(f"PGO oracle source digest mismatch: {entry['path']}")
    cases = {case["name"]: case for case in measurement.parse_cases()}
    if set(cases) != {entry["name"] for entry in manifest.get("case", [])}:
        raise ValueError("PGO oracle case manifest is incomplete")
    splits = {
        name: measurement.parse_split(name)
        for name in ["training", "held-out", "adversarial"]
    }
    with tempfile.TemporaryDirectory(prefix="ckc-pgo-oracle-audit-") as temporary:
        root = pathlib.Path(temporary)
        for entry in manifest["case"]:
            case = cases[entry["name"]]
            c = root / f"{case['name']}-c{measurement.dynamic_suffix()}"
            ubsan = root / f"{case['name']}-ubsan{measurement.dynamic_suffix()}"
            rust = root / f"{case['name']}-rust{measurement.dynamic_suffix()}"
            c_common = [
                clang, "-std=c11", "-O3", "-march=native", "-fno-fast-math",
                "-ffp-contract=off", "-fno-builtin",
                f"-DCK_PGO_ORACLE_CASE={entry['oracle_case']}",
                REPO / "benches/oracles/pgo/c/pgo_oracle.c",
            ]
            measurement.command_output(c_common + measurement.oracle_link_flags(c))
            measurement.command_output(
                c_common
                + ["-fsanitize=undefined", "-fno-sanitize-recover=all",
                   "-fsanitize-trap=undefined"]
                + measurement.oracle_link_flags(ubsan)
            )
            rust_common = [
                "rustc", "+1.90.0", "--edition", "2024", "--crate-type", "cdylib",
                "-Awarnings", "-C", "opt-level=3", "-C", "target-cpu=native",
                "-C", "panic=abort", "-C", "llvm-args=-fp-contract=off",
                "--cfg", f'oracle_case="{case["name"]}"',
                REPO / "benches/oracles/pgo/rust/pgo_oracle.rs", "-o", rust,
            ]
            measurement.command_output(rust_common)
            for split in splits.values():
                for record in split.get(case["name"], []):
                    results = {
                        measurement.Kernel(library, case, record).result_digest()
                        for library in [c, ubsan, rust]
                    }
                    if len(results) != 1:
                        raise ValueError(
                            f"PGO oracle differential mismatch for {case['name']}/{record['record']}"
                        )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--clang", default=os.environ.get("CKC_CLANG_ORACLE"))
    parser.add_argument("--pgo", action="store_true", help="audit the schema-8 PGO oracle corpus")
    args = parser.parse_args()
    if not args.clang:
        print("oracle audit failed: CKC_CLANG_ORACLE is required", file=sys.stderr)
        return 1
    try:
        if args.pgo:
            audit_pgo(args.clang)
        else:
            audit(args.clang)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"oracle audit failed: {error}", file=sys.stderr)
        return 1
    print("oracle audit passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
