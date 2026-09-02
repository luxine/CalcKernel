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
TUNE_MANIFEST = REPO / "benches/oracles/tune/manifest.toml"
TUNE_CASES = REPO / "benches/cases/tune-cases.tsv"


def digest(path: pathlib.Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def run(command: list[object]) -> None:
    command = [str(item) for item in command]
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


def tune_cases() -> list[dict]:
    lines = TUNE_CASES.read_text(encoding="utf-8").splitlines()
    if not lines or lines[0] != "ckc-tune-cases\t1":
        raise ValueError("unsupported tune case manifest")
    result = []
    for line in lines[1:]:
        if not line or line.startswith("#"):
            continue
        fields = line.split("\t")
        if len(fields) != 13:
            raise ValueError("malformed tune case record")
        result.append(dict(zip([
            "name", "source", "manifest", "search_record", "search_seed",
            "search_digest", "validation_record", "validation_seed",
            "validation_digest", "release_record", "release_seed",
            "release_digest", "partition",
        ], fields, strict=True)))
    if len(result) != 7 or len({row["name"] for row in result}) != 7:
        raise ValueError("tune case manifest must contain exactly seven unique cases")
    return result


def tune_split(path: pathlib.Path, expected_header: str) -> list[dict]:
    lines = path.read_text(encoding="utf-8").splitlines()
    if not lines or lines[0] != expected_header:
        raise ValueError(f"unsupported tune input split: {path}")
    result = []
    for line in lines[1:]:
        if not line or line.startswith("#"):
            continue
        fields = line.split("\t")
        if len(fields) != 5:
            raise ValueError(f"malformed tune input row: {line}")
        case, record, length, seed, parameter = fields
        result.append({
            "case": case, "record": record, "length": int(length),
            "seed": int(seed), "parameter": parameter,
        })
    return result


def tune_record(rows: list[dict], case: str, record: str, seed: str) -> dict:
    provenance = {
        "contract-noalias": "memory-bound",
        "contract-fixed-length": "call-constant-length",
    }.get(case, case)
    matches = [row for row in rows if row["case"] == provenance
               and row["record"] == record and row["seed"] == int(seed)]
    if len(matches) != 1:
        raise ValueError(f"missing or ambiguous input {case}/{record}")
    return matches[0]


class TuneKernel:
    def __init__(self, library: pathlib.Path, case: str, record: dict):
        self.library = ctypes.CDLL(str(library))
        self.function = self.library.kernel
        self.case = case
        self.record = record
        self.keepalive = []
        self.output = None
        self.arguments = self._arguments()

    def _u32(self, length: int, salt: int):
        value = (ctypes.c_uint32 * max(1, length))()
        for index in range(length):
            value[index] = ((index + salt) * 2_654_435_761) % 1_000_002 + 1
        self.keepalive.append(value)
        return value

    def _arguments(self):
        length, seed = self.record["length"], self.record["seed"]
        if self.case == "branch-layout":
            items = (ctypes.c_uint64 * max(1, length))(
                *([int(self.record["parameter"])] * length)
            )
            self.keepalive.append(items)
            self.function.argtypes = [ctypes.POINTER(ctypes.c_uint64), ctypes.c_uint32,
                                      ctypes.c_uint32, ctypes.c_uint64]
            self.function.restype = ctypes.c_uint64
            return items, length, length, seed
        if self.case == "call-constant-length":
            actual = 4_000
            source = (ctypes.c_uint32 * actual)(*([int(self.record["parameter"])] * actual))
            self.output = (ctypes.c_uint32 * actual)()
            self.keepalive.extend([source, self.output])
            self.function.argtypes = [ctypes.POINTER(ctypes.c_uint32), ctypes.c_uint32,
                                      ctypes.POINTER(ctypes.c_uint32), ctypes.c_uint32]
            return source, actual, self.output, actual
        if self.case in {"trip-unroll-simd", "contract-noalias", "contract-fixed-length"}:
            actual = 16 if self.case == "contract-fixed-length" else length
            source = self._u32(actual, seed)
            self.output = (ctypes.c_uint32 * max(1, actual))()
            self.keepalive.append(self.output)
            self.function.argtypes = [ctypes.POINTER(ctypes.c_uint32), ctypes.c_uint32,
                                      ctypes.POINTER(ctypes.c_uint32), ctypes.c_uint32,
                                      ctypes.c_uint32]
            return source, actual, self.output, actual, actual
        if self.case == "memory-bound":
            left, right = self._u32(length, seed), self._u32(length, seed + 17)
            self.output = (ctypes.c_uint32 * max(1, length))()
            self.keepalive.append(self.output)
            self.function.argtypes = [ctypes.POINTER(ctypes.c_uint32), ctypes.c_uint32,
                                      ctypes.POINTER(ctypes.c_uint32), ctypes.c_uint32,
                                      ctypes.POINTER(ctypes.c_uint32), ctypes.c_uint32,
                                      ctypes.c_uint32]
            return left, length, right, length, self.output, length, length
        if self.case == "compute-bound":
            source = (ctypes.c_double * max(1, length))()
            for index in range(length):
                source[index] = (index - length / 2 + seed) / 16.0 + 0.25
            self.output = (ctypes.c_double * max(1, length))()
            self.keepalive.extend([source, self.output])
            self.function.argtypes = [ctypes.POINTER(ctypes.c_double), ctypes.c_uint32,
                                      ctypes.POINTER(ctypes.c_double), ctypes.c_uint32,
                                      ctypes.c_uint32, ctypes.c_double]
            return source, length, self.output, length, length, float(self.record["parameter"])
        raise ValueError(f"unknown tune case {self.case}")

    def result(self) -> bytes:
        returned = self.function(*self.arguments)
        if self.case == "branch-layout":
            return int(returned).to_bytes(8, "little")
        return bytes(self.output)


def tune_result_digest(case_id: str, result: bytes) -> str:
    material = bytearray(b"CK-TUNE-RESULT\0")
    material.extend((1).to_bytes(4, "big"))
    encoded = case_id.encode("utf-8")
    material.extend(len(encoded).to_bytes(4, "big"))
    material.extend(encoded)
    material.extend(len(result).to_bytes(8, "big"))
    material.extend(result)
    return hashlib.sha256(material).hexdigest()


def tune_link_flags(output: pathlib.Path) -> list[object]:
    if platform.system() == "Darwin":
        return ["-dynamiclib", "-fPIC", "-Wl,-adhoc_codesign", "-o", output]
    if platform.system() == "Windows":
        return ["-shared", "-o", output]
    return ["-shared", "-fPIC", "-o", output]


def audit_tune(clang: str) -> None:
    clang_version = subprocess.run([clang, "--version"], text=True, capture_output=True,
                                   check=True).stdout
    rust_version = subprocess.run(["rustc", "+1.90.0", "--version"], text=True,
                                  capture_output=True, check=True).stdout
    if "clang version 22.1.8" not in clang_version:
        raise ValueError("CKC_CLANG_ORACLE must be Clang 22.1.8")
    if not rust_version.startswith("rustc 1.90.0 "):
        raise ValueError(f"tune oracle compiler must be rustc 1.90.0, got {rust_version.strip()}")
    with TUNE_MANIFEST.open("rb") as source:
        manifest = tomllib.load(source)
    if manifest.get("schema_version") != 1 or manifest.get("native_abi_schema") != 1:
        raise ValueError("tune oracle manifest schema is not pinned")
    for language in ["c", "rust"]:
        source = REPO / manifest[language]["source"]
        if digest(source) != manifest[language]["sha256"]:
            raise ValueError(f"tune {language} oracle source digest mismatch")
    cases = tune_cases()
    oracle_cases = {row["name"]: row for row in manifest.get("case", [])}
    if set(oracle_cases) != {row["name"] for row in cases}:
        raise ValueError("tune oracle manifest case coverage is incomplete")
    for row in cases:
        source = REPO / row["source"]
        if not source.is_file() or source.is_symlink():
            raise ValueError(f"invalid CK tune source: {row['source']}")
        text = (REPO / "benches/tune/workloads" / row["manifest"]).read_text(encoding="utf-8")
        if row["search_digest"] not in text or row["validation_digest"] not in text \
                or row["release_digest"] in text or "release-held-out" in text:
            raise ValueError(f"tune manifest partition leak or expected digest mismatch: {row['name']}")
    splits = {
        "search": tune_split(REPO / "benches/fixtures/pgo/training.tsv",
                             "ckc-pgo-inputs\t1\ttraining"),
        "validation": tune_split(REPO / "benches/fixtures/pgo/held-out.tsv",
                                 "ckc-pgo-inputs\t1\theld-out"),
        "release": tune_split(REPO / "benches/fixtures/tune/release-held-out.tsv",
                              "ckc-tune-inputs\t1\trelease-held-out"),
    }
    compiler = REPO / "target/release/ckc"
    if not compiler.is_file():
        run(["cargo", "build", "--release", "--features", "native-toolchain", "--locked", "--bin", "ckc"])
    with tempfile.TemporaryDirectory(prefix="ckc-tune-oracle-audit-") as temporary:
        root = pathlib.Path(temporary)
        for row in cases:
            case = row["name"]
            oracle = oracle_cases[case]
            artifacts = []
            for flavor, extra in [
                ("c", []),
                ("c-ubsan", manifest["c"]["ubsan_args"]),
                ("rust", []),
            ]:
                output = root / f"{case}-{flavor}{suffix()}"
                if flavor.startswith("c"):
                    run([
                        clang, "-std=c11", "-O3", "-march=native", "-fno-fast-math",
                        "-ffp-contract=off", "-fno-builtin",
                        f"-DCK_TUNE_ORACLE_CASE={oracle['oracle_case']}",
                        *extra, REPO / manifest["c"]["source"], *tune_link_flags(output),
                    ])
                else:
                    run([
                        "rustc", "+1.90.0", "--edition", "2024", "--crate-type", "cdylib",
                        "-Awarnings", "-C", "opt-level=3", "-C", "target-cpu=native",
                        "-C", "panic=abort", "-C", "llvm-args=-fp-contract=off",
                        "--cfg", f'tune_case="{case}"', REPO / manifest["rust"]["source"],
                        "-o", output,
                    ])
                artifacts.append(output)
            if row["partition"] == "domain":
                generic_c = root / f"{case}-c-generic{suffix()}"
                run([
                    clang, "-std=c11", "-O3", "-march=native", "-fno-fast-math",
                    "-ffp-contract=off", "-fno-builtin", "-DCK_TUNE_GENERIC=1",
                    f"-DCK_TUNE_ORACLE_CASE={oracle['oracle_case']}",
                    REPO / manifest["c"]["source"], *tune_link_flags(generic_c),
                ])
                artifacts.append(generic_c)
            ck_base = root / f"{case}-ck"
            run([
                compiler, "build", REPO / row["source"], "--kind", "dynamic", "--out", ck_base,
                "-O3", "--cpu", "native", "--overflow", "unchecked", "--bounds", "unchecked",
            ])
            ck_library = ck_base.with_suffix(suffix())
            if not ck_library.is_file():
                raise ValueError(f"CK tune oracle build omitted {ck_library.name}")
            artifacts.append(ck_library)
            for split in ["search", "validation", "release"]:
                record = tune_record(splits[split], case, row[f"{split}_record"], row[f"{split}_seed"])
                raw = [TuneKernel(path, case, record).result() for path in artifacts]
                if len(set(raw)) != 1:
                    raise ValueError(f"CK/C/Rust tune oracle mismatch for {case}/{split}")
                case_id = f"{case}.{split}"
                actual = tune_result_digest(case_id, raw[0])
                if actual != row[f"{split}_digest"]:
                    raise ValueError(
                        f"tune expected digest mismatch for {case}/{split}: {actual}"
                    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--clang", default=os.environ.get("CKC_CLANG_ORACLE"))
    parser.add_argument("--pgo", action="store_true", help="audit the schema-8 PGO oracle corpus")
    parser.add_argument("--tune", action="store_true", help="audit the schema-9 tune oracle corpus")
    args = parser.parse_args()
    if not args.clang:
        print("oracle audit failed: CKC_CLANG_ORACLE is required", file=sys.stderr)
        return 1
    try:
        if args.pgo and args.tune:
            raise ValueError("--pgo and --tune are mutually exclusive")
        if args.tune:
            audit_tune(args.clang)
        elif args.pgo:
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
