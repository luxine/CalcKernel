"""Contract-1 mutation fixtures; these synthetic values are never performance evidence."""

from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
import os
import pathlib
import stat
import tempfile
import unittest

REPO = pathlib.Path(__file__).resolve().parents[2]


def load_gate():
    specification = importlib.util.spec_from_file_location(
        "ckc_predicated_gate", REPO / "scripts/check-v014-predicated-update.py")
    module = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(module)
    return module


gate = load_gate()

RECIPE_FILES = sorted([
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
])
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


def text(value: str) -> bytes:
    raw = value.encode()
    return len(raw).to_bytes(4, "big") + raw


def values(items: list[bytes]) -> bytes:
    return len(items).to_bytes(4, "big") + b"".join(items)


def digest(domain: bytes, *items: bytes) -> str:
    result = hashlib.sha256(domain)
    for item in items:
        result.update(item)
    return result.hexdigest()


def field(tag: int, value: bytes) -> bytes:
    return tag.to_bytes(2, "big") + len(value).to_bytes(4, "big") + value


def fields(*items: bytes) -> bytes:
    return b"".join(field(index + 1, item) for index, item in enumerate(items))


def record(value: bytes) -> bytes:
    return len(value).to_bytes(4, "big") + value


def records(items: list[bytes]) -> bytes:
    return len(items).to_bytes(4, "big") + b"".join(record(item) for item in items)


def dlist(items: list[bytes]) -> bytes:
    return len(items).to_bytes(4, "big") + b"".join(items)


class Fixture:
    def __init__(self):
        target = REPO / "target" / "acceptance" / "v0.14" / "stage-18"
        target.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(prefix="contract-fixture-", dir=target)
        self.parent = pathlib.Path(self.temporary.name)
        self.evidence = self.parent / "v014-predicated-update-1-1"
        self.evidence.mkdir(mode=0o700)
        self.report_path = self.parent / "report.json"
        self.counter = 0
        self.candidate_sha = "1" * 40
        self.ids = {
            "site": bytes.fromhex("21" * 32),
            "unit": bytes.fromhex("22" * 32),
            "variant": bytes.fromhex("23" * 32),
            "alternative": bytes.fromhex("24" * 32),
            "plan": bytes.fromhex("25" * 32),
            "pre": bytes.fromhex("26" * 32),
            "post": bytes.fromhex("27" * 32),
        }
        self.report = self.build()

    def close(self):
        self.temporary.cleanup()

    def write(self, relative: str, data: bytes, mode: int = 0o600) -> dict:
        path = self.evidence / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("xb") as destination:
            destination.write(data)
        path.chmod(mode)
        return self.identity(path)

    def rewrite(self, identity: dict, data: bytes) -> dict:
        path = self.evidence / identity["path"]
        path.write_bytes(data)
        return self.identity(path)

    def identity(self, path: pathlib.Path) -> dict:
        data = path.read_bytes()
        return {
            "root": "evidence",
            "path": path.relative_to(self.evidence).as_posix(),
            "bytes": len(data),
            "sha256": hashlib.sha256(data).hexdigest(),
        }

    def repository_identity(self, relative: str) -> dict:
        path = REPO / relative
        data = path.read_bytes()
        return {"root": "repository", "path": relative, "bytes": len(data),
                "sha256": hashlib.sha256(data).hexdigest()}

    def file_value(self, identity: dict) -> bytes:
        root = {"repository": 1, "evidence": 2}[identity["root"]]
        return (root.to_bytes(1, "big") + text(identity["path"])
                + identity["bytes"].to_bytes(8, "big")
                + bytes.fromhex(identity["sha256"]))

    def repo_path(self, identity: dict) -> str:
        return (self.evidence / identity["path"]).relative_to(REPO).as_posix()

    def command(self, name: str, executable: dict, argv: list[str], *, inputs=None,
                outputs=None, environment=None, stdout=b"", stderr=b"") -> dict:
        self.counter += 1
        prefix = f"commands/{self.counter:04d}-{name}"
        stdout_id = self.write(prefix + ".stdout", stdout)
        stderr_id = self.write(prefix + ".stderr", stderr)
        key = lambda item: (item["root"], item["path"])
        return {
            "argv": argv,
            "workingDirectory": "repository",
            "executable": executable,
            "inputs": sorted(inputs or [], key=key),
            "environment": environment or [],
            "outputs": sorted(outputs or [], key=key),
            "status": 0,
            "stdout": stdout_id,
            "stderr": stderr_id,
        }

    def decision_bytes(self, *, choices=1, minimum=128) -> bytes:
        ids = self.ids
        simd = fields((256).to_bytes(4, "big"), (1).to_bytes(4, "big"),
                      minimum.to_bytes(4, "big"))
        payload = fields(b"\x04", record(simd))
        alternative = fields(ids["site"], ids["alternative"], ids["pre"], ids["post"],
                             record(payload))
        variant = fields(ids["variant"], b"\x04", records([alternative]),
                         (1).to_bytes(8, "big"), (1).to_bytes(8, "big"),
                         (1).to_bytes(8, "big"), ids["post"])
        unit = fields(ids["unit"], dlist([ids["site"]]), ids["pre"], records([variant]))
        anchor = fields(text("floyd"), b"\x02", (0).to_bytes(4, "big"))
        site = fields(ids["site"], b"\x04", bytes.fromhex("28" * 32), ids["pre"],
                      (0).to_bytes(4, "big"), record(anchor))
        frontier = fields(bytes.fromhex("29" * 32), records([site]), records([unit]), records([]))
        choice = fields(ids["unit"], ids["variant"], b"\x04", ids["pre"], ids["post"])
        candidate = fields(
            ids["plan"], records([choice] * choices), bytes.fromhex("31" * 32),
            bytes.fromhex("32" * 32), (1).to_bytes(8, "big"), b"\x08",
            (0).to_bytes(2, "big"), b"\x00", records([]), record(b""), b"\x00",
            bytes.fromhex("33" * 32),
        )
        candidates = fields(record(b""), records([candidate]))
        selection = fields(record(b""), record(b""), ids["plan"], b"\x02", b"\x00")
        body = fields(b"", b"", b"", b"", frontier, candidates, selection, b"")
        checksum = hashlib.sha256(b"CK-TUNING-DECISION\0" + b"CKTUNE01"
                                  + (1).to_bytes(4, "big") + body).digest()
        return b"CKTUNE01" + (1).to_bytes(4, "big") + body + checksum

    def attestation(self, minimum=128) -> bytes:
        ids = self.ids
        line = (
            "CKTUNE-ATTEST/1 shape=predicated-same-place-update function=floyd "
            f"header=0 compare=1 load=2 store=3 unit={ids['unit'].hex()} "
            f"variant={ids['variant'].hex()} alternative={ids['alternative'].hex()} "
            f"vectorBits=256 uf=1 minimum={minimum} pre={ids['pre'].hex()} "
            f"post={ids['post'].hex()}\n"
        )
        return line.encode("ascii")

    def snapshot_digest(self, namespace: str, files: list[dict]) -> str:
        return digest(b"CK-V014-CACHE-SNAPSHOT\0", text(namespace),
                      values([self.file_value(item) for item in files]))

    def make_cache(self, command: str) -> dict:
        namespace = f"cache/{command}/ckc"
        directory = self.evidence / namespace
        directory.mkdir(parents=True, mode=0o700)
        directory.parent.chmod(0o700)
        directory.chmod(0o700)
        metadata = directory.stat()
        lock = self.write(f"cache-locks/{command}.lock", f"CKPREDLOCK/1 {command}\n".encode())
        before = {"namespace": namespace, "files": [],
                  "digest": self.snapshot_digest(namespace, [])}
        cache_file = self.write(f"{namespace}/entry.bin", f"cache-{command}\n".encode())
        after = {"namespace": namespace, "files": [cache_file],
                 "digest": self.snapshot_digest(namespace, [cache_file])}
        receipts = {}
        for phase, snapshot in [("before", before), ("after", after)]:
            contents = (
                f"CKPREDCACHE/1 command={command} phase={phase} device={metadata.st_dev} "
                f"inode={metadata.st_ino} count={len(snapshot['files'])} digest={snapshot['digest']}\n"
            ).encode()
            receipts[phase] = self.write(f"cache-receipts/{command}-{phase}.txt", contents)
        return {
            "command": command, "namespace": namespace, "device": metadata.st_dev,
            "inode": metadata.st_ino, "lock": lock, "before": before, "after": after,
            "beforeReceipt": receipts["before"], "afterReceipt": receipts["after"],
        }

    def destination_id(self, path: pathlib.Path) -> str:
        metadata = path.parent.stat()
        parent = fields(b"\x01", metadata.st_dev.to_bytes(16, "big"),
                        metadata.st_ino.to_bytes(16, "big"), b"\x01")
        return hashlib.sha256(
            b"CK-TUNE-DESTINATION\0" + record(fields(record(parent), text(path.name)))
        ).hexdigest()

    def make_artifact(self, name: str, subdir: str, primary: bytes, header: bytes) -> dict:
        primary_id = self.write(f"build/{subdir}/artifact.so", primary, 0o700)
        header_id = self.write(f"build/{subdir}/artifact.h", header)
        return {"primary": primary_id, "outputs": [
            {"role": "primary", "file": primary_id},
            {"role": "header", "file": header_id},
        ]}

    def make_timing(self, split: str, runner: dict, artifacts: dict) -> dict:
        n, seed, expected = SPLITS[split]
        runner_path = self.repo_path(runner)
        elapsed = {"pgoOnly": 100_000_000,
                   "pgoTuned": 100_000_000 if split == "validation" else 95_000_000}

        def invoke(channel: str, phase: str, row: int, call: int, duration: int) -> tuple[dict, dict]:
            artifact = artifacts[channel]["primary"]
            receipt = {"elapsedNs": duration, "iterations": 1, "completed": 1,
                       "correctnessDigest": expected}
            stdout = f"CKPREDPERF/1 {split} {n} {seed} 1 1 {duration} {expected}\n".encode()
            command = self.command(
                f"{split}-{phase}-{row}-{call}-{channel}", runner,
                [runner_path, "--ck-predicated-perf", self.repo_path(artifact), split,
                 str(n), str(seed), "1"], inputs=[artifact], stdout=stdout,
            )
            return command, receipt

        calibration_command, attempt = invoke("pgoOnly", "calibration", 0, 0, 50_000_000)
        confirmation_command, confirmation = invoke("pgoOnly", "confirmation", 0, 0, 50_000_000)

        def order(phase: str, row: int) -> list[str]:
            material = (b"CK-V014-PRED-ORDER\0" + text(self.candidate_sha) + text(split)
                        + text(phase) + row.to_bytes(4, "big"))
            channels = ["pgoOnly", "pgoTuned"]
            rotate = int.from_bytes(hashlib.sha256(material).digest()[:8], "big") % 2
            return channels[rotate:] + channels[:rotate]

        def rows(phase: str, count: int):
            orders = []
            commands = {"pgoOnly": [], "pgoTuned": []}
            receipts = {"pgoOnly": [], "pgoTuned": []}
            calls = {"pgoOnly": [], "pgoTuned": []}
            for row in range(count):
                scheduled = order(phase, row)
                orders.append(scheduled)
                row_commands = {channel: [] for channel in commands}
                row_receipts = {channel: [] for channel in commands}
                for channel in scheduled:
                    for call in range(3):
                        command, receipt = invoke(channel, phase, row, call, elapsed[channel])
                        row_commands[channel].append(command)
                        row_receipts[channel].append(receipt)
                for channel in commands:
                    commands[channel].append(row_commands[channel])
                    receipts[channel].append(row_receipts[channel])
                    calls[channel].append([item["elapsedNs"] for item in row_receipts[channel]])
            return orders, commands, receipts, calls

        warm_order, warm_commands, warm_receipts, _ = rows("warmup", 3)
        sample_order, sample_commands, call_receipts, calls = rows("measured", 20)
        samples = {channel: [min(row) for row in calls[channel]] for channel in calls}
        medians = {channel: sorted(samples[channel])[10] for channel in samples}
        return {
            "split": split, "n": n, "seed": seed, "expectedDigest": expected,
            "calibration": {"channel": "pgoOnly", "attempts": [attempt],
                            "selectedIterationsPerCall": 1, "confirmation": confirmation},
            "calibrationCommands": [calibration_command],
            "confirmationCommand": confirmation_command,
            "warmupOrder": warm_order, "sampleOrder": sample_order,
            "warmupCommands": warm_commands, "sampleCommands": sample_commands,
            "warmupReceipts": warm_receipts, "callReceipts": call_receipts,
            "callsNs": calls, "samplesNs": samples, "mediansNs": medians,
            "ratioNum": medians["pgoTuned"], "ratioDen": medians["pgoOnly"],
        }

    def build(self) -> dict:
        compiler = self.write("predicated/compiler/ckc", b"#!/bin/sh\nexit 0\n", 0o700)
        runner = self.write("predicated/runner/ckc-tune-runner", b"#!/bin/sh\nexit 0\n", 0o700)
        source = self.repository_identity("benches/fixtures/tune/predicated_update.ck")
        manifest = self.repository_identity("benches/tune/workloads/predicated-update.cktune.toml")
        inputs = {name: self.repository_identity(path) for name, path in {
            "training": "benches/fixtures/tune/predicated-update-training.tsv",
            "validation": "benches/fixtures/tune/predicated-update-validation.tsv",
            "release": "benches/fixtures/tune/predicated-update-release.tsv",
        }.items()}
        flush = "ck_profile_flush_" + "a" * 64
        generation_header = f"int {flush}(void);\n".encode()
        artifacts = {
            "generation": self.make_artifact("generation", "generation", b"generation\n", generation_header),
            "pgoOnly": self.make_artifact("pgoOnly", "pgo-only", b"pgo-only\n", b"header\n"),
            "pgoTuned": self.make_artifact("pgoTuned", "pgo-tuned", b"tuned\n", b"tuned-header\n"),
            "replayed": self.make_artifact("replayed", "replayed", b"tuned\n", b"tuned-header\n"),
        }
        decision_file = self.write("build/pgo-tuned/decision.cktune", self.decision_bytes())
        decision = {"file": decision_file, "decisionDigest": decision_file["sha256"],
                    "planDigest": self.ids["plan"].hex(), "selected": True}
        attestation_bytes = self.attestation()
        tuned_attestation = self.write("attestation/tuned.txt", attestation_bytes)
        replay_attestation = self.write("attestation/replayed.txt", attestation_bytes)
        attestation = {
            "tuned": tuned_attestation,
            "replayed": replay_attestation,
            "digest": hashlib.sha256(b"CK-V014-PRED-ATTEST\0"
                                     + len(attestation_bytes).to_bytes(8, "big")
                                     + attestation_bytes).hexdigest(),
        }

        shard_directory = self.evidence / "profile/shards"
        shard_directory.mkdir(parents=True)
        directory_metadata = shard_directory.stat()
        shard = self.write("profile/shards/training.ckpart", b"profile shard\n")
        before_digest = digest(b"CK-V014-PRED-DIRECTORY\0", text("before"), values([]))
        after_digest = digest(b"CK-V014-PRED-DIRECTORY\0", text("after"),
                              values([self.file_value(shard)]))
        before_receipt = self.write(
            "profile/shards-before.txt",
            (f"CKPREDDIR/1 phase=before device={directory_metadata.st_dev} "
             f"inode={directory_metadata.st_ino} count=0 digest={before_digest}\n").encode())
        after_receipt = self.write(
            "profile/shards-after.txt",
            (f"CKPREDDIR/1 phase=after device={directory_metadata.st_dev} "
             f"inode={directory_metadata.st_ino} count=1 digest={after_digest}\n").encode())
        profile_final = self.write("profile/predicated.ckprof", b"CKPROF01 fixture\n")
        profile_identity = "41" * 32
        profile = {
            "directory": {"root": "evidence", "path": "profile/shards",
                          "device": directory_metadata.st_dev, "inode": directory_metadata.st_ino,
                          "before": {"entries": [], "digest": before_digest,
                                     "receipt": before_receipt},
                          "after": {"entries": [shard], "digest": after_digest,
                                    "receipt": after_receipt}},
            "shards": [shard], "final": profile_final,
            "identityDigest": profile_identity, "inspection": None,
        }
        caches = [self.make_cache(name) for name in
                  ["profileGeneration", "pgoOnly", "pgoTuned", "replayed"]]
        cache_env = {
            row["command"]: [{"name": "XDG_CACHE_HOME",
                              "value": (self.evidence / "cache" / row["command"])
                              .relative_to(REPO).as_posix(), "references": []}]
            for row in caches
        }

        locks = []
        destinations = [decision_file, *(item["file"] for item in artifacts["pgoTuned"]["outputs"])]
        for destination in destinations:
            path = self.evidence / destination["path"]
            identifier = self.destination_id(path)
            lock = self.write(
                f"{path.parent.relative_to(self.evidence).as_posix()}/.ckc-tune-dest-{identifier}.lock",
                b"CKTLCK01" + bytes.fromhex(identifier),
            )
            locks.append({"destination": destination, "destinationId": identifier, "file": lock})
        locks.sort(key=lambda row: row["file"]["path"])

        compiler_path = self.repo_path(compiler)
        runner_path = self.repo_path(runner)
        profile_path = self.repo_path(profile_final)
        shard_path = self.repo_path(shard)
        decision_path = self.repo_path(decision_file)
        common = ["--kind", "dynamic", "--cpu", "native", "-O3", "--overflow", "unchecked",
                  "--bounds", "unchecked", "--pgo-use", profile_path]
        base = lambda subdir: (self.evidence / f"build/{subdir}/artifact").relative_to(REPO).as_posix()
        artifact_files = lambda name: [item["file"] for item in artifacts[name]["outputs"]]
        commands = {
            "profileGeneration": self.command(
                "profile-generation", compiler,
                [compiler_path, "build", source["path"], "--out", base("generation"),
                 "--kind", "dynamic", "--cpu", "native", "-O3", "--overflow", "unchecked",
                 "--bounds", "unchecked", "--pgo-generate",
                 (shard_directory.relative_to(REPO)).as_posix()],
                inputs=[source], outputs=artifact_files("generation"),
                environment=cache_env["profileGeneration"]),
            "trainingRun": self.command(
                "training-run", runner,
                [runner_path, "--ck-predicated-profile", self.repo_path(artifacts["generation"]["primary"]),
                 flush, "128", "113"], inputs=artifact_files("generation"), outputs=[shard],
                stdout=f"CKPREDPROFILE/1 128 113 {SPLITS['training'][2]} 0\n".encode()),
            "profileMerge": self.command(
                "profile-merge", compiler, [compiler_path, "pgo", "merge", shard_path,
                                             "--out", profile_path], inputs=[shard],
                outputs=[profile_final]),
            "profileInspect": self.command(
                "profile-inspect", compiler, [compiler_path, "pgo", "inspect", profile_path, "--json"],
                inputs=[profile_final], stdout=(json.dumps({"identityDigest": profile_identity},
                                                           separators=(",", ":")) + "\n").encode()),
            "pgoOnly": self.command(
                "pgo-only", compiler, [compiler_path, "build", source["path"], "--out",
                                       base("pgo-only"), *common],
                inputs=[source, profile_final], outputs=artifact_files("pgoOnly"),
                environment=cache_env["pgoOnly"]),
            "pgoTuned": self.command(
                "pgo-tuned", compiler, [compiler_path, "tune", "build", source["path"],
                                        "--config", manifest["path"], "--out", base("pgo-tuned"),
                                        *common, "--budget", "standard", "--tune-out", decision_path,
                                        "--no-tune-cache", "--explain-optimization"],
                inputs=[source, manifest, profile_final],
                outputs=[*artifact_files("pgoTuned"), decision_file, *(row["file"] for row in locks)],
                environment=cache_env["pgoTuned"], stderr=attestation_bytes),
            "replayed": self.command(
                "replayed", compiler, [compiler_path, "build", source["path"], "--out",
                                       base("replayed"), *common, "--tune-use", decision_path,
                                       "--explain-optimization"],
                inputs=[source, profile_final, decision_file], outputs=artifact_files("replayed"),
                environment=cache_env["replayed"], stderr=attestation_bytes),
        }
        profile["inspection"] = commands["profileInspect"]["stdout"]

        oracle_commands = {}
        for name, split in [("training", "training"), ("validation", "validation"),
                            ("release", "release-held-out")]:
            n, seed, expected = SPLITS[split]
            oracle_commands[name] = self.command(
                f"oracle-{name}", runner,
                [runner_path, "--ck-predicated-oracle", split, str(n), str(seed)],
                inputs=[inputs[name]], stdout=f"CKPREDORACLE/1 {split} {n} {seed} {expected}\n".encode())
        correctness = {"training": SPLITS["training"][2],
                       "validation": SPLITS["validation"][2],
                       "release": SPLITS["release-held-out"][2],
                       "oracleCommands": oracle_commands}

        recipe_files = [self.repository_identity(path) for path in RECIPE_FILES]
        threshold_values = [text(name) + value.to_bytes(8, "big")
                            for name, value in sorted(THRESHOLDS.items())]
        recipe_digest = digest(b"CK-V014-PRED-RECIPE\0", (1).to_bytes(4, "big"),
                               values([self.file_value(item) for item in recipe_files]),
                               values(threshold_values))
        report = {
            "schemaVersion": 1, "candidateVersion": "0.14.0",
            "candidateSha": self.candidate_sha, "evidenceDirectory": self.evidence.name,
            "toolchain": {}, "hardware": {},
            "recipe": {"schema": 1, "files": recipe_files,
                       "thresholds": copy.deepcopy(THRESHOLDS), "digest": recipe_digest},
            "compiler": compiler, "runner": runner, "source": source, "inputs": inputs,
            "profile": profile, "manifest": manifest, "decision": decision,
            "attestation": attestation, "artifacts": artifacts, "publicationLocks": locks,
            "cacheScratch": caches, "commands": commands, "correctness": correctness,
            "validation": self.make_timing("validation", runner, artifacts),
            "release": self.make_timing("release-held-out", runner, artifacts),
        }
        return report

    def replace_identity(self, value, old: dict, new: dict):
        if isinstance(value, dict):
            if value == old:
                value.clear()
                value.update(new)
            else:
                for nested in value.values():
                    self.replace_identity(nested, old, new)
        elif isinstance(value, list):
            for nested in value:
                self.replace_identity(nested, old, new)

    def replace_decision(self, report: dict, *, choices=1, minimum=128):
        old = copy.deepcopy(report["decision"]["file"])
        new = self.rewrite(old, self.decision_bytes(choices=choices, minimum=minimum))
        self.replace_identity(report, old, new)
        report["decision"]["decisionDigest"] = new["sha256"]


class PredicatedUpdateGateTests(unittest.TestCase):
    def setUp(self):
        self.fixture = Fixture()
        self.addCleanup(self.fixture.close)

    def check(self, report=None):
        gate.check_report(report or self.fixture.report, self.fixture.report_path, external=False)

    def reject(self, mutate, message: str):
        report = copy.deepcopy(self.fixture.report)
        mutate(report)
        with self.assertRaisesRegex((ValueError, KeyError, TypeError), message):
            self.check(report)

    def test_valid_independent_fixture_passes(self):
        self.check()

    def test_top_level_missing_unknown_and_noncanonical_json_fail_closed(self):
        self.reject(lambda report: report.pop("profile"), "missing or unknown")
        self.reject(lambda report: report.__setitem__("unknown", 1), "missing or unknown")
        raw = json.dumps(self.fixture.report, indent=2) + "\n"
        self.fixture.report_path.write_text(raw, encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "canonical"):
            gate.load_canonical(self.fixture.report_path)
        self.fixture.report_path.write_text('{"schemaVersion":1,"schemaVersion":1}\n')
        with self.assertRaisesRegex(ValueError, "duplicate JSON key"):
            gate.load_canonical(self.fixture.report_path)

    def test_recipe_threshold_order_and_digest_fail_closed(self):
        self.reject(lambda report: report["recipe"]["thresholds"].__setitem__(
            "releaseMaximumNum", 96), "threshold")
        self.reject(lambda report: report["recipe"]["files"].reverse(), "set/order")
        self.reject(lambda report: report["recipe"].__setitem__("digest", "a" * 64),
                    "recipe digest")

    def test_decision_attestation_compound_and_unreachable_variants_fail_closed(self):
        self.reject(lambda report: report["attestation"].__setitem__("digest", "b" * 64),
                    "attestation digest")
        compound = copy.deepcopy(self.fixture.report)
        self.fixture.replace_decision(compound, choices=2)
        with self.assertRaisesRegex(ValueError, "exactly one PlanChoice"):
            self.check(compound)
        unreachable = copy.deepcopy(self.fixture.report)
        self.fixture.replace_decision(unreachable, minimum=2048)
        with self.assertRaisesRegex(ValueError, "minimum exceeds 128"):
            self.check(unreachable)

    def test_ratio_order_receipt_and_cardinality_fail_closed(self):
        self.reject(lambda report: report["release"].__setitem__("ratioNum", 96_000_000),
                    "ratio operands")
        self.reject(lambda report: report["validation"]["sampleOrder"][0].reverse(),
                    "order mismatch")
        self.reject(lambda report: report["release"]["callReceipts"]["pgoOnly"][0].pop(),
                    "calls-per-row")
        self.reject(lambda report: report["validation"]["sampleCommands"]["pgoTuned"].pop(),
                    "row count")

    def test_command_profile_cache_lock_and_inventory_foreign_keys_fail_closed(self):
        self.reject(lambda report: report["commands"]["pgoOnly"]["argv"].append("--extra"),
                    "argv mismatch")
        self.reject(lambda report: report["profile"].__setitem__("shards", []),
                    "shard foreign key")
        self.reject(lambda report: report["cacheScratch"][0]["before"]["files"].append(
            report["cacheScratch"][0]["after"]["files"][0]), "snapshot digest")
        self.reject(lambda report: report["publicationLocks"][0].__setitem__(
            "destinationId", "c" * 64), "destination id")
        (self.fixture.evidence / "unidentified.bin").write_bytes(b"unlisted\n")
        with self.assertRaisesRegex(ValueError, "inventory closure"):
            self.check()


if __name__ == "__main__":
    unittest.main()
