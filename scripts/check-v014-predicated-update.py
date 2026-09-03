#!/usr/bin/env python3
"""Verify CK 0.14 Predicated-Update Performance Contract 1 evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import re
import stat
import subprocess

REPO = pathlib.Path(__file__).resolve().parents[1]
TOP_KEYS = {
    "schemaVersion", "candidateVersion", "candidateSha", "evidenceDirectory",
    "toolchain", "hardware", "recipe", "compiler", "runner", "source", "inputs",
    "profile", "manifest", "decision", "attestation", "artifacts", "publicationLocks",
    "cacheScratch", "commands", "correctness", "validation", "release",
}
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
    "callsPerRow": 3, "measuredRows": 20,
    "releaseMaximumDen": 100, "releaseMaximumNum": 95,
    "stabilityLowerDen": 100, "stabilityLowerNum": 80,
    "stabilityRequiredRows": 16, "stabilityUpperDen": 100,
    "stabilityUpperNum": 120, "validationMaximumDen": 100,
    "validationMaximumNum": 102, "warmupRows": 3,
}
SPLITS = {
    "training": (128, 113, "d6105453012eedb8a8db812555f116dd69ca6a8e0242faf81f10038947581608"),
    "validation": (256, 127, "e21128a8623d0c072111b02c8a3f1ce3309d12f8bf0b3beddc1e0f6342dfc6c8"),
    "release-held-out": (1024, 131, "4d9a1612967ec78ffb3d0ecc035929bb8217cefc13dfeb3fb2e21989186b055d"),
}
CACHE_COMMANDS = ["profileGeneration", "pgoOnly", "pgoTuned", "replayed"]
DIGEST = re.compile(r"[0-9a-f]{64}\Z")
ATTESTATION = re.compile(
    r"CKTUNE-ATTEST/1 shape=predicated-same-place-update function=floyd "
    r"header=(0|[1-9][0-9]*) compare=(0|[1-9][0-9]*) load=(0|[1-9][0-9]*) "
    r"store=(0|[1-9][0-9]*) unit=([0-9a-f]{64}) variant=([0-9a-f]{64}) "
    r"alternative=([0-9a-f]{64}) vectorBits=([1-9][0-9]*) uf=([1-9][0-9]*) "
    r"minimum=([1-9][0-9]*) pre=([0-9a-f]{64}) post=([0-9a-f]{64})\n\Z"
)


def fail(message: str):
    raise ValueError(message)


def exact_keys(value: dict, expected: set[str], name: str):
    if not isinstance(value, dict) or set(value) != expected:
        fail(f"{name} has missing or unknown keys")


def u64(value, name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or not 0 <= value <= (1 << 64) - 1:
        fail(f"{name} is not a U64 integer")
    return value


def digest(value, name: str) -> str:
    if not isinstance(value, str) or DIGEST.fullmatch(value) is None:
        fail(f"{name} is not a lowercase digest")
    return value


def text(value: str) -> bytes:
    raw = value.encode("utf-8")
    return len(raw).to_bytes(4, "big") + raw


def list_value(values: list[bytes]) -> bytes:
    return len(values).to_bytes(4, "big") + b"".join(values)


def p(domain: bytes, *values: bytes) -> str:
    result = hashlib.sha256(domain)
    for value in values:
        result.update(value)
    return result.hexdigest()


def safe_path(root: pathlib.Path, relative: str) -> pathlib.Path:
    if not isinstance(relative, str) or not relative or "\\" in relative:
        fail("noncanonical relative path")
    candidate = pathlib.PurePosixPath(relative)
    if candidate.is_absolute() or any(part in ("", ".", "..") for part in candidate.parts):
        fail("traversing relative path")
    current = root
    for part in candidate.parts:
        current = current / part
        metadata = current.lstat()
        if stat.S_ISLNK(metadata.st_mode):
            fail("identity path contains a symlink")
    return current


def check_file(identity: dict, evidence: pathlib.Path, name: str) -> pathlib.Path:
    exact_keys(identity, {"root", "path", "bytes", "sha256"}, name)
    base = {"repository": REPO, "evidence": evidence}.get(identity["root"])
    if base is None:
        fail(f"{name} has an unknown root")
    path = safe_path(base, identity["path"])
    metadata = path.lstat()
    if not stat.S_ISREG(metadata.st_mode):
        fail(f"{name} is not a regular file")
    if metadata.st_size != u64(identity["bytes"], f"{name}.bytes"):
        fail(f"{name} byte count mismatch")
    actual = hashlib.sha256(path.read_bytes()).hexdigest()
    if actual != digest(identity["sha256"], f"{name}.sha256"):
        fail(f"{name} digest mismatch")
    return path


def file_value(value: dict) -> bytes:
    return (
        {"repository": 1, "evidence": 2}[value["root"]].to_bytes(1, "big")
        + text(value["path"])
        + value["bytes"].to_bytes(8, "big")
        + bytes.fromhex(value["sha256"])
    )


def load_canonical(path: pathlib.Path) -> dict:
    raw = path.read_bytes()
    if not raw.endswith(b"\n"):
        fail("report lacks one final LF")
    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                fail("duplicate JSON key")
            result[key] = value
        return result
    report = json.loads(raw, object_pairs_hook=pairs, parse_float=lambda _: fail("JSON floats are forbidden"))
    canonical = (json.dumps(report, sort_keys=True, separators=(",", ":")) + "\n").encode()
    if raw != canonical:
        fail("report JSON is not canonical")
    return report


def check_recipe(value: dict, evidence: pathlib.Path):
    exact_keys(value, {"schema", "files", "thresholds", "digest"}, "recipe")
    if u64(value["schema"], "recipe.schema") != 1 or value["thresholds"] != THRESHOLDS:
        fail("recipe schema or thresholds mismatch")
    exact_keys(value["thresholds"], set(THRESHOLDS), "recipe.thresholds")
    for key, expected_number in THRESHOLDS.items():
        if u64(value["thresholds"][key], f"recipe.thresholds.{key}") != expected_number:
            fail("recipe threshold type mismatch")
    if not isinstance(value["files"], list) or [item.get("path") for item in value["files"]] != sorted(RECIPE_FILES):
        fail("recipe file set/order mismatch")
    for item in value["files"]:
        if item.get("root") != "repository":
            fail("recipe file has the wrong root")
        check_file(item, evidence, "recipe file")
    threshold_values = [text(key) + number.to_bytes(8, "big") for key, number in sorted(THRESHOLDS.items())]
    expected = p(b"CK-V014-PRED-RECIPE\0", (1).to_bytes(4, "big"),
                 list_value([file_value(item) for item in value["files"]]), list_value(threshold_values))
    if value["digest"] != expected:
        fail("recipe digest mismatch")


def check_command(value: dict, evidence: pathlib.Path, name: str, executable: dict | None = None):
    exact_keys(value, {"argv", "workingDirectory", "executable", "inputs", "environment",
                       "outputs", "status", "stdout", "stderr"}, name)
    if value["workingDirectory"] != "repository" or u64(value["status"], f"{name}.status") != 0:
        fail(f"{name} working directory or status mismatch")
    if not isinstance(value["argv"], list) or not value["argv"] or not all(isinstance(item, str) for item in value["argv"]):
        fail(f"{name} argv is invalid")
    actual_executable = check_file(value["executable"], evidence, f"{name}.executable")
    argv_executable = safe_path(REPO, value["argv"][0])
    if argv_executable.read_bytes() != actual_executable.read_bytes():
        fail(f"{name} argv executable mismatch")
    if executable is not None and value["executable"] != executable:
        fail(f"{name} top-level executable mismatch")
    for field in ["inputs", "outputs"]:
        items = value[field]
        if not isinstance(items, list) or items != sorted(items, key=lambda item: (item["root"], item["path"])):
            fail(f"{name}.{field} is not path-sorted")
        for item in items:
            check_file(item, evidence, f"{name}.{field}")
    if not isinstance(value["environment"], list):
        fail(f"{name}.environment is invalid")
    environment_names = []
    for item in value["environment"]:
        exact_keys(item, {"name", "value", "references"}, f"{name}.environment")
        if not isinstance(item["name"], str) or not isinstance(item["value"], str) \
                or not isinstance(item["references"], list):
            fail(f"{name}.environment references are invalid")
        environment_names.append(item["name"])
    if environment_names != sorted(set(environment_names)):
        fail(f"{name}.environment is not unique and sorted")
    check_file(value["stdout"], evidence, f"{name}.stdout")
    check_file(value["stderr"], evidence, f"{name}.stderr")


def check_directory(value: dict, evidence: pathlib.Path):
    exact_keys(value, {"root", "path", "device", "inode", "before", "after"}, "profile.directory")
    if value["root"] != "evidence" or value["path"] != "profile/shards":
        fail("profile directory identity mismatch")
    directory = safe_path(evidence, value["path"])
    metadata = directory.stat()
    if (metadata.st_dev, metadata.st_ino) != (u64(value["device"], "directory.device"), u64(value["inode"], "directory.inode")):
        fail("profile directory device/inode mismatch")
    for phase in ["before", "after"]:
        snapshot = value[phase]
        exact_keys(snapshot, {"entries", "digest", "receipt"}, f"directory.{phase}")
        if snapshot["entries"] != sorted(snapshot["entries"], key=lambda item: (item["root"], item["path"])):
            fail("directory entries are not sorted")
        for item in snapshot["entries"]:
            check_file(item, evidence, f"directory.{phase}.entry")
        expected = p(b"CK-V014-PRED-DIRECTORY\0", text(phase),
                     list_value([file_value(item) for item in snapshot["entries"]]))
        if snapshot["digest"] != expected:
            fail("directory snapshot digest mismatch")
        receipt = check_file(snapshot["receipt"], evidence, f"directory.{phase}.receipt")
        exact = (f"CKPREDDIR/1 phase={phase} device={metadata.st_dev} inode={metadata.st_ino} "
                 f"count={len(snapshot['entries'])} digest={expected}\n").encode()
        if receipt.read_bytes() != exact:
            fail("directory snapshot receipt mismatch")
    if value["before"]["entries"] or len(value["after"]["entries"]) != 1:
        fail("profile directory cardinality mismatch")
    live = [path for path in directory.iterdir() if path.is_file() and not path.is_symlink()]
    if [path.name for path in live] != [pathlib.Path(value["after"]["entries"][0]["path"]).name]:
        fail("profile live shard mismatch")


def cache_snapshot_digest(snapshot: dict) -> str:
    return p(b"CK-V014-CACHE-SNAPSHOT\0", text(snapshot["namespace"]),
             list_value([file_value(item) for item in snapshot["files"]]))


def check_cache_scratch(rows: list, evidence: pathlib.Path, commands: dict):
    if not isinstance(rows, list) or [item.get("command") for item in rows] != CACHE_COMMANDS:
        fail("cache scratch order/cardinality mismatch")
    seen = set()
    for row in rows:
        exact_keys(row, {"command", "namespace", "device", "inode", "lock", "before", "after",
                         "beforeReceipt", "afterReceipt"}, "cache scratch")
        command = row["command"]
        if row["namespace"] != f"cache/{command}/ckc" or row["namespace"] in seen:
            fail("cache namespace mismatch")
        seen.add(row["namespace"])
        namespace = safe_path(evidence, row["namespace"])
        metadata = namespace.stat()
        if (metadata.st_dev, metadata.st_ino) != (row["device"], row["inode"]):
            fail("cache namespace device/inode mismatch")
        expected_env = [{"name": "XDG_CACHE_HOME", "value": safe_path(evidence, f"cache/{command}").relative_to(REPO).as_posix(), "references": []}]
        if commands[command]["environment"] != expected_env:
            fail("cache environment mapping mismatch")
        lock = check_file(row["lock"], evidence, "cache lock")
        if lock.read_bytes() != f"CKPREDLOCK/1 {command}\n".encode() or stat.S_IMODE(lock.stat().st_mode) & 0o077:
            fail("cache lock content or mode mismatch")
        for phase in ["before", "after"]:
            snapshot = row[phase]
            exact_keys(snapshot, {"namespace", "files", "digest"}, f"cache.{phase}")
            if snapshot["namespace"] != row["namespace"] or snapshot["files"] != sorted(snapshot["files"], key=lambda item: (item["root"], item["path"])):
                fail("cache snapshot namespace or order mismatch")
            for item in snapshot["files"]:
                check_file(item, evidence, "cache snapshot file")
            expected_digest = cache_snapshot_digest(snapshot)
            if snapshot["digest"] != expected_digest:
                fail("cache snapshot digest mismatch")
            receipt = check_file(row[f"{phase}Receipt"], evidence, "cache receipt")
            exact = (f"CKPREDCACHE/1 command={command} phase={phase} device={row['device']} inode={row['inode']} "
                     f"count={len(snapshot['files'])} digest={expected_digest}\n").encode()
            if receipt.read_bytes() != exact:
                fail("cache receipt mismatch")
        if row["before"]["files"]:
            fail("cache before snapshot is nonempty")
        live = sorted((check_file(item, evidence, "cache live file") for item in row["after"]["files"]))
        actual = sorted(path for path in namespace.rglob("*") if path.is_file() and not path.is_symlink())
        if live != actual:
            fail("cache after snapshot is incomplete")


def field(tag: int, value: bytes) -> bytes:
    return tag.to_bytes(2, "big") + len(value).to_bytes(4, "big") + value


def record(value: bytes) -> bytes:
    return len(value).to_bytes(4, "big") + value


def destination_id(parent: pathlib.Path, leaf: str) -> str:
    metadata = parent.stat()
    parent_value = field(1, b"\x01") + field(2, metadata.st_dev.to_bytes(16, "big"))
    parent_value += field(3, metadata.st_ino.to_bytes(16, "big")) + field(4, b"\x01")
    material = field(1, record(parent_value)) + field(2, text(leaf))
    return hashlib.sha256(b"CK-TUNE-DESTINATION\0" + record(material)).hexdigest()


def check_publication_locks(rows: list, evidence: pathlib.Path, artifacts: dict, decision: dict):
    destinations = [decision["file"], *(item["file"] for item in artifacts["pgoTuned"]["outputs"])]
    if not isinstance(rows, list) or len(rows) != len(destinations) or rows != sorted(rows, key=lambda item: item["file"]["path"]):
        fail("publication lock cardinality/order mismatch")
    expected_destinations = {item["path"] for item in destinations}
    if {row.get("destination", {}).get("path") for row in rows} != expected_destinations:
        fail("publication lock destination set mismatch")
    for row in rows:
        exact_keys(row, {"destination", "destinationId", "file"}, "publication lock")
        destination = check_file(row["destination"], evidence, "publication destination")
        identifier = destination_id(destination.parent, destination.name)
        if row["destinationId"] != identifier:
            fail("publication destination id mismatch")
        lock = check_file(row["file"], evidence, "publication lock file")
        if lock.parent != destination.parent or lock.name != f".ckc-tune-dest-{identifier}.lock":
            fail("publication lock path mismatch")
        if lock.read_bytes() != b"CKTLCK01" + bytes.fromhex(identifier) or stat.S_IMODE(lock.stat().st_mode) & 0o077:
            fail("publication lock bytes or mode mismatch")


def parse_fields(data: bytes) -> dict[int, bytes]:
    result = {}
    offset = 0
    expected = 1
    while offset < len(data):
        if offset + 6 > len(data):
            fail("truncated tuning record")
        tag = int.from_bytes(data[offset:offset + 2], "big")
        length = int.from_bytes(data[offset + 2:offset + 6], "big")
        end = offset + 6 + length
        if tag != expected or end > len(data):
            fail("noncanonical tuning record")
        result[tag] = data[offset + 6:end]
        offset = end
        expected += 1
    return result


def unwrap_record(data: bytes) -> bytes:
    if len(data) < 4 or int.from_bytes(data[:4], "big") != len(data) - 4:
        fail("invalid tuning record envelope")
    return data[4:]


def record_list(data: bytes) -> list[bytes]:
    if len(data) < 4:
        fail("truncated tuning list")
    count = int.from_bytes(data[:4], "big")
    values = []
    offset = 4
    for _ in range(count):
        if offset + 4 > len(data):
            fail("truncated tuning list record")
        length = int.from_bytes(data[offset:offset + 4], "big")
        end = offset + 4 + length
        if end > len(data):
            fail("truncated tuning list value")
        values.append(data[offset + 4:end])
        offset = end
    if offset != len(data):
        fail("trailing tuning list bytes")
    return values


def digest_list(data: bytes) -> list[str]:
    if len(data) < 4 or (len(data) - 4) % 32:
        fail("invalid tuning digest list")
    count = int.from_bytes(data[:4], "big")
    if len(data) != 4 + count * 32:
        fail("invalid tuning digest count")
    return [data[4 + index * 32:4 + (index + 1) * 32].hex() for index in range(count)]


def parse_text(data: bytes) -> str:
    if len(data) < 4 or int.from_bytes(data[:4], "big") != len(data) - 4:
        fail("invalid tuning text")
    return data[4:].decode("utf-8")


def check_decision_and_attestation(report: dict, evidence: pathlib.Path, external: bool):
    decision = report["decision"]
    exact_keys(decision, {"file", "decisionDigest", "planDigest", "selected"}, "decision")
    path = check_file(decision["file"], evidence, "decision.file")
    data = path.read_bytes()
    if decision["decisionDigest"] != decision["file"]["sha256"] or decision["selected"] is not True:
        fail("decision identity or selected flag mismatch")
    if data[:12] != b"CKTUNE01" + (1).to_bytes(4, "big") or len(data) < 44:
        fail("decision header mismatch")
    if hashlib.sha256(b"CK-TUNING-DECISION\0" + data[:-32]).digest() != data[-32:]:
        fail("decision outer digest mismatch")
    top = parse_fields(data[12:-32])
    if set(top) != set(range(1, 9)):
        fail("decision top-level records mismatch")
    selection = parse_fields(top[7])
    if set(selection) != set(range(1, 6)):
        fail("decision selection record mismatch")
    plan_digest = selection[3].hex()
    if plan_digest != digest(decision["planDigest"], "decision.planDigest"):
        fail("selected plan digest mismatch")
    candidates_record = parse_fields(top[6])
    selected = []
    for candidate_bytes in record_list(candidates_record[2]):
        candidate = parse_fields(candidate_bytes)
        if set(candidate) != set(range(1, 13)):
            fail("decision candidate record mismatch")
        if candidate[6] == b"\x08":
            selected.append(candidate)
    if len(selected) != 1 or selected[0][1].hex() != plan_digest:
        fail("decision does not contain exactly one selected candidate")
    choices = record_list(selected[0][2])
    if len(choices) != 1:
        fail("selected candidate must contain exactly one PlanChoice")
    choice = parse_fields(choices[0])
    if set(choice) != set(range(1, 6)):
        fail("selected PlanChoice record mismatch")
    if choice[3] != b"\x04":
        fail("selected choice is not Loop SIMD")
    unit_id, variant_id = choice[1].hex(), choice[2].hex()

    frontier = parse_fields(top[5])
    units = [parse_fields(item) for item in record_list(frontier[3])]
    units = [item for item in units if item[1].hex() == unit_id]
    if len(units) != 1 or set(units[0]) != set(range(1, 5)) \
            or len(digest_list(units[0][2])) != 1:
        fail("selected TuneUnit is not one-site")
    site_id = digest_list(units[0][2])[0]
    variants = [parse_fields(item) for item in record_list(units[0][4])]
    variants = [item for item in variants if item[1].hex() == variant_id]
    if len(variants) != 1 or set(variants[0]) != set(range(1, 8)) or variants[0][2] != b"\x04":
        fail("selected UnitVariant is not unique Loop SIMD")
    alternatives = record_list(variants[0][3])
    if len(alternatives) != 1:
        fail("selected UnitVariant must contain one SiteAlternative")
    alternative = parse_fields(alternatives[0])
    if set(alternative) != set(range(1, 6)):
        fail("selected SiteAlternative record mismatch")
    if alternative[1].hex() != site_id:
        fail("selected SiteAlternative names another site")
    alternative_id = alternative[2].hex()
    payload = parse_fields(unwrap_record(alternative[5]))
    if set(payload) != {1, 2} or payload[1] != b"\x04":
        fail("selected SiteAlternative payload is not Loop SIMD")
    simd = parse_fields(unwrap_record(payload[2]))
    if set(simd) != {1, 2, 3} or any(len(simd[tag]) != 4 for tag in simd):
        fail("selected Loop SIMD payload is malformed")
    vector_bits = int.from_bytes(simd[1], "big")
    interleave = int.from_bytes(simd[2], "big")
    minimum = int.from_bytes(simd[3], "big")
    if minimum == 0 or minimum > 128:
        fail("selected predicated minimum exceeds 128")

    sites = [parse_fields(item) for item in record_list(frontier[2])]
    sites = [item for item in sites if item[1].hex() == site_id]
    if len(sites) != 1 or set(sites[0]) != set(range(1, 7)) or sites[0][2] != b"\x04":
        fail("selected Loop SIMD site is absent")
    anchor = parse_fields(unwrap_record(sites[0][6]))
    if set(anchor) != {1, 2, 3} or parse_text(anchor[1]) != "floyd":
        fail("selected Loop SIMD site is not floyd")

    attestation = report["attestation"]
    exact_keys(attestation, {"tuned", "replayed", "digest"}, "attestation")
    tuned = check_file(attestation["tuned"], evidence, "attestation.tuned").read_bytes()
    replayed = check_file(attestation["replayed"], evidence, "attestation.replayed").read_bytes()
    if tuned != replayed:
        fail("tuned/replayed attestation bytes differ")
    match = ATTESTATION.fullmatch(tuned.decode("ascii"))
    if match is None:
        fail("attestation line is noncanonical")
    values = match.groups()
    if (values[4], values[5], values[6]) != (unit_id, variant_id, alternative_id):
        fail("attestation selected ids mismatch")
    if (int(values[7]), int(values[8]), int(values[9])) != (vector_bits, interleave, minimum):
        fail("attestation SIMD payload mismatch")
    if (values[10], values[11]) != (choice[4].hex(), choice[5].hex()):
        fail("attestation pre/post state mismatch")
    expected_attestation = hashlib.sha256(
        b"CK-V014-PRED-ATTEST\0" + len(tuned).to_bytes(8, "big") + tuned
    ).hexdigest()
    if attestation["digest"] != expected_attestation:
        fail("attestation digest mismatch")
    for command_name in ["pgoTuned", "replayed"]:
        command_stderr = check_file(
            report["commands"][command_name]["stderr"], evidence, f"{command_name}.stderr"
        ).read_text(encoding="utf-8")
        lines = [line + "\n" for line in command_stderr.splitlines()
                 if line.startswith("CKTUNE-ATTEST/")]
        if lines != [tuned.decode("ascii")]:
            fail(f"{command_name} command attestation mismatch")
    lane_width = vector_bits // 64
    if vector_bits % 64 or lane_width == 0:
        fail("attested vector width is not f64")
    for split, (n, _, _) in SPLITS.items():
        if n < minimum or n < lane_width * interleave or n * n > (1 << 32) - 1:
            fail(f"attested vector body is unreachable for {split}")
    if external:
        result = subprocess.run(
            [str(check_file(report["compiler"], evidence, "compiler")), "tune", "inspect", str(path), "--json"],
            cwd=REPO, env={}, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )
        if result.returncode != 0:
            fail("retained compiler rejects the tuning decision")


def check_artifact(value: dict, evidence: pathlib.Path, name: str):
    exact_keys(value, {"primary", "outputs"}, name)
    if not isinstance(value["outputs"], list) or not value["outputs"]:
        fail(f"{name} has no outputs")
    roles = [item.get("role") for item in value["outputs"]]
    if roles != ["primary", "header"]:
        fail(f"{name} roles/order mismatch")
    for item in value["outputs"]:
        exact_keys(item, {"role", "file"}, f"{name}.output")
        check_file(item["file"], evidence, f"{name}.output.file")
    if value["primary"] != value["outputs"][0]["file"]:
        fail(f"{name} primary mismatch")


def check_profile(value: dict, evidence: pathlib.Path, commands: dict):
    exact_keys(value, {"directory", "shards", "final", "identityDigest", "inspection"}, "profile")
    check_directory(value["directory"], evidence)
    if value["shards"] != value["directory"]["after"]["entries"]:
        fail("profile shard foreign key mismatch")
    check_file(value["final"], evidence, "profile.final")
    inspection_path = check_file(value["inspection"], evidence, "profile.inspection")
    inspection = json.loads(inspection_path.read_text(encoding="utf-8"))
    if inspection.get("identityDigest") != digest(value["identityDigest"], "profile.identityDigest"):
        fail("profile inspection identity mismatch")
    if commands["profileInspect"]["stdout"] != value["inspection"]:
        fail("profile inspection stdout foreign key mismatch")


def repository_relative(path: pathlib.Path) -> str:
    try:
        return path.relative_to(REPO).as_posix()
    except ValueError:
        fail("evidence path is not below the repository")


def evidence_argv_path(identity: dict, evidence: pathlib.Path) -> str:
    if identity.get("root") != "evidence":
        fail("generated argv path is not evidence-rooted")
    return repository_relative(safe_path(evidence, identity["path"]))


def artifact_files(artifact: dict) -> list[dict]:
    return [item["file"] for item in artifact["outputs"]]


def check_build_graph(report: dict, evidence: pathlib.Path):
    commands = report["commands"]
    artifacts = report["artifacts"]
    compiler = evidence_argv_path(report["compiler"], evidence)
    runner = evidence_argv_path(report["runner"], evidence)
    source = report["source"]["path"]
    manifest = report["manifest"]["path"]
    profile = evidence_argv_path(report["profile"]["final"], evidence)
    shards = repository_relative(evidence / report["profile"]["directory"]["path"])
    shard = evidence_argv_path(report["profile"]["shards"][0], evidence)
    decision = evidence_argv_path(report["decision"]["file"], evidence)
    common = ["--kind", "dynamic", "--cpu", "native", "-O3", "--overflow", "unchecked",
              "--bounds", "unchecked", "--pgo-use", profile]

    bases = {
        "generation": "build/generation/artifact",
        "pgoOnly": "build/pgo-only/artifact",
        "pgoTuned": "build/pgo-tuned/artifact",
        "replayed": "build/replayed/artifact",
    }
    for name, base in bases.items():
        expected_paths = [f"{base}.so", f"{base}.h"]
        if [item["file"]["path"] for item in artifacts[name]["outputs"]] != expected_paths:
            fail(f"{name} platform artifact paths mismatch")

    exact_argv = {
        "profileGeneration": [compiler, "build", source, "--out",
                              repository_relative(evidence / bases["generation"]),
                              "--kind", "dynamic", "--cpu", "native", "-O3", "--overflow",
                              "unchecked", "--bounds", "unchecked", "--pgo-generate", shards],
        "profileMerge": [compiler, "pgo", "merge", shard, "--out", profile],
        "profileInspect": [compiler, "pgo", "inspect", profile, "--json"],
        "pgoOnly": [compiler, "build", source, "--out",
                    repository_relative(evidence / bases["pgoOnly"]), *common],
        "pgoTuned": [compiler, "tune", "build", source, "--config", manifest, "--out",
                     repository_relative(evidence / bases["pgoTuned"]), *common,
                     "--budget", "standard", "--tune-out", decision, "--no-tune-cache",
                     "--explain-optimization"],
        "replayed": [compiler, "build", source, "--out",
                     repository_relative(evidence / bases["replayed"]), *common,
                     "--tune-use", decision, "--explain-optimization"],
    }
    for name, argv in exact_argv.items():
        if commands[name]["argv"] != argv:
            fail(f"{name} argv mismatch")

    generation_primary = artifacts["generation"]["primary"]
    header = artifacts["generation"]["outputs"][1]["file"]
    training = commands["trainingRun"]
    if len(training["argv"]) != 6 or training["argv"][:2] != [runner, "--ck-predicated-profile"] \
            or training["argv"][2] != evidence_argv_path(generation_primary, evidence) \
            or re.fullmatch(r"ck_profile_flush_[0-9a-f]{64}", training["argv"][3]) is None \
            or training["argv"][4:] != ["128", "113"]:
        fail("training-run argv mismatch")
    header_text = check_file(header, evidence, "generation header").read_text(encoding="utf-8")
    header_symbols = re.findall(r"ck_profile_flush_[0-9a-f]{64}", header_text)
    if header_symbols != [training["argv"][3]]:
        fail("generation flush symbol/header mismatch")
    profile_fields = check_file(training["stdout"], evidence, "training stdout").read_text(
        encoding="ascii").split()
    if profile_fields != ["CKPREDPROFILE/1", "128", "113", SPLITS["training"][2], "0"]:
        fail("training profile receipt mismatch")
    require_empty_stream(training["stderr"], evidence, "training stderr")

    empty = []
    expected_inputs = {
        "profileGeneration": [report["source"]],
        "trainingRun": sorted([generation_primary, header], key=lambda item: (item["root"], item["path"])),
        "profileMerge": [report["profile"]["shards"][0]],
        "profileInspect": [report["profile"]["final"]],
        "pgoOnly": sorted([report["source"], report["profile"]["final"]], key=lambda item: (item["root"], item["path"])),
        "pgoTuned": sorted([report["source"], report["manifest"], report["profile"]["final"]], key=lambda item: (item["root"], item["path"])),
        "replayed": sorted([report["source"], report["profile"]["final"], report["decision"]["file"]], key=lambda item: (item["root"], item["path"])),
    }
    expected_outputs = {
        "profileGeneration": sorted(artifact_files(artifacts["generation"]), key=lambda item: (item["root"], item["path"])),
        "trainingRun": report["profile"]["shards"],
        "profileMerge": [report["profile"]["final"]],
        "profileInspect": empty,
        "pgoOnly": sorted(artifact_files(artifacts["pgoOnly"]), key=lambda item: (item["root"], item["path"])),
        "pgoTuned": sorted([*artifact_files(artifacts["pgoTuned"]), report["decision"]["file"],
                            *(row["file"] for row in report["publicationLocks"])],
                           key=lambda item: (item["root"], item["path"])),
        "replayed": sorted(artifact_files(artifacts["replayed"]), key=lambda item: (item["root"], item["path"])),
    }
    for name in commands:
        if commands[name]["inputs"] != expected_inputs[name] or commands[name]["outputs"] != expected_outputs[name]:
            fail(f"{name} input/output foreign keys mismatch")
        expected_environment = [] if name in {"trainingRun", "profileMerge", "profileInspect"} else next(
            [{"name": "XDG_CACHE_HOME", "value": repository_relative(evidence / "cache" / name), "references": []}]
            for row in report["cacheScratch"] if row["command"] == name
        )
        if commands[name]["environment"] != expected_environment:
            fail(f"{name} environment mismatch")


def check_frozen_generator():
    mask = (1 << 64) - 1
    def splitmix64(value: int) -> int:
        z = (value + 0x9e3779b97f4a7c15) & mask
        z = ((z ^ (z >> 30)) * 0xbf58476d1ce4e5b9) & mask
        z = ((z ^ (z >> 27)) * 0x94d049bb133111eb) & mask
        return (z ^ (z >> 31)) & mask
    def bits(i: int, j: int, n: int, seed: int) -> str:
        import struct
        if i == j:
            value = 0.0
        else:
            random = splitmix64(seed ^ (i << 32) ^ j)
            if j == (i + 1) % n:
                value = float(1 + random % 16)
            elif ((random >> 8) % 4) == 0:
                value = float("inf")
            else:
                value = float(1 + random % 1024)
        return struct.pack(">d", value).hex()
    expected = [
        "0000000000000000", "4026000000000000", "408b680000000000", "7ff0000000000000",
        "7ff0000000000000", "408b900000000000", "407b800000000000", "4081900000000000",
        "408a980000000000", "4073700000000000", "4072c00000000000", "7ff0000000000000",
        "408c180000000000", "408fd00000000000", "4084c80000000000", "408ba00000000000",
    ]
    if [bits(0, index, 128, 113) for index in range(16)] != expected:
        fail("independent SplitMix64 generator golden cells mismatch")


def require_empty_stream(identity: dict, evidence: pathlib.Path, name: str):
    if check_file(identity, evidence, name).read_bytes():
        fail(f"{name} is not empty")


def check_direct_runner_command(command: dict, evidence: pathlib.Path, runner: dict,
                                argv: list[str], inputs: list[dict]):
    check_command(command, evidence, "direct runner command", runner)
    if command["argv"] != argv or command["inputs"] != sorted(
            inputs, key=lambda item: (item["root"], item["path"])):
        fail("direct runner argv/input mismatch")
    if command["environment"] or command["outputs"]:
        fail("direct runner command has environment or outputs")
    require_empty_stream(command["stderr"], evidence, "direct runner stderr")


def parse_perf_receipt(command: dict, evidence: pathlib.Path, split: str, n: int, seed: int,
                       iterations: int, expected: str) -> dict:
    path = check_file(command["stdout"], evidence, "performance stdout")
    fields = path.read_text(encoding="ascii").split()
    if len(fields) != 8 or fields[:4] != ["CKPREDPERF/1", split, str(n), str(seed)]:
        fail("performance receipt framing mismatch")
    for index in [4, 5, 6]:
        if re.fullmatch(r"[1-9][0-9]*", fields[index]) is None:
            fail("performance receipt integer is noncanonical")
    receipt = {"elapsedNs": u64(int(fields[6]), "receipt.elapsedNs"),
               "iterations": u64(int(fields[4]), "receipt.iterations"),
               "completed": u64(int(fields[5]), "receipt.completed"),
               "correctnessDigest": digest(fields[7], "receipt.correctnessDigest")}
    if receipt["iterations"] != iterations or receipt["completed"] != iterations \
            or receipt["elapsedNs"] <= 0 or receipt["correctnessDigest"] != expected:
        fail("performance receipt value mismatch")
    return receipt


def order_for(candidate_sha: str, split: str, phase: str, row: int) -> list[str]:
    raw = b"CK-V014-PRED-ORDER\0" + text(candidate_sha) + text(split) + text(phase) + row.to_bytes(4, "big")
    rotate = int.from_bytes(hashlib.sha256(raw).digest()[:8], "big") % 2
    channels = ["pgoOnly", "pgoTuned"]
    return channels[rotate:] + channels[:rotate]


def check_timing_split(value: dict, evidence: pathlib.Path, candidate_sha: str, expected_split: str,
                       runner: dict, artifacts: dict, thresholds: dict):
    expected_keys = {"split", "n", "seed", "expectedDigest", "calibration",
                     "calibrationCommands", "confirmationCommand", "warmupOrder", "sampleOrder",
                     "warmupCommands", "sampleCommands", "warmupReceipts", "callReceipts",
                     "callsNs", "samplesNs", "mediansNs", "ratioNum", "ratioDen"}
    exact_keys(value, expected_keys, expected_split)
    n, seed, expected = SPLITS[expected_split]
    if (value["split"], u64(value["n"], "timing.n"), u64(value["seed"], "timing.seed"),
            digest(value["expectedDigest"], "timing.expectedDigest")) != (expected_split, n, seed, expected):
        fail("timing split coordinate mismatch")
    calibration = value["calibration"]
    exact_keys(calibration, {"channel", "attempts", "selectedIterationsPerCall", "confirmation"}, "calibration")
    if calibration["channel"] != "pgoOnly" or not 1 <= len(calibration["attempts"]) <= 32:
        fail("calibration shape mismatch")
    iterations = u64(calibration["selectedIterationsPerCall"], "calibration.iterations")
    runner_argv = evidence_argv_path(runner, evidence)
    def validate_command(command: dict, channel: str, requested: int):
        artifact = artifacts[channel]["primary"]
        expected_argv = [runner_argv, "--ck-predicated-perf", evidence_argv_path(artifact, evidence),
                         expected_split, str(n), str(seed), str(requested)]
        check_direct_runner_command(command, evidence, runner, expected_argv, [artifact])
    expected_iterations = 1
    for index, (attempt, command) in enumerate(zip(calibration["attempts"], value["calibrationCommands"], strict=True)):
        exact_keys(attempt, {"elapsedNs", "iterations", "completed", "correctnessDigest"}, "calibration attempt")
        validate_command(command, "pgoOnly", expected_iterations)
        if attempt != parse_perf_receipt(command, evidence, expected_split, n, seed, expected_iterations, expected):
            fail("calibration attempt/receipt mismatch")
        if index + 1 < len(calibration["attempts"]) and attempt["elapsedNs"] >= 50_000_000:
            fail("calibration continued after reaching duration")
        expected_iterations *= 2
    if iterations != calibration["attempts"][-1]["iterations"] or calibration["attempts"][-1]["elapsedNs"] < 50_000_000:
        fail("calibration selection mismatch")
    validate_command(value["confirmationCommand"], "pgoOnly", iterations)
    confirmation = parse_perf_receipt(value["confirmationCommand"], evidence, expected_split, n, seed, iterations, expected)
    if calibration["confirmation"] != confirmation or confirmation["elapsedNs"] < 50_000_000:
        fail("calibration confirmation mismatch")

    for phase, rows, order_key, command_key, receipt_key, calls_key in [
        ("warmup", 3, "warmupOrder", "warmupCommands", "warmupReceipts", None),
        ("measured", 20, "sampleOrder", "sampleCommands", "callReceipts", "callsNs"),
    ]:
        expected_orders = [order_for(candidate_sha, expected_split, phase, row) for row in range(rows)]
        if value[order_key] != expected_orders:
            fail(f"{phase} order mismatch")
        commands = value[command_key]
        receipts = value[receipt_key]
        if set(commands) != {"pgoOnly", "pgoTuned"} or set(receipts) != set(commands):
            fail(f"{phase} channel set mismatch")
        for channel in commands:
            if len(commands[channel]) != rows or len(receipts[channel]) != rows:
                fail(f"{phase} row count mismatch")
            for command_row, receipt_row in zip(commands[channel], receipts[channel], strict=True):
                if len(command_row) != 3 or len(receipt_row) != 3:
                    fail(f"{phase} calls-per-row mismatch")
                parsed = []
                for command in command_row:
                    validate_command(command, channel, iterations)
                    parsed.append(parse_perf_receipt(command, evidence, expected_split, n, seed, iterations, expected))
                if receipt_row != parsed:
                    fail(f"{phase} retained receipt mismatch")
        if calls_key:
            calls = value[calls_key]
            expected_calls = {channel: [[receipt["elapsedNs"] for receipt in row] for row in receipts[channel]] for channel in receipts}
            if calls != expected_calls:
                fail("measured calls do not equal receipts")

    for mapping in [value["callsNs"], value["samplesNs"], value["mediansNs"]]:
        exact_keys(mapping, {"pgoOnly", "pgoTuned"}, "timing channel map")
    samples = {channel: [min(row) for row in value["callsNs"][channel]] for channel in value["callsNs"]}
    medians = {channel: sorted(rows)[10] for channel, rows in samples.items()}
    if value["samplesNs"] != samples or value["mediansNs"] != medians:
        fail("sample minimum or upper median mismatch")
    for channel, rows in samples.items():
        median = medians[channel]
        stable = sum(median * thresholds["stabilityLowerNum"] <= sample * thresholds["stabilityLowerDen"]
                     and sample * thresholds["stabilityUpperDen"] <= median * thresholds["stabilityUpperNum"]
                     for sample in rows)
        if stable < thresholds["stabilityRequiredRows"]:
            fail(f"{expected_split} {channel} is unstable")
    if (value["ratioNum"], value["ratioDen"]) != (medians["pgoTuned"], medians["pgoOnly"]):
        fail("timing ratio operands mismatch")
    prefix = "validation" if expected_split == "validation" else "release"
    if value["ratioNum"] * thresholds[f"{prefix}MaximumDen"] > value["ratioDen"] * thresholds[f"{prefix}MaximumNum"]:
        fail(f"{expected_split} performance ratio failed")


def collect_evidence_identities(value, found: dict[str, dict]):
    if isinstance(value, dict):
        if set(value) == {"root", "path", "bytes", "sha256"} and value.get("root") == "evidence":
            prior = found.setdefault(value["path"], value)
            if prior != value:
                fail("conflicting repeated evidence identity")
        else:
            for nested in value.values():
                collect_evidence_identities(nested, found)
    elif isinstance(value, list):
        for nested in value:
            collect_evidence_identities(nested, found)


def check_evidence_inventory(report: dict, evidence: pathlib.Path):
    identities = {}
    collect_evidence_identities(report, identities)
    for identity in identities.values():
        check_file(identity, evidence, "evidence inventory")
    actual = []
    for path in evidence.rglob("*"):
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode):
            fail("evidence tree contains a symlink")
        if path.is_file():
            actual.append(path.relative_to(evidence).as_posix())
        elif not path.is_dir():
            fail("evidence tree contains a foreign entry")
    if sorted(identities) != sorted(actual):
        fail("evidence inventory closure mismatch")


def check_report(report: dict, report_path: pathlib.Path, schema_nine: dict | None = None,
                 schema_nine_path: pathlib.Path | None = None, external: bool = False):
    exact_keys(report, TOP_KEYS, "report")
    if u64(report["schemaVersion"], "schemaVersion") != 1 or report["candidateVersion"] != "0.14.0" \
            or re.fullmatch(r"[0-9a-f]{40}", report["candidateSha"] or "") is None:
        fail("report version or candidate SHA mismatch")
    evidence = report_path.parent / report["evidenceDirectory"]
    if re.fullmatch(r"v014-predicated-update-[1-9][0-9]*-[1-9][0-9]*", report["evidenceDirectory"] or "") is None \
            or evidence.name != report["evidenceDirectory"] or not evidence.is_dir() or evidence.is_symlink():
        fail("evidence directory mismatch")
    if external:
        current = subprocess.run(["git", "rev-parse", "HEAD"], cwd=REPO, text=True,
                                 capture_output=True, check=True).stdout.strip()
        if report["candidateSha"] != current:
            fail("report candidate SHA is stale")
    compiler_path = check_file(report["compiler"], evidence, "compiler")
    check_file(report["runner"], evidence, "runner")
    check_file(report["source"], evidence, "source")
    check_file(report["manifest"], evidence, "manifest")
    if report["compiler"].get("root") != "evidence" or report["compiler"].get("path") != "predicated/compiler/ckc" \
            or report["runner"].get("root") != "evidence" \
            or report["runner"].get("path") != "predicated/runner/ckc-tune-runner":
        fail("compiler/runner retained identity mismatch")
    if (report["source"].get("root"), report["source"].get("path")) != \
            ("repository", "benches/fixtures/tune/predicated_update.ck") \
            or (report["manifest"].get("root"), report["manifest"].get("path")) != \
            ("repository", "benches/tune/workloads/predicated-update.cktune.toml"):
        fail("source/manifest identity mismatch")
    exact_keys(report["inputs"], {"training", "validation", "release"}, "inputs")
    input_paths = {
        "training": "benches/fixtures/tune/predicated-update-training.tsv",
        "validation": "benches/fixtures/tune/predicated-update-validation.tsv",
        "release": "benches/fixtures/tune/predicated-update-release.tsv",
    }
    for name, item in report["inputs"].items():
        check_file(item, evidence, "input")
        if (item.get("root"), item.get("path")) != ("repository", input_paths[name]):
            fail("input identity mismatch")
    check_frozen_generator()
    check_recipe(report["recipe"], evidence)
    if schema_nine is not None:
        if report["candidateSha"] != schema_nine.get("candidateSha") or report["toolchain"] != schema_nine.get("toolchain") \
                or report["hardware"] != schema_nine.get("hardware"):
            fail("schema-nine candidate/toolchain/hardware foreign key mismatch")
        schema_evidence = schema_nine_path.parent / schema_nine["evidenceDirectory"]
        candidate = check_file(schema_nine["candidateBinary"], schema_evidence, "schema-nine compiler")
        if candidate.read_bytes() != compiler_path.read_bytes():
            fail("candidate compiler bytes differ from schema nine")

    exact_keys(report["commands"], {"profileGeneration", "trainingRun", "profileMerge", "profileInspect",
                                     "pgoOnly", "pgoTuned", "replayed"}, "commands")
    for name, command in report["commands"].items():
        check_command(command, evidence, f"commands.{name}", report["runner"] if name == "trainingRun" else report["compiler"])
    exact_keys(report["artifacts"], {"generation", "pgoOnly", "pgoTuned", "replayed"}, "artifacts")
    for name, artifact in report["artifacts"].items():
        check_artifact(artifact, evidence, f"artifacts.{name}")
    for tuned, replayed in zip(report["artifacts"]["pgoTuned"]["outputs"], report["artifacts"]["replayed"]["outputs"], strict=True):
        if tuned["role"] != replayed["role"] or tuned["file"]["sha256"] != replayed["file"]["sha256"]:
            fail("tuned/replayed artifact bytes differ")
    check_profile(report["profile"], evidence, report["commands"])
    check_cache_scratch(report["cacheScratch"], evidence, report["commands"])
    check_publication_locks(report["publicationLocks"], evidence, report["artifacts"], report["decision"])
    check_build_graph(report, evidence)
    check_decision_and_attestation(report, evidence, external)

    correctness = report["correctness"]
    exact_keys(correctness, {"training", "validation", "release", "oracleCommands"}, "correctness")
    if (correctness["training"], correctness["validation"], correctness["release"]) != \
            (SPLITS["training"][2], SPLITS["validation"][2], SPLITS["release-held-out"][2]):
        fail("correctness digest table mismatch")
    exact_keys(correctness["oracleCommands"], {"training", "validation", "release"}, "oracle commands")
    for name, split in [("training", "training"), ("validation", "validation"), ("release", "release-held-out")]:
        command = correctness["oracleCommands"][name]
        n, seed, expected = SPLITS[split]
        check_direct_runner_command(
            command, evidence, report["runner"],
            [evidence_argv_path(report["runner"], evidence), "--ck-predicated-oracle",
             split, str(n), str(seed)], [report["inputs"][name]],
        )
        fields = (evidence / command["stdout"]["path"]).read_text(encoding="ascii").split()
        if fields != ["CKPREDORACLE/1", split, str(n), str(seed), expected]:
            fail("oracle command receipt mismatch")
    check_timing_split(report["validation"], evidence, report["candidateSha"], "validation",
                       report["runner"], report["artifacts"], report["recipe"]["thresholds"])
    check_timing_split(report["release"], evidence, report["candidateSha"], "release-held-out",
                       report["runner"], report["artifacts"], report["recipe"]["thresholds"])
    check_evidence_inventory(report, evidence)


def check(report_path: pathlib.Path, schema_nine_path: pathlib.Path):
    result = subprocess.run(
        ["python3", "-B", str(REPO / "scripts/check-native-performance.py"), "--schema", "9", str(schema_nine_path)],
        cwd=REPO, env={},
    )
    if result.returncode != 0:
        fail("schema-nine prerequisite failed")
    report = load_canonical(report_path)
    schema_nine = load_canonical(schema_nine_path)
    check_report(report, report_path, schema_nine, schema_nine_path, external=True)


def main():
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument("report")
    parser.add_argument("--schema-nine", required=True)
    arguments = parser.parse_args()
    check(pathlib.Path(arguments.report).resolve(), pathlib.Path(arguments.schema_nine).resolve())
    print("CK 0.14 predicated-update performance contract passed")


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"check-v014-predicated-update: {error}", file=os.sys.stderr)
        raise SystemExit(1)
