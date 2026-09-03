#!/usr/bin/env python3
"""Collect CK 0.14 predicated-update evidence without judging performance."""

from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import os
import pathlib
import platform
import re
import shutil
import stat
import subprocess
import time

REPO = pathlib.Path(__file__).resolve().parents[1]
RECIPE_FILES = [
    "benches/fixtures/tune/predicated_update.ck",
    "benches/fixtures/tune/predicated-update-training.tsv",
    "benches/fixtures/tune/predicated-update-validation.tsv",
    "benches/fixtures/tune/predicated-update-release.tsv",
    "benches/tune/workloads/predicated-update.cktune.toml",
    "benches/tune/runner.rs",
    "benches/tune_perf.rs",
    "scripts/measure-v014-predicated-update.py",
    "scripts/check-v014-predicated-update.py",
    "specs/0.14/offline-autotuning.md",
    "specs/0.14/predicated-update-performance-1.md",
    "LICENSE",
    "THIRD_PARTY_NOTICES.md",
]
THRESHOLDS = {
    "callsPerRow": 3,
    "measuredRows": 20,
    "releaseMaximumDen": 100,
    "releaseMaximumNum": 95,
    "stabilityLowerDen": 100,
    "stabilityLowerNum": 80,
    "stabilityRequiredRows": 16,
    "stabilityUpperDen": 100,
    "stabilityUpperNum": 120,
    "validationMaximumDen": 100,
    "validationMaximumNum": 102,
    "warmupRows": 3,
}
SPLITS = {
    "training": (128, 113, "d6105453012eedb8a8db812555f116dd69ca6a8e0242faf81f10038947581608"),
    "validation": (256, 127, "e21128a8623d0c072111b02c8a3f1ce3309d12f8bf0b3beddc1e0f6342dfc6c8"),
    "release-held-out": (1024, 131, "4d9a1612967ec78ffb3d0ecc035929bb8217cefc13dfeb3fb2e21989186b055d"),
}
CACHE_COMMANDS = ["profileGeneration", "pgoOnly", "pgoTuned", "replayed"]


def fail(message: str):
    raise ValueError(message)


def text(value: str) -> bytes:
    raw = value.encode("utf-8")
    return len(raw).to_bytes(4, "big") + raw


def list_value(values: list[bytes]) -> bytes:
    return len(values).to_bytes(4, "big") + b"".join(values)


def p(domain: bytes, *values: bytes) -> str:
    digest = hashlib.sha256(domain)
    for value in values:
        digest.update(value)
    return digest.hexdigest()


def sha256(path: pathlib.Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def safe_relative(path: pathlib.Path, root: pathlib.Path) -> str:
    relative = path.resolve().relative_to(root.resolve()).as_posix()
    if not relative or relative.startswith("../"):
        fail("path escapes its identity root")
    return relative


def capture_file_identity(path: pathlib.Path, root: str, base: pathlib.Path) -> dict:
    metadata = path.lstat()
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        fail(f"not a regular no-follow file: {path}")
    return {
        "root": root,
        "path": safe_relative(path, base),
        "bytes": metadata.st_size,
        "sha256": sha256(path),
    }


def file_value(identity: dict) -> bytes:
    return (
        {"repository": 1, "evidence": 2}[identity["root"]].to_bytes(1, "big")
        + text(identity["path"])
        + identity["bytes"].to_bytes(8, "big")
        + bytes.fromhex(identity["sha256"])
    )


def repository_identity(relative: str) -> dict:
    return capture_file_identity(REPO / relative, "repository", REPO)


def evidence_identity(evidence: pathlib.Path, relative: str) -> dict:
    return capture_file_identity(evidence / relative, "evidence", evidence)


def create_evidence_root(output: pathlib.Path) -> pathlib.Path:
    output = output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    output.relative_to(REPO.resolve())
    name = f"v014-predicated-update-{int(time.time())}-{os.getpid()}"
    evidence = output.parent / name
    evidence.mkdir(mode=0o700)
    if evidence.is_symlink():
        fail("evidence root is a symlink")
    return evidence


def copy_executable(source: pathlib.Path, destination: pathlib.Path):
    source_meta = source.lstat()
    if source.is_symlink() or not source.is_file():
        fail(f"tool is not a regular file: {source}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with source.open("rb") as reader, destination.open("xb") as writer:
        shutil.copyfileobj(reader, writer)
    destination.chmod(source_meta.st_mode & 0o777)


def capture_recipe() -> dict:
    files = sorted((repository_identity(path) for path in RECIPE_FILES), key=lambda item: item["path"])
    threshold_values = [text(key) + value.to_bytes(8, "big") for key, value in sorted(THRESHOLDS.items())]
    digest = p(
        b"CK-V014-PRED-RECIPE\0",
        (1).to_bytes(4, "big"),
        list_value([file_value(item) for item in files]),
        list_value(threshold_values),
    )
    return {"schema": 1, "files": files, "thresholds": THRESHOLDS, "digest": digest}


def run_evidence_command(
    name: str,
    argv: list[str],
    executable: dict,
    inputs: list[dict],
    environment: list[dict],
    evidence: pathlib.Path,
) -> dict:
    log_dir = evidence / "commands"
    log_dir.mkdir(exist_ok=True)
    stdout_path = log_dir / f"{name}.stdout"
    stderr_path = log_dir / f"{name}.stderr"
    child_env = {entry["name"]: entry["value"] for entry in environment}
    result = subprocess.run(argv, cwd=REPO, env=child_env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    stdout_path.write_bytes(result.stdout)
    stderr_path.write_bytes(result.stderr)
    if result.returncode != 0:
        fail(f"evidence command {name} exited {result.returncode}: {result.stderr.decode(errors='replace')}")
    return {
        "argv": argv,
        "workingDirectory": "repository",
        "executable": executable,
        "inputs": sorted(inputs, key=lambda item: (item["root"], item["path"])),
        "environment": environment,
        "outputs": [],
        "status": 0,
        "stdout": capture_file_identity(stdout_path, "evidence", evidence),
        "stderr": capture_file_identity(stderr_path, "evidence", evidence),
    }


def cache_snapshot(evidence: pathlib.Path, namespace: pathlib.Path) -> dict:
    files = []
    for path in sorted(namespace.rglob("*")):
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode):
            fail("cache contains a symlink")
        if path.is_dir():
            continue
        if not path.is_file():
            fail("cache contains a foreign entry")
        files.append(capture_file_identity(path, "evidence", evidence))
    relative = safe_relative(namespace, evidence)
    digest = p(
        b"CK-V014-CACHE-SNAPSHOT\0",
        text(relative),
        list_value([file_value(item) for item in files]),
    )
    return {"namespace": relative, "files": files, "digest": digest}


def write_cache_receipt(evidence: pathlib.Path, command: str, phase: str, device: int, inode: int, snapshot: dict) -> dict:
    path = evidence / "cache-receipts" / f"{command}-{phase}.txt"
    path.parent.mkdir(exist_ok=True)
    path.write_text(
        f"CKPREDCACHE/1 command={command} phase={phase} device={device} inode={inode} count={len(snapshot['files'])} digest={snapshot['digest']}\n",
        encoding="utf-8",
    )
    return capture_file_identity(path, "evidence", evidence)


def capture_cache_scratch(evidence: pathlib.Path, command: str) -> dict:
    base = evidence / "cache" / command
    namespace = base / "ckc"
    namespace.mkdir(parents=True, mode=0o700)
    base.chmod(0o700)
    namespace.chmod(0o700)
    descriptor = os.open(namespace, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    details = os.fstat(descriptor)
    lock_path = evidence / "cache-locks" / f"{command}.lock"
    lock_path.parent.mkdir(exist_ok=True)
    lock_descriptor = os.open(lock_path, os.O_CREAT | os.O_EXCL | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    os.write(lock_descriptor, f"CKPREDLOCK/1 {command}\n".encode())
    fcntl.flock(lock_descriptor, fcntl.LOCK_EX)
    before = cache_snapshot(evidence, namespace)
    return {
        "command": command,
        "namespace": safe_relative(namespace, evidence),
        "device": details.st_dev,
        "inode": details.st_ino,
        "lock": capture_file_identity(lock_path, "evidence", evidence),
        "before": before,
        "after": None,
        "beforeReceipt": write_cache_receipt(evidence, command, "before", details.st_dev, details.st_ino, before),
        "afterReceipt": None,
        "_base": base,
        "_namespace": namespace,
        "_descriptor": descriptor,
        "_lockDescriptor": lock_descriptor,
    }


def finish_cache_scratch(evidence: pathlib.Path, item: dict):
    item["after"] = cache_snapshot(evidence, item["_namespace"])
    item["afterReceipt"] = write_cache_receipt(
        evidence, item["command"], "after", item["device"], item["inode"], item["after"]
    )
    os.close(item.pop("_descriptor"))
    lock_descriptor = item.pop("_lockDescriptor")
    fcntl.flock(lock_descriptor, fcntl.LOCK_UN)
    os.close(lock_descriptor)
    item.pop("_base")
    item.pop("_namespace")


def capture_profile_directory(evidence: pathlib.Path, directory: pathlib.Path, phase: str) -> dict:
    entries = []
    for path in sorted(directory.iterdir()):
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
            fail("profile directory contains a non-regular entry")
        entries.append(capture_file_identity(path, "evidence", evidence))
    digest = p(
        b"CK-V014-PRED-DIRECTORY\0",
        text(phase),
        list_value([file_value(item) for item in entries]),
    )
    return {"entries": entries, "digest": digest}


def write_directory_receipt(evidence: pathlib.Path, phase: str, device: int, inode: int, snapshot: dict) -> dict:
    path = evidence / "profile" / f"shards-{phase}.txt"
    path.write_text(
        f"CKPREDDIR/1 phase={phase} device={device} inode={inode} count={len(snapshot['entries'])} digest={snapshot['digest']}\n",
        encoding="utf-8",
    )
    return capture_file_identity(path, "evidence", evidence)


def artifact_paths(base: pathlib.Path) -> list[tuple[str, pathlib.Path]]:
    return [("primary", base.with_suffix(".so")), ("header", base.with_suffix(".h"))]


def artifact_identity(evidence: pathlib.Path, base: pathlib.Path) -> dict:
    outputs = [
        {"role": role, "file": capture_file_identity(path, "evidence", evidence)}
        for role, path in artifact_paths(base)
    ]
    return {"primary": outputs[0]["file"], "outputs": outputs}


def compiler_environment(evidence: pathlib.Path, command: str, cache: dict) -> list[dict]:
    base = evidence / "cache" / command
    return [{"name": "XDG_CACHE_HOME", "value": safe_relative(base, REPO), "references": []}]


def destination_id(parent: pathlib.Path, leaf: str) -> str:
    metadata = parent.stat()
    def field(tag: int, value: bytes) -> bytes:
        return tag.to_bytes(2, "big") + len(value).to_bytes(4, "big") + value
    def record(value: bytes) -> bytes:
        return len(value).to_bytes(4, "big") + value
    parent_fields = (
        field(1, b"\x01")
        + field(2, metadata.st_dev.to_bytes(16, "big"))
        + field(3, metadata.st_ino.to_bytes(16, "big"))
        + field(4, b"\x01")
    )
    key_fields = field(1, record(parent_fields)) + field(2, text(leaf))
    return hashlib.sha256(b"CK-TUNE-DESTINATION\0" + record(key_fields)).hexdigest()


def publication_locks(evidence: pathlib.Path, destinations: list[pathlib.Path]) -> list[dict]:
    rows = []
    for destination in destinations:
        identifier = destination_id(destination.parent, destination.name)
        lock = destination.parent / f".ckc-tune-dest-{identifier}.lock"
        rows.append({
            "destination": capture_file_identity(destination, "evidence", evidence),
            "destinationId": identifier,
            "file": capture_file_identity(lock, "evidence", evidence),
        })
    return sorted(rows, key=lambda item: item["file"]["path"])


def extract_attestation(command: dict, evidence: pathlib.Path, name: str) -> dict:
    stderr = (evidence / command["stderr"]["path"]).read_text(encoding="utf-8")
    lines = [line for line in stderr.splitlines() if line.startswith("CKTUNE-ATTEST/")]
    if len(lines) != 1:
        fail(f"{name} did not emit exactly one predicated attestation")
    path = evidence / "attestation" / f"{name}.txt"
    path.parent.mkdir(exist_ok=True)
    path.write_text(lines[0] + "\n", encoding="utf-8")
    return capture_file_identity(path, "evidence", evidence)


def parse_attestation(identity: dict, evidence: pathlib.Path) -> dict:
    line = (evidence / identity["path"]).read_text(encoding="utf-8").strip("\n")
    fields = dict(item.split("=", 1) for item in line.split()[1:])
    return fields


def parse_selected_plan(decision_path: pathlib.Path) -> str:
    data = decision_path.read_bytes()
    if data[:8] != b"CKTUNE01" or data[8:12] != (1).to_bytes(4, "big"):
        fail("tuning decision header is invalid")
    top = parse_fields(data[12:-32])
    selection = parse_fields(top[7])
    return selection[3].hex()


def parse_fields(data: bytes) -> dict[int, bytes]:
    fields = {}
    offset = 0
    while offset < len(data):
        if offset + 6 > len(data):
            fail("truncated canonical record")
        tag = int.from_bytes(data[offset:offset + 2], "big")
        length = int.from_bytes(data[offset + 2:offset + 6], "big")
        end = offset + 6 + length
        if tag in fields or end > len(data):
            fail("noncanonical record")
        fields[tag] = data[offset + 6:end]
        offset = end
    return fields


def collect_build_graph(evidence: pathlib.Path, compiler: dict, runner: dict, source: dict, manifest: dict) -> dict:
    compiler_argv = safe_relative(evidence / compiler["path"], REPO)
    runner_argv = safe_relative(evidence / runner["path"], REPO)
    source_path = source["path"]
    manifest_path = manifest["path"]
    commands = {}
    artifacts = {}
    caches = {}

    shards = evidence / "profile" / "shards"
    shards.mkdir(parents=True)
    shard_fd = os.open(shards, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    shard_stat = os.fstat(shard_fd)
    before = capture_profile_directory(evidence, shards, "before")
    before["receipt"] = write_directory_receipt(evidence, "before", shard_stat.st_dev, shard_stat.st_ino, before)

    generation_base = evidence / "build" / "generation" / "artifact"
    generation_base.parent.mkdir(parents=True)
    caches["profileGeneration"] = capture_cache_scratch(evidence, "profileGeneration")
    generation_argv = [compiler_argv, "build", source_path, "--out", safe_relative(generation_base, REPO),
                       "--kind", "dynamic", "--cpu", "native", "-O3", "--overflow", "unchecked",
                       "--bounds", "unchecked", "--pgo-generate", safe_relative(shards, REPO)]
    commands["profileGeneration"] = run_evidence_command(
        "profile-generation", generation_argv, compiler, [source],
        compiler_environment(evidence, "profileGeneration", caches["profileGeneration"]), evidence)
    finish_cache_scratch(evidence, caches["profileGeneration"])
    artifacts["generation"] = artifact_identity(evidence, generation_base)
    commands["profileGeneration"]["outputs"] = sorted(
        (item["file"] for item in artifacts["generation"]["outputs"]),
        key=lambda item: (item["root"], item["path"]),
    )

    header_path = generation_base.with_suffix(".h")
    header = header_path.read_text(encoding="utf-8")
    symbols = re.findall(r"ck_profile_flush_[0-9a-f]{64}", header)
    if len(symbols) != 1:
        fail("generation header does not declare one flush symbol")
    training_argv = [runner_argv, "--ck-predicated-profile", safe_relative(generation_base.with_suffix(".so"), REPO), symbols[0], "128", "113"]
    commands["trainingRun"] = run_evidence_command(
        "training-run", training_argv, runner,
        [artifacts["generation"]["outputs"][0]["file"], artifacts["generation"]["outputs"][1]["file"]], [], evidence)
    after = capture_profile_directory(evidence, shards, "after")
    if len(after["entries"]) != 1:
        fail("profile training did not leave exactly one shard")
    after["receipt"] = write_directory_receipt(evidence, "after", shard_stat.st_dev, shard_stat.st_ino, after)
    os.close(shard_fd)
    shard = after["entries"][0]
    commands["trainingRun"]["outputs"] = [shard]

    final_profile = evidence / "profile" / "predicated.ckprof"
    merge_argv = [compiler_argv, "pgo", "merge", safe_relative(evidence / shard["path"], REPO), "--out", safe_relative(final_profile, REPO)]
    commands["profileMerge"] = run_evidence_command("profile-merge", merge_argv, compiler, [shard], [], evidence)
    final_identity = capture_file_identity(final_profile, "evidence", evidence)
    commands["profileMerge"]["outputs"] = [final_identity]
    inspect_argv = [compiler_argv, "pgo", "inspect", safe_relative(final_profile, REPO), "--json"]
    commands["profileInspect"] = run_evidence_command("profile-inspect", inspect_argv, compiler, [final_identity], [], evidence)
    inspection = commands["profileInspect"]["stdout"]
    inspection_json = json.loads((evidence / inspection["path"]).read_text(encoding="utf-8"))

    common = ["--kind", "dynamic", "--cpu", "native", "-O3", "--overflow", "unchecked", "--bounds", "unchecked", "--pgo-use", safe_relative(final_profile, REPO)]
    for command, subdir in [("pgoOnly", "pgo-only"), ("pgoTuned", "pgo-tuned"), ("replayed", "replayed")]:
        base = evidence / "build" / subdir / "artifact"
        base.parent.mkdir(parents=True)
        caches[command] = capture_cache_scratch(evidence, command)
        if command == "pgoTuned":
            decision_path = base.parent / "decision.cktune"
            argv = [compiler_argv, "tune", "build", source_path, "--config", manifest_path, "--out", safe_relative(base, REPO),
                    *common, "--budget", "standard", "--tune-out", safe_relative(decision_path, REPO),
                    "--no-tune-cache", "--explain-optimization"]
            inputs = [source, manifest, final_identity]
        elif command == "replayed":
            decision_path = evidence / "build" / "pgo-tuned" / "decision.cktune"
            argv = [compiler_argv, "build", source_path, "--out", safe_relative(base, REPO), *common,
                    "--tune-use", safe_relative(decision_path, REPO), "--explain-optimization"]
            inputs = [source, final_identity, capture_file_identity(decision_path, "evidence", evidence)]
        else:
            argv = [compiler_argv, "build", source_path, "--out", safe_relative(base, REPO), *common]
            inputs = [source, final_identity]
        commands[command] = run_evidence_command(
            command, argv, compiler, inputs, compiler_environment(evidence, command, caches[command]), evidence)
        finish_cache_scratch(evidence, caches[command])
        artifacts[command] = artifact_identity(evidence, base)
        commands[command]["outputs"] = sorted(
            (item["file"] for item in artifacts[command]["outputs"]),
            key=lambda item: (item["root"], item["path"]),
        )
        if command == "pgoTuned":
            commands[command]["outputs"].append(capture_file_identity(decision_path, "evidence", evidence))

    decision_path = evidence / "build" / "pgo-tuned" / "decision.cktune"
    decision_file = capture_file_identity(decision_path, "evidence", evidence)
    tuned_attestation = extract_attestation(commands["pgoTuned"], evidence, "tuned")
    replay_attestation = extract_attestation(commands["replayed"], evidence, "replayed")
    if (evidence / tuned_attestation["path"]).read_bytes() != (evidence / replay_attestation["path"]).read_bytes():
        fail("tuned and replayed attestations differ")
    line = (evidence / tuned_attestation["path"]).read_bytes()
    attest_digest = hashlib.sha256(b"CK-V014-PRED-ATTEST\0" + len(line).to_bytes(8, "big") + line).hexdigest()
    destinations = [decision_path, *(path for _, path in artifact_paths(evidence / "build" / "pgo-tuned" / "artifact"))]
    locks = publication_locks(evidence, destinations)
    commands["pgoTuned"]["outputs"].extend(item["file"] for item in locks)
    commands["pgoTuned"]["outputs"].sort(key=lambda item: (item["root"], item["path"]))

    for left, right in zip(artifacts["pgoTuned"]["outputs"], artifacts["replayed"]["outputs"], strict=True):
        if left["role"] != right["role"] or left["file"]["sha256"] != right["file"]["sha256"]:
            fail("tuned and replayed artifacts differ")
    return {
        "commands": commands,
        "artifacts": artifacts,
        "cacheScratch": [caches[name] for name in CACHE_COMMANDS],
        "publicationLocks": locks,
        "profile": {
            "directory": {"root": "evidence", "path": safe_relative(shards, evidence),
                          "device": shard_stat.st_dev, "inode": shard_stat.st_ino,
                          "before": before, "after": after},
            "shards": [shard], "final": final_identity,
            "identityDigest": inspection_json["identityDigest"], "inspection": inspection,
        },
        "decision": {"file": decision_file, "decisionDigest": decision_file["sha256"],
                     "planDigest": parse_selected_plan(decision_path), "selected": True},
        "attestation": {"tuned": tuned_attestation, "replayed": replay_attestation, "digest": attest_digest},
    }


def parse_direct_receipt(stdout: bytes, prefix: str, split: str, n: int, seed: int, iterations: int | None, expected: str) -> dict:
    line = stdout.decode("ascii")
    if not line.endswith("\n") or line.count("\n") != 1:
        fail("runner receipt is not one LF-terminated line")
    fields = line.split()
    if prefix == "CKPREDORACLE/1":
        if fields != [prefix, split, str(n), str(seed), expected]:
            fail("oracle receipt mismatch")
        return {}
    if len(fields) != 8 or fields[:4] != [prefix, split, str(n), str(seed)]:
        fail("performance receipt mismatch")
    requested = int(fields[4])
    completed = int(fields[5])
    elapsed = int(fields[6])
    if iterations != requested or completed != requested or elapsed <= 0 or fields[7] != expected:
        fail("performance receipt values mismatch")
    return {"elapsedNs": elapsed, "iterations": requested, "completed": completed, "correctnessDigest": expected}


def collect_correctness(evidence: pathlib.Path, runner: dict, inputs: dict) -> dict:
    runner_argv = safe_relative(evidence / runner["path"], REPO)
    commands = {}
    for split, key in [("training", "training"), ("validation", "validation"), ("release-held-out", "release")]:
        n, seed, expected = SPLITS[split]
        argv = [runner_argv, "--ck-predicated-oracle", split, str(n), str(seed)]
        command = run_evidence_command(f"oracle-{split}", argv, runner, [inputs[key]], [], evidence)
        parse_direct_receipt((evidence / command["stdout"]["path"]).read_bytes(), "CKPREDORACLE/1", split, n, seed, None, expected)
        commands[key] = command
    return {"training": SPLITS["training"][2], "validation": SPLITS["validation"][2],
            "release": SPLITS["release-held-out"][2], "oracleCommands": commands}


def order_for(candidate_sha: str, split: str, phase: str, row: int) -> list[str]:
    material = b"CK-V014-PRED-ORDER\0" + text(candidate_sha) + text(split) + text(phase) + row.to_bytes(4, "big")
    rotate = int.from_bytes(hashlib.sha256(material).digest()[:8], "big") % 2
    channels = ["pgoOnly", "pgoTuned"]
    return channels[rotate:] + channels[:rotate]


def collect_timing_split(evidence: pathlib.Path, runner: dict, artifacts: dict, split: str, candidate_sha: str) -> dict:
    n, seed, expected = SPLITS[split]
    runner_argv = safe_relative(evidence / runner["path"], REPO)
    counters = {"calibration": 0, "confirmation": 0, "warmup": 0, "sample": 0}
    def invoke(channel: str, iterations: int, phase: str) -> tuple[dict, dict]:
        index = counters[phase]
        counters[phase] += 1
        artifact = artifacts[channel]["primary"]
        argv = [runner_argv, "--ck-predicated-perf", safe_relative(evidence / artifact["path"], REPO),
                split, str(n), str(seed), str(iterations)]
        command = run_evidence_command(f"{split}-{phase}-{index:04d}-{channel}", argv, runner, [artifact], [], evidence)
        receipt = parse_direct_receipt((evidence / command["stdout"]["path"]).read_bytes(),
                                       "CKPREDPERF/1", split, n, seed, iterations, expected)
        return command, receipt

    attempts = []
    calibration_commands = []
    iterations = 1
    for _ in range(32):
        command, receipt = invoke("pgoOnly", iterations, "calibration")
        calibration_commands.append(command)
        attempts.append(receipt)
        if receipt["elapsedNs"] >= 50_000_000:
            break
        iterations *= 2
        if iterations > (1 << 64) - 1:
            fail("calibration iteration overflow")
    else:
        fail("calibration did not reach its duration")
    confirmation_command, confirmation = invoke("pgoOnly", iterations, "confirmation")
    if confirmation["elapsedNs"] < 50_000_000:
        fail("calibration confirmation was too short")
    calibration = {"channel": "pgoOnly", "attempts": attempts,
                   "selectedIterationsPerCall": iterations, "confirmation": confirmation}

    def rows(phase: str, count: int):
        orders = []
        commands = {"pgoOnly": [], "pgoTuned": []}
        receipts = {"pgoOnly": [], "pgoTuned": []}
        calls = {"pgoOnly": [], "pgoTuned": []}
        for row in range(count):
            order = order_for(candidate_sha, split, phase, row)
            orders.append(order)
            row_commands = {channel: [] for channel in commands}
            row_receipts = {channel: [] for channel in commands}
            for channel in order:
                for _ in range(3):
                    command, receipt = invoke(channel, iterations, phase)
                    row_commands[channel].append(command)
                    row_receipts[channel].append(receipt)
            for channel in commands:
                commands[channel].append(row_commands[channel])
                receipts[channel].append(row_receipts[channel])
                calls[channel].append([item["elapsedNs"] for item in row_receipts[channel]])
        return orders, commands, receipts, calls

    warmup_order, warmup_commands, warmup_receipts, _ = rows("warmup", 3)
    sample_order, sample_commands, call_receipts, calls_ns = rows("measured", 20)
    samples = {channel: [min(row) for row in calls_ns[channel]] for channel in calls_ns}
    medians = {channel: sorted(values)[10] for channel, values in samples.items()}
    return {
        "split": split, "n": n, "seed": seed, "expectedDigest": expected,
        "calibration": calibration, "calibrationCommands": calibration_commands,
        "confirmationCommand": confirmation_command, "warmupOrder": warmup_order,
        "sampleOrder": sample_order, "warmupCommands": warmup_commands,
        "sampleCommands": sample_commands, "warmupReceipts": warmup_receipts,
        "callReceipts": call_receipts, "callsNs": calls_ns, "samplesNs": samples,
        "mediansNs": medians, "ratioNum": medians["pgoTuned"], "ratioDen": medians["pgoOnly"],
    }


def write_canonical_report(output: pathlib.Path, report: dict):
    with output.open("x", encoding="utf-8") as destination:
        destination.write(json.dumps(report, sort_keys=True, separators=(",", ":")) + "\n")


def copy_schema_nine_evidence(value, source_root: pathlib.Path, evidence: pathlib.Path):
    """Copy the schema-nine FileIdentity closure without changing its JSON identity."""
    if isinstance(value, dict):
        if set(value) == {"root", "path", "bytes", "sha256"} and value.get("root") == "evidence":
            source = source_root / value["path"]
            destination = evidence / value["path"]
            if destination.exists():
                if capture_file_identity(destination, "evidence", evidence) != value:
                    fail("schema-nine evidence path collides with different bytes")
            else:
                copy_executable(source, destination)
            if capture_file_identity(destination, "evidence", evidence) != value:
                fail("copied schema-nine evidence identity mismatch")
            return
        for nested in value.values():
            copy_schema_nine_evidence(nested, source_root, evidence)
    elif isinstance(value, list):
        for nested in value:
            copy_schema_nine_evidence(nested, source_root, evidence)


def collect(output: pathlib.Path):
    if platform.system() != "Linux":
        fail("real predicated-update collection requires a stable Linux performance host")
    status = subprocess.run(["git", "status", "--porcelain"], cwd=REPO, capture_output=True, check=True).stdout
    if status:
        fail("candidate checkout must be clean")
    candidate_sha = subprocess.run(["git", "rev-parse", "HEAD"], cwd=REPO, text=True,
                                   capture_output=True, check=True).stdout.strip()
    if not re.fullmatch(r"[0-9a-f]{40}", candidate_sha):
        fail("candidate SHA is not canonical")
    schema_nine_path = output.resolve().with_name("v0.14-results.json")
    schema_nine = json.loads(schema_nine_path.read_text(encoding="utf-8"))
    evidence = create_evidence_root(output)
    schema_nine_evidence = schema_nine_path.parent / schema_nine["evidenceDirectory"]
    copy_schema_nine_evidence(schema_nine["toolchain"], schema_nine_evidence, evidence)
    compiler_path = evidence / "predicated" / "compiler" / "ckc"
    runner_path = evidence / "predicated" / "runner" / "ckc-tune-runner"
    schema_nine_compiler = schema_nine_evidence / schema_nine["candidateBinary"]["path"]
    copy_executable(schema_nine_compiler, compiler_path)
    copy_executable(REPO / "target/release/ckc-tune-runner", runner_path)
    compiler = capture_file_identity(compiler_path, "evidence", evidence)
    runner = capture_file_identity(runner_path, "evidence", evidence)
    source = repository_identity("benches/fixtures/tune/predicated_update.ck")
    manifest = repository_identity("benches/tune/workloads/predicated-update.cktune.toml")
    inputs = {
        "training": repository_identity("benches/fixtures/tune/predicated-update-training.tsv"),
        "validation": repository_identity("benches/fixtures/tune/predicated-update-validation.tsv"),
        "release": repository_identity("benches/fixtures/tune/predicated-update-release.tsv"),
    }
    graph = collect_build_graph(evidence, compiler, runner, source, manifest)
    correctness = collect_correctness(evidence, runner, inputs)
    validation = collect_timing_split(evidence, runner, graph["artifacts"], "validation", candidate_sha)
    release = collect_timing_split(evidence, runner, graph["artifacts"], "release-held-out", candidate_sha)
    report = {
        "schemaVersion": 1, "candidateVersion": "0.14.0", "candidateSha": candidate_sha,
        "evidenceDirectory": evidence.name, "toolchain": schema_nine["toolchain"],
        "hardware": schema_nine["hardware"], "recipe": capture_recipe(), "compiler": compiler,
        "runner": runner, "source": source, "inputs": inputs, "profile": graph["profile"],
        "manifest": manifest, "decision": graph["decision"], "attestation": graph["attestation"],
        "artifacts": graph["artifacts"], "publicationLocks": graph["publicationLocks"],
        "cacheScratch": graph["cacheScratch"], "commands": graph["commands"],
        "correctness": correctness, "validation": validation, "release": release,
    }
    write_canonical_report(output.resolve(), report)


def main():
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument("--task", required=True, choices=["collect-predicated-update"])
    parser.add_argument("--out", required=True)
    arguments = parser.parse_args()
    collect(pathlib.Path(arguments.out))


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"measure-v014-predicated-update: {error}", file=os.sys.stderr)
        raise SystemExit(1)
