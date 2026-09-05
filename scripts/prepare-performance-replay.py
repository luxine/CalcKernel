#!/usr/bin/env python3
"""Prepare an independently built pinned V0.13 through retained V0.10 replay bundle."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import json
import os
import pathlib
import platform
import shlex
import shutil
import subprocess
import sys
import tarfile
import tomllib

V013_COMMIT = "f82baf42b762e9b19542bcb0af593c1de9252891"
V013_MANIFEST_SHA256 = "1b2f62cdc4a5300c11821f11dfbff264a310352ac685a6be7ef79fafba956b31"
V012_COMMIT = "c70681e578a14ceea0b2bf0d730661140514793e"
V012_MANIFEST_SHA256 = "f1d9668f59e0767a921fc60b6a72b0cec0dafca88f25d8798b5c69848dba8dba"
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
V013_SOURCES = {
    **V012_SOURCES,
    "contract-noalias": "benches/oracles/fixtures/contract_noalias.ck",
    "contract-fixed-length": "benches/oracles/fixtures/contract_fixed_length.ck",
}


def sha256_file(path: pathlib.Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def baseline_identity(version: str) -> dict:
    if version == "0.13":
        return {
            "version": version,
            "commit": V013_COMMIT,
            "manifest": "v0_13_replay.toml",
            "manifestSha256": V013_MANIFEST_SHA256,
            "compiler": "ckc-v013",
            "header": "ckc-v013-performance-replay",
            "runtimeAbi": "2",
            "adapters": (),
        }
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
    if version in {"0.12", "0.13"}:
        sources = V013_SOURCES if version == "0.13" else V012_SOURCES
        for case, relative in sources.items():
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
    if version in {"0.12", "0.13"} and not required.issubset(fields):
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


def copy_replay_for_recipe(
    source: pathlib.Path,
    destination: pathlib.Path,
    current_recipe: str,
    historical_recipe: str,
) -> None:
    if os.path.lexists(destination):
        raise ValueError(f"historical replay target already exists: {destination}")
    if not source.is_dir() or source.is_symlink():
        raise ValueError(f"replay bundle must be a direct directory: {source}")
    if any(
        len(value) != 64 or any(character not in "0123456789abcdef" for character in value)
        for value in (current_recipe, historical_recipe)
    ):
        raise ValueError("replay recipe identity must be a lowercase SHA-256 digest")

    entries = sorted(source.iterdir())
    if not entries or any(not entry.is_file() or entry.is_symlink() for entry in entries):
        raise ValueError(f"replay bundle must contain only direct regular files: {source}")
    manifest = source / "replay.tsv"
    if manifest not in entries:
        raise ValueError("replay bundle has no manifest")
    text = manifest.read_text(encoding="utf-8")
    old_record = f"recipeSha256\t{current_recipe}\n"
    if text.count(old_record) != 1 or text.count("recipeSha256\t") != 1:
        raise ValueError("replay recipe identity does not match the current recipe")

    destination.mkdir(parents=True)
    for entry in entries:
        if entry != manifest:
            shutil.copy2(entry, destination / entry.name)
    (destination / "replay.tsv").write_text(
        text.replace(old_record, f"recipeSha256\t{historical_recipe}\n"),
        encoding="utf-8",
        newline="\n",
    )


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


def prepare(repo: pathlib.Path, out: pathlib.Path, version: str = "0.12",
            with_performance: bool = False) -> None:
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
        def run(command, cwd=repo, environment=None) -> str:
            command = [str(arg) for arg in command]
            description = shlex.join(command)
            print(f"replay preparation: {description}", flush=True)
            log.write(f"$ {description}\n")
            log.flush()
            result = subprocess.run(command, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                    encoding="utf-8", errors="replace", check=False,
                                    env={**(environment or os.environ), "GIT_TERMINAL_PROMPT": "0"})
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
        if version in {"0.12", "0.13"}:
            archive = out / f"ckc-v{version.replace('.', '')}-distribution.tar.gz"
            deterministic_archive(
                archive,
                [
                    (f"ckc-v{version}/ckc", out / identity["compiler"]),
                    (f"ckc-v{version}/LICENSE", source / "LICENSE"),
                    (f"ckc-v{version}/THIRD_PARTY_NOTICES.md", source / "THIRD_PARTY_NOTICES.md"),
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
        historical = []
        if with_performance:
            if version != "0.13":
                raise ValueError("--with-performance is defined only for the v0.13 replay")
            required = [
                "CKC_V012_RUNTIME_BUNDLE", "CKC_V011_RUNTIME_BUNDLE",
                "CKC_V010_RUNTIME_BUNDLE", "CKC_LLVM_PREFIX", "CKC_CLANG_ORACLE",
            ]
            missing = [name for name in required if not os.environ.get(name)]
            if missing:
                raise ValueError(
                    "historical v0.13 performance replay requires " + ", ".join(missing)
                )
            historical_recipe = recipe_digest(source)
            historical_root = source / "target/ckc-perf/historical-replays"
            historical_bundles = {}
            for environment_name, retained_name in [
                ("CKC_V012_RUNTIME_BUNDLE", "replay-v012"),
                ("CKC_V011_RUNTIME_BUNDLE", "replay-v011"),
                ("CKC_V010_RUNTIME_BUNDLE", "replay-v010"),
            ]:
                historical_bundle = historical_root / retained_name
                copy_replay_for_recipe(
                    pathlib.Path(os.environ[environment_name]).absolute(),
                    historical_bundle,
                    recipe,
                    historical_recipe,
                )
                historical_bundles[environment_name] = str(historical_bundle)
            replay_environment = {
                **os.environ,
                **historical_bundles,
                "CKC_CANDIDATE_COMPILER": str(compiler),
                "GITHUB_SHA": identity["commit"],
            }
            run([
                "cargo", "+1.90.0", "bench", "--features", "native-toolchain",
                "--bench", "ckc_perf", "--", "--case", "proof", "--task", "check",
                "--cpu", "baseline",
            ], source, replay_environment)
            run([
                "cargo", "+1.90.0", "bench", "--features", "native-toolchain",
                "--bench", "pgo_perf", "--", "--task", "collect", "--out",
                "target/ckc-perf/v0.13-results.json",
            ], source, replay_environment)
            historical_report = source / "target/ckc-perf/v0.13-results.json"
            decoded = json.loads(historical_report.read_text(encoding="utf-8"))
            evidence_name = decoded.get("evidenceDirectory")
            if not isinstance(evidence_name, str) or "/" in evidence_name or "\\" in evidence_name:
                raise ValueError("historical v0.13 report has an unsafe evidence directory")
            historical_evidence = historical_report.parent / evidence_name
            if not historical_evidence.is_dir() or historical_evidence.is_symlink():
                raise ValueError("historical v0.13 evidence directory is missing or indirect")
            destination = out / "schema8"
            destination.mkdir()
            shutil.copy2(historical_report, destination / "v0.13-results.json")
            shutil.copy2(source / "scripts/check-native-performance.py",
                         out / "check-native-performance-v013.py")
            for entry in historical_evidence.rglob("*"):
                if entry.is_symlink():
                    raise ValueError("historical v0.13 evidence contains a symlink")
                if entry.is_file():
                    relative = entry.relative_to(historical_evidence)
                    target_file = destination / evidence_name / relative
                    target_file.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(entry, target_file)
                elif not entry.is_dir():
                    raise ValueError("historical v0.13 evidence contains a special file")
            for environment_name, retained_name in [
                ("CKC_V012_RUNTIME_BUNDLE", "replay-v012"),
                ("CKC_V011_RUNTIME_BUNDLE", "replay-v011"),
                ("CKC_V010_RUNTIME_BUNDLE", "replay-v010"),
            ]:
                source_bundle = pathlib.Path(replay_environment[environment_name])
                target_bundle = destination / retained_name
                target_bundle.mkdir()
                for entry in sorted(source_bundle.iterdir()):
                    if entry.is_symlink():
                        raise ValueError(f"{environment_name} contains a symlink")
                    if entry.is_file():
                        shutil.copy2(entry, target_bundle / entry.name)
            run([
                "python3", "-B", "scripts/check-native-performance.py",
                historical_report,
            ], source, replay_environment)
            historical = [
                ("historicalReport", "schema8/v0.13-results.json"),
                ("historicalChecker", "check-native-performance-v013.py"),
            ]
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
            "llvmVersion": "22.1.8", "target": target,
            "cpuPolicy": manifest.get("cpu_policy", "baseline"),
            "llvmComponentSha256": component_digest,
            "recipeSha256": recipe,
            "adapterSetSha256": named_digest((f"benches/baselines/{name}", digest) for name, digest in identity["adapters"]),
            "sourceDiffSha256": original_state[1],
            "baselineManifestSha256": identity["manifestSha256"],
        }
        records = [f"{identity['header']}\t{3 if version == '0.13' else 2 if version == '0.12' else 1}"]
        records.extend(f"{name}\t{value}" for name, value in metadata.items())
        records.extend("artifact\t" + "\t".join(artifact) for artifact in artifacts)
        if version in {"0.12", "0.13"}:
            records.append(
                f"distributionArchive\t{archive.name}\t"
                f"{archive.stat().st_size}\t{sha256_file(archive)}"
            )
        for kind, relative in historical:
            retained = out / relative
            records.append(
                f"{kind}\t{relative}\t{retained.stat().st_size}\t{sha256_file(retained)}"
            )
        (out / "replay.tsv").write_text("\n".join(records) + "\n", encoding="utf-8", newline="\n")
    # The detached clone is owned build scratch, not replay evidence. Retaining it
    # would make the supposedly closed bundle include Git internals and target data.
    shutil.rmtree(source)
    print(f"Prepared pinned V{identity['version']} replay bundle: {out}", flush=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True, type=pathlib.Path, help="new owned bundle output directory")
    parser.add_argument("--baseline", choices=("0.13", "0.12", "0.11", "0.10"), default="0.12")
    parser.add_argument("--with-performance", action="store_true")
    args = parser.parse_args()
    try:
        prepare(
            pathlib.Path(__file__).resolve().parents[1], args.out, args.baseline,
            args.with_performance,
        )
    except (OSError, ValueError) as error:
        print(f"replay preparation failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
