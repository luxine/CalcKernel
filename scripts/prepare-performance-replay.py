#!/usr/bin/env python3
"""Prepare an independently built pinned V0.12, V0.11, or retained V0.10 replay bundle."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import os
import pathlib
import platform
import shlex
import shutil
import subprocess
import sys
import tarfile
import tomllib

V012_COMMIT = "3bb6d97ced97aa04c22de8e22238c69a6e107eb7"
V012_MANIFEST_SHA256 = "4273f7c2b7f1b91c342874ecdab64bb27e09d1bfe3c3156bcde9d3ccac1324ce"
BASELINE_COMMIT = "80c0acf6bb5d65e4d9d40352b9501ea32b79f43d"
BASELINE_MANIFEST_SHA256 = "495cde2e3a2afb847ddcad9707fec4e6880f26dc6c3085442290af7e2737421e"
V010_COMMIT = "df816502876fba41676f9ebc190e4fadd18cd5a5"
V010_MANIFEST_SHA256 = "27c0b995ba51cd799c2bcb89e1df0a4d40538fbf3200e1197f06ecab2ebad4f3"
RUNTIME_CASES = ("branch_mix", "integer_accumulate", "proof_loop", "remainder_chain")
ADAPTERS = (
    ("v0_10_linux_cpp_runtime_harness.patch", "099305e8a9d5ff8d54e574b0fbd202a511f28a8543508f8c0ea06001704cdaff"),
    ("v0_10_clang_cpu_harness.patch", "f22d58f4e2712e792a5b933376fe3a81fa1bd44a4cdb39b2790359ab5a40c7f1"),
    ("v0_10_mir_optimizer_harness.patch", "828138f376472b177d8bbd1aa4f7888ed323ec03d098e21a74abcfce32a98d0b"),
    ("v0_10_proof_loop_harness.patch", "316b64bf3e24ade271d870444bb66a85018c4dcb66229afce202da2d2b53af6e"),
)
RECIPE_FILES = (
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
)

V012_SOURCES = {
    "branch-layout": "benches/fixtures/pgo/branch_layout.ck",
    "call-constant-length": "benches/fixtures/pgo/call_constant_length.ck",
    "trip-unroll-simd": "benches/oracles/fixtures/map_u32.ck",
    "memory-bound": "benches/oracles/fixtures/zip_u32.ck",
    "compute-bound": "benches/fixtures/pgo/compute_bound.ck",
}


def sha256_file(path: pathlib.Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def baseline_identity(version: str) -> dict:
    if version == "0.12":
        return {
            "version": version,
            "commit": V012_COMMIT,
            "manifest": "v0_12_replay.toml",
            "manifestSha256": V012_MANIFEST_SHA256,
            "compiler": "ckc-v012",
            "header": "ckc-v012-runtime-replay",
            "runtimeAbi": "2",
            "adapters": (),
        }
    if version == "0.11":
        return {
            "version": version,
            "commit": BASELINE_COMMIT,
            "manifest": "v0_11_compiler.toml",
            "manifestSha256": BASELINE_MANIFEST_SHA256,
            "compiler": "ckc-v011",
            "header": "ckc-v011-runtime-replay",
            "runtimeAbi": "2",
            "adapters": (),
        }
    if version == "0.10":
        return {
            "version": version,
            "commit": V010_COMMIT,
            "manifest": "v0_10_compiler.toml",
            "manifestSha256": V010_MANIFEST_SHA256,
            "compiler": "ckc-v010",
            "header": "ckc-v010-runtime-replay",
            "runtimeAbi": "1",
            "adapters": ADAPTERS,
        }
    raise ValueError(f"unsupported replay baseline {version!r}")


def validate_pins(repo: pathlib.Path, version: str = "0.12") -> dict:
    identity = baseline_identity(version)
    baseline = repo / "benches/baselines"
    manifest_path = baseline / identity["manifest"]
    if sha256_file(manifest_path) != identity["manifestSha256"]:
        raise ValueError(f"the frozen V{version} baseline manifest has changed")
    with manifest_path.open("rb") as source:
        manifest = tomllib.load(source)
    if manifest.get("commit") != identity["commit"]:
        raise ValueError(f"the frozen V{version} commit identity has changed")
    for name, digest in identity["adapters"]:
        if sha256_file(baseline / name) != digest:
            raise ValueError(f"pinned replay adapter has changed: {name}")
    source_digests = manifest.get("source_digests", manifest)
    if version == "0.12":
        for case, relative in V012_SOURCES.items():
            if sha256_file(repo / relative) != source_digests.get(case):
                raise ValueError(f"pinned replay CK source has changed: {case}")
        return manifest
    for case in RUNTIME_CASES:
        source = repo / "tests/fixtures/performance/native" / f"{case}.ck"
        expected = source_digests.get(case, manifest.get(f"source_digest_{case}"))
        if sha256_file(source) != expected:
            raise ValueError(f"pinned replay CK source has changed: {case}")
    if version == "0.11":
        for mode in ("unchecked", "checked"):
            for case in RUNTIME_CASES:
                oracle = repo / "benches/baselines/v0_10_c_oracle" / f"{case}-{mode}.c"
                key = f"{case}_{mode}"
                if sha256_file(oracle) != manifest["c_oracle_digests"].get(key):
                    raise ValueError(f"pinned replay C oracle has changed: {case}/{mode}")
    return manifest


def validate_compiler_output(
    text: str, target: str, manifest_sha256: str, version: str = "0.12"
) -> None:
    identity = baseline_identity(version)
    lines = text.splitlines()
    if not lines or lines[0] != f"ckc {version}.0":
        raise ValueError(f"replay must use the actual pinned {version}.0 compiler")
    fields = {}
    for line in lines[1:]:
        name, separator, value = line.partition(": ")
        if not separator or not value or name in fields:
            raise ValueError("malformed or duplicate verbose compiler identity")
        fields[name] = value
    expected = {
        "Native ABI": "1",
        "Runtime ABI": identity["runtimeAbi"],
        "LLVM": "22.1.8",
        "LLVM manifest SHA-256": manifest_sha256,
        "Target": target,
    }
    required = set(expected) | {"Code generator", "ORC object layer"}
    if version in {"0.10", "0.11"} and set(fields) != required:
        raise ValueError("incomplete or unknown verbose compiler identity")
    if version == "0.12" and not required.issubset(fields):
        raise ValueError("incomplete verbose compiler identity")
    for field, value in expected.items():
        if fields[field] != value:
            raise ValueError(f"replay compiler {field} does not match pinned identity")


def named_digest(entries) -> str:
    digest = hashlib.sha256()
    for name, value in sorted(entries):
        digest.update(f"{name}\0{value}\n".encode("utf-8"))
    return digest.hexdigest()


def recipe_digest(repo: pathlib.Path) -> str:
    return named_digest((name, sha256_file(repo / name)) for name in RECIPE_FILES)


def deterministic_archive(output: pathlib.Path, files: list[tuple[str, pathlib.Path]]) -> None:
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w", format=tarfile.PAX_FORMAT) as archive:
        for name, source in sorted(files):
            data = source.read_bytes()
            entry = tarfile.TarInfo(name)
            entry.size = len(data)
            entry.mtime = 0
            entry.uid = entry.gid = 0
            entry.uname = entry.gname = ""
            entry.mode = 0o755 if name.endswith("/ckc") else 0o644
            archive.addfile(entry, io.BytesIO(data))
    with output.open("wb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
            compressed.write(buffer.getvalue())


def host_identity() -> tuple[str, str, str]:
    os_name = {"Linux": "linux", "Darwin": "macos"}.get(platform.system())
    arch = {"x86_64": "x86_64", "amd64": "x86_64", "arm64": "aarch64", "aarch64": "aarch64"}.get(platform.machine().lower())
    if os_name is None or arch is None:
        raise ValueError("runtime replay requires a host with a frozen performance baseline")
    target = f"{os_name}-{arch}"
    triple = f"{arch}-unknown-linux-gnu" if os_name == "linux" else f"{arch}-apple-darwin"
    return target, triple, ".so" if os_name == "linux" else ".dylib"


def prepare(repo: pathlib.Path, out: pathlib.Path, version: str = "0.12") -> None:
    repo = repo.resolve()
    out = out.absolute()
    if os.path.lexists(out):
        raise ValueError(f"replay output already exists; choose a new owned directory: {out}")
    identity = baseline_identity(version)
    manifest = validate_pins(repo, version)
    target, triple, suffix = host_identity()
    if not os.environ.get("CKC_LLVM_PREFIX"):
        raise ValueError("set CKC_LLVM_PREFIX to the pinned release LLVM prefix first")
    component_manifest = pathlib.Path(os.environ["CKC_LLVM_PREFIX"]) / "share/ckc/llvm-build.toml"
    component_digest = sha256_file(component_manifest)
    # V0.11 is measured live and therefore pins source/compiler identity only;
    # retained historical medians exist solely in the V0.10 schema-2 manifest.
    if version == "0.10":
        runtime_keys = {
            (entry["target"], entry["cpu"], entry["mode"], entry["case"])
            for entry in manifest["runtime"]
        }
        if any(
            (target, "baseline", mode, case) not in runtime_keys
            for mode in ("unchecked", "checked")
            for case in RUNTIME_CASES
        ):
            raise ValueError(f"no complete frozen runtime identity for {target}/baseline")
    recipe = recipe_digest(repo)
    out.mkdir(parents=True)
    source = out / ".source"

    with (out / "preparation.log").open("w", encoding="utf-8", newline="\n") as log:
        def run(command, cwd=repo) -> str:
            command = [str(arg) for arg in command]
            description = shlex.join(command)
            print(f"replay preparation: {description}", flush=True)
            log.write(f"$ {description}\n")
            log.flush()
            result = subprocess.run(command, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                    encoding="utf-8", errors="replace", check=False,
                                    env={**os.environ, "GIT_TERMINAL_PROMPT": "0"})
            log.write(result.stdout)
            log.flush()
            if result.returncode:
                raise ValueError(f"command failed ({result.returncode}); retained {out / 'preparation.log'}\n{result.stdout[-3000:]}")
            return result.stdout

        # This is an owned local clone, never an existing user baseline worktree.
        run(["git", "clone", "--no-hardlinks", "--no-checkout", repo, source])
        run(["git", "config", "core.autocrlf", "false"], source)
        run(["git", "checkout", "--detach", identity["commit"]], source)
        if run(["git", "rev-parse", "HEAD"], source).strip() != identity["commit"]:
            raise ValueError("baseline checkout is not the exact pinned commit")
        if run(["git", "status", "--porcelain", "--untracked-files=all"], source).strip():
            raise ValueError("baseline checkout must be clean before approved adapters")
        for name, digest in identity["adapters"]:
            patch = repo / "benches/baselines" / name
            if sha256_file(patch) != digest:
                raise ValueError(f"adapter changed during preparation: {name}")
            run(["git", "apply", "--check", patch], source)
            run(["git", "apply", patch], source)

        def source_state() -> tuple[str, str]:
            if run(["git", "rev-parse", "HEAD"], source).strip() != identity["commit"]:
                raise ValueError("baseline checkout moved during preparation")
            if run(["git", "ls-files", "--others", "--exclude-standard"], source).strip():
                raise ValueError("unexpected untracked baseline source input")
            names = set(run(["git", "diff", "--name-only"], source).splitlines())
            expected_names = {"build.rs", "benches/ckc_perf.rs"} if version == "0.10" else set()
            if names != expected_names:
                raise ValueError("only the fixed version-specific adapters may modify baseline source")
            diff = run(["git", "diff", "--binary", "--full-index", "--no-ext-diff"], source)
            status = run(["git", "status", "--porcelain", "--untracked-files=all"], source)
            return status, hashlib.sha256(diff.encode("utf-8")).hexdigest()

        original_state = source_state()
        run(["rustc", "+1.90.0", "--version", "--verbose"], source)
        run(["cargo", "+1.90.0", "build", "--release", "--locked", "--features", "native-toolchain", "--bin", "ckc"], source)
        compiler = source / "target/release/ckc"
        verbose = run([compiler, "--version", "--verbose"], source)
        # build.rs embeds the installed component manifest, not the source recipe.
        validate_compiler_output(verbose, triple, component_digest, identity["version"])
        compiler_digest = sha256_file(compiler)
        shutil.copy2(compiler, out / identity["compiler"])
        artifacts = []
        if version == "0.12":
            archive = out / "ckc-v012-distribution.tar.gz"
            deterministic_archive(
                archive,
                [
                    ("ckc-v0.12/ckc", out / identity["compiler"]),
                    ("ckc-v0.12/LICENSE", source / "LICENSE"),
                    ("ckc-v0.12/THIRD_PARTY_NOTICES.md", source / "THIRD_PARTY_NOTICES.md"),
                ],
            )
        else:
            for mode in ("unchecked", "checked"):
                for case in RUNTIME_CASES:
                    fixture = repo / "tests/fixtures/performance/native" / f"{case}.ck"
                    source_digests = manifest.get("source_digests", manifest)
                    expected_source = source_digests.get(case, manifest.get(f"source_digest_{case}"))
                    if sha256_file(fixture) != expected_source:
                        raise ValueError(f"runtime source changed during preparation: {case}")
                    filename = f"{case}-{mode}{suffix}"
                    library = out / filename
                    run([compiler, "build", fixture, "--kind", "dynamic", "--out", library,
                         "-O3", "--cpu", "baseline", "--overflow", mode, "--bounds", mode], source)
                    if not library.is_file() or library.is_symlink() or library.stat().st_size == 0:
                        raise ValueError(f"baseline did not emit a nonempty library: {filename}")
                    artifacts.append((mode, case, filename, str(library.stat().st_size), sha256_file(library)))
        if source_state() != original_state:
            raise ValueError("baseline source changed after applying the exact approved adapters")
        validate_pins(repo, identity["version"])
        if recipe_digest(repo) != recipe:
            raise ValueError("preparation/replay implementation changed during preparation")
        if sha256_file(component_manifest) != component_digest:
            raise ValueError("installed LLVM component identity changed during preparation")
        if sha256_file(compiler) != compiler_digest or sha256_file(out / identity["compiler"]) != compiler_digest:
            raise ValueError("baseline compiler changed during library emission")

        metadata = {
            "commit": identity["commit"],
            "compilerIdentity": f"calckernel {identity['version']}.0 ({identity['commit']})",
            "compilerSha256": compiler_digest,
            "compilerBytes": str((out / identity["compiler"]).stat().st_size),
            "llvmVersion": "22.1.8", "target": target, "cpuPolicy": "baseline",
            "llvmComponentSha256": component_digest,
            "recipeSha256": recipe,
            "adapterSetSha256": named_digest((f"benches/baselines/{name}", digest) for name, digest in identity["adapters"]),
            "sourceDiffSha256": original_state[1],
            "baselineManifestSha256": identity["manifestSha256"],
        }
        records = [f"{identity['header']}\t{2 if version == '0.12' else 1}"]
        records.extend(f"{name}\t{value}" for name, value in metadata.items())
        records.extend("artifact\t" + "\t".join(artifact) for artifact in artifacts)
        if version == "0.12":
            records.append(
                "distributionArchive\tckc-v012-distribution.tar.gz\t"
                f"{archive.stat().st_size}\t{sha256_file(archive)}"
            )
        (out / "replay.tsv").write_text("\n".join(records) + "\n", encoding="utf-8", newline="\n")
    print(f"Prepared pinned V{identity['version']} replay bundle: {out}", flush=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True, type=pathlib.Path, help="new owned bundle output directory")
    parser.add_argument("--baseline", choices=("0.12", "0.11", "0.10"), default="0.12")
    args = parser.parse_args()
    try:
        prepare(pathlib.Path(__file__).resolve().parents[1], args.out, args.baseline)
    except (OSError, ValueError) as error:
        print(f"replay preparation failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
