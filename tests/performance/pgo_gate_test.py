"""Schema-8 acceptance tests; synthetic bytes are never performance evidence."""

import contextlib
import copy
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

REPO = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "gate_v013", REPO / "scripts/check-native-performance.py"
)
gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gate)

COLLECTOR_SPEC = importlib.util.spec_from_file_location(
    "collector_v013", REPO / "scripts/measure-v013-performance.py"
)
collector = importlib.util.module_from_spec(COLLECTOR_SPEC)
COLLECTOR_SPEC.loader.exec_module(collector)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def identity(relative):
    data = (REPO / relative).read_bytes()
    return {"path": relative, "bytes": len(data), "sha256": digest(data)}


def order(width, rows):
    return [[(row + offset) % width for offset in range(width)] for row in range(rows)]


def stream(record, prefix, value, count=20):
    record[prefix + "MedianNs"] = value
    record[prefix + "SamplesNs"] = [value] * count


class SchemaEightGateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="ckc-schema8-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.prefix = self.root / "prefix"
        component = self.prefix / "share/ckc/llvm-build.toml"
        component.parent.mkdir(parents=True)
        component.write_bytes(b"pinned component\n")
        self.evidence = self.root / "v013-measurement-1-2"
        self.evidence.mkdir()

        def artifact(case, role):
            filename = f"{case}-{role}.bin"
            data = f"{case}/{role}".encode()
            (self.evidence / filename).write_bytes(data)
            return {"case": case, "role": role, "file": filename,
                    "bytes": len(data), "sha256": digest(data)}

        cases = list(gate.PGO_CASES)
        training = [artifact(case, role) for case in cases for role in ["baseline", "multiversion"]]
        profiles = [artifact(case, role) for case in cases for role in ["baseline", "multiversion"]]
        roles = ["ordinary", "pgo", "multiversion", "combined", "selected-direct", "clang-pgo", "rust-pgo"]
        variants = [artifact(case, role) for case in cases for role in roles]

        candidate = b"synthetic ckc v0.13"
        (self.evidence / "ckc-v013").write_bytes(candidate)
        cumulative = b"{}"
        (self.evidence / "results-schema7.json").write_bytes(cumulative)
        candidate_archive = b"candidate archive"
        (self.evidence / "ckc-v013-distribution.tar.gz").write_bytes(candidate_archive)

        bundle = self.root / "v012"
        bundle.mkdir()
        compiler = b"synthetic ckc v0.12"
        archive = b"exact v0.12 distribution archive bytes"
        (bundle / "ckc-v012").write_bytes(compiler)
        (bundle / "ckc-v012-distribution.tar.gz").write_bytes(archive)
        metadata = {
            "commit": gate.V012_COMMIT,
            "compilerIdentity": gate.V012_COMPILER,
            "compilerSha256": digest(compiler),
            "compilerBytes": str(len(compiler)),
            "llvmVersion": gate.LLVM_VERSION,
            "target": "linux-x86_64",
            "cpuPolicy": "baseline",
            "recipeSha256": gate.named_digest(gate.RECIPE_FILES),
            "adapterSetSha256": digest(b""),
            "sourceDiffSha256": digest(b""),
            "baselineManifestSha256": gate.V012_MANIFEST_SHA256,
            "llvmComponentSha256": digest(component.read_bytes()),
        }
        lines = ["ckc-v012-runtime-replay\t2"]
        lines.extend(f"{key}\t{value}" for key, value in metadata.items())
        lines.append(
            f"distributionArchive\tckc-v012-distribution.tar.gz\t{len(archive)}\t{digest(archive)}"
        )
        manifest = "\n".join(lines) + "\n"
        (bundle / "replay.tsv").write_text(manifest)

        case_rows = []
        for name, flags in gate.PGO_CASES.items():
            row = {
                "name": name, "pgoSensitive": flags[0], "multiversionEligible": flags[1],
                "heldOutOnly": True, "referenceEquivalent": True, "batchCalls": 16,
                "resultDigest": "a" * 64, "resolverCalls": 1 if flags[1] else 0,
                "warmupOrder": order(8, 3), "sampleOrder": order(8, 20),
                "generationMedianNs": 400, "generationSamplesNs": [400] * 20,
            }
            for prefix, value in {
                "ordinary": 100, "replayV012": 100, "pgo": 90, "multiversion": 88,
                "combined": 87, "selectedDirect": 87, "clangPgo": 90, "rustPgo": 92,
            }.items():
                stream(row, prefix, value)
            case_rows.append(row)

        compile_rows = []
        size_rows = []
        for name in cases:
            row = {"case": name, "warmupOrder": order(4, 3), "sampleOrder": order(4, 15)}
            for prefix, value in {"ordinary": 100, "pgo": 120, "multiversion": 200, "combined": 250}.items():
                stream(row, prefix, value, 15)
            compile_rows.append(row)
            size_rows.append({"case": name, "ordinaryBytes": 100, "pgoBytes": 120,
                              "multiversionBytes": 180, "combinedBytes": 190})

        capability_material = {
            "schema": 1, "targetSetSchema": 1, "requiredTier": "x86-64-v3",
            "availableTiers": ["baseline", "x86-64-v3"], "features": ["avx2", "fma"],
            "osState": ["xsave", "ymm"], "resolverPolicy": "resolve-once-before-timing",
        }
        capability = dict(capability_material, digest=gate.canonical_digest(capability_material))
        recipe_files = [identity(name) for name in gate.SCHEMA8_RECIPE_FILES]
        workload_sources = [identity(name) for name in [
            "benches/fixtures/pgo/branch_layout.ck",
            "benches/fixtures/pgo/call_constant_length.ck",
            "benches/oracles/fixtures/map_u32.ck",
            "benches/oracles/fixtures/zip_u32.ck",
            "benches/fixtures/pgo/compute_bound.ck",
        ]]
        self.report = {
            "schemaVersion": 8, "candidateVersion": "0.13.0", "candidateSha": "1" * 40,
            "replayCommit": gate.V012_COMMIT, "evidenceDirectory": self.evidence.name,
            "toolchain": {"llvmVersion": "22.1.8", "clangVersion": "22.1.8",
                          "rustVersion": "1.90.0", "componentManifestSha256": digest(component.read_bytes()),
                          "clangProfileRuntimeSha256": "c" * 64},
            "hardware": {"target": "linux-x86_64", "arch": "x86_64", "os": "linux",
                         "cpuModel": "synthetic-v3", "logicalCpus": 8},
            "capabilityManifest": capability,
            "recipe": {"schema": 1, "files": recipe_files,
                       "digest": gate.named_digest(gate.SCHEMA8_RECIPE_FILES),
                       "thresholds": gate.SCHEMA8_THRESHOLDS},
            "workload": {
                "manifest": identity("benches/cases/pgo-cases.tsv"), "sources": workload_sources,
                "training": identity("benches/fixtures/pgo/training.tsv"),
                "heldOut": identity("benches/fixtures/pgo/held-out.tsv"),
                "adversarial": identity("benches/fixtures/pgo/adversarial.tsv"),
            },
            "candidateBinary": {"file": "ckc-v013", "bytes": len(candidate), "sha256": digest(candidate)},
            "replayBundle": {
                "metadata": metadata, "manifestSha256": digest(manifest.encode()),
                "compiler": {"file": "ckc-v012", "bytes": len(compiler), "sha256": digest(compiler)},
                "archive": {"file": "ckc-v012-distribution.tar.gz", "bytes": len(archive), "sha256": digest(archive)},
            },
            "cumulativeSchemaSeven": {"file": "results-schema7.json", "bytes": len(cumulative),
                                      "sha256": digest(cumulative)},
            "trainingShards": training, "finalProfiles": profiles,
            "targetSets": [
                {"case": case, "policy": policy, "schema": 1, "digest": "b" * 64,
                 "tiers": ["baseline"] if policy == "baseline" else ["baseline", "x86-64-v3"]}
                for case in cases for policy in ["baseline", "multiversion"]
            ],
            "variantObjects": variants,
            "sampling": {
                "protocol": "rotating-eight-channel-v1", "warmupRows": 3, "sampleRows": 20,
                "callsPerSample": 7, "channelNames": gate.PGO_CHANNELS,
                "stabilityPolicy": "at-least-80-percent-within-25-percent-of-median",
                "rerunPolicy": "unstable-evidence-is-invalid-no-selective-rerun",
            },
            "cases": case_rows, "compileTime": compile_rows, "artifactSize": size_rows,
            "archiveSize": {
                "candidateFile": "ckc-v013-distribution.tar.gz", "candidateBytes": len(candidate_archive),
                "candidateSha256": digest(candidate_archive), "replayFile": "ckc-v012-distribution.tar.gz",
                "replayBytes": len(archive), "replaySha256": digest(archive),
            },
            "correctness": {"training": True, "heldOut": True, "adversarial": True,
                            "differential": True, "ubAudit": True, "featureAudit": True},
        }
        self.environment = patch.dict(os.environ, {
            "CKC_V012_RUNTIME_BUNDLE": str(bundle), "CKC_LLVM_PREFIX": str(self.prefix)
        })
        self.environment.start()
        self.addCleanup(self.environment.stop)
        for target, replacement in [
            ("host_target_name", lambda: "linux-x86_64"),
            ("current_candidate_sha", lambda: "1" * 40),
            ("check_schema7", lambda *_: None),
            ("clang_profile_runtime_digest", lambda: "c" * 64),
        ]:
            mocked = patch.object(gate, target, replacement)
            mocked.start()
            self.addCleanup(mocked.stop)

    def check(self, report=None):
        path = self.root / "v0.13-results.json"
        path.write_text(json.dumps(self.report if report is None else report))
        gate.check(path, gate.DEFAULT_BASELINE_MANIFEST)

    def reject(self, mutate, message):
        report = copy.deepcopy(self.report)
        mutate(report)
        with self.assertRaisesRegex(ValueError, message):
            self.check(report)

    def test_complete_schema_eight_passes(self):
        self.check()

    def test_collector_retains_a_self_contained_schema_seven_bundle(self):
        source_root = self.root / "schema-seven-source"
        source_root.mkdir()
        measurement = source_root / "measurement-123-456"
        measurement.mkdir()
        (measurement / "payload.bin").write_bytes(b"measured evidence")
        source = source_root / "results.json"
        source.write_text(json.dumps({"evidenceDirectory": measurement.name}))
        destination = self.root / "schema-eight-evidence"
        destination.mkdir()

        retained = collector.retain_cumulative_schema_seven(source, destination)

        self.assertEqual(retained, destination / "results-schema7.json")
        self.assertEqual(retained.read_bytes(), source.read_bytes())
        self.assertEqual(
            (destination / measurement.name / "payload.bin").read_bytes(),
            b"measured evidence",
        )

    def test_collector_rejects_redirected_or_escaping_schema_seven_evidence(self):
        source_root = self.root / "schema-seven-invalid"
        source_root.mkdir()
        source = source_root / "results.json"
        destination = self.root / "schema-eight-invalid"
        destination.mkdir()

        source.write_text(json.dumps({"evidenceDirectory": "../outside"}))
        with self.assertRaisesRegex(ValueError, "evidenceDirectory"):
            collector.retain_cumulative_schema_seven(source, destination)

        real = source_root / "measurement-real"
        real.mkdir()
        redirected = source_root / "measurement-123-456"
        redirected.symlink_to(real, target_is_directory=True)
        source.write_text(json.dumps({"evidenceDirectory": redirected.name}))
        with self.assertRaisesRegex(ValueError, "real directory"):
            collector.retain_cumulative_schema_seven(source, destination)

    def test_identity_capability_profile_and_evidence_fail_closed(self):
        self.reject(lambda r: r.__setitem__("candidateSha", "2" * 40), "candidateSha")
        self.reject(lambda r: r["capabilityManifest"]["availableTiers"].pop(), "enhanced tier")
        self.reject(lambda r: r["trainingShards"].pop(), "exact case/role")
        self.reject(lambda r: r["finalProfiles"][0].__setitem__("sha256", "f" * 64), "finalProfiles")
        self.reject(lambda r: r["variantObjects"].pop(), "exact case/role")
        self.reject(lambda r: r["recipe"].__setitem__("thresholds", "changed"), "threshold")

    def test_sampling_and_every_threshold_family_fail_closed(self):
        self.reject(lambda r: r["cases"][0].__setitem__("sampleOrder", order(8, 19)), "order")
        self.reject(lambda r: stream(r["cases"][0], "ordinary", 106), "1.05")
        self.reject(lambda r: [stream(row, "pgo", 100) for row in r["cases"] if row["pgoSensitive"]], "1.05")
        self.reject(
            lambda r: [
                (stream(row, "multiversion", 100), stream(row, "selectedDirect", 100))
                for row in r["cases"] if row["multiversionEligible"]
            ],
            "1.08",
        )
        self.reject(lambda r: [stream(row, "selectedDirect", 86) for row in r["cases"] if row["multiversionEligible"]], "0.98")
        self.reject(lambda r: [stream(row, "combined", 110) for row in r["cases"]], "5%")
        self.reject(lambda r: [stream(row, "clangPgo", 70) for row in r["cases"]], "90%")
        self.reject(lambda r: stream(r["compileTime"][0], "pgo", 201, 15), "2x")
        self.reject(lambda r: r["artifactSize"][0].__setitem__("pgoBytes", 151), "1.5x")
        self.reject(lambda r: r["archiveSize"].__setitem__("candidateBytes", 1000), "archive")

    def test_unknown_duplicate_and_changed_workload_are_rejected(self):
        report = copy.deepcopy(self.report)
        report["unknown"] = True
        with self.assertRaisesRegex(ValueError, "unknown"):
            self.check(report)
        self.reject(lambda r: r["targetSets"].append(r["targetSets"][0]), "targetSets")
        self.reject(lambda r: r["workload"]["training"].__setitem__("sha256", "f" * 64), "training")
        path = self.root / "duplicate.json"
        text = json.dumps(self.report).replace('"schemaVersion": 8', '"schemaVersion": 8, "schemaVersion": 8')
        path.write_text(text)
        with self.assertRaisesRegex(ValueError, "duplicate"):
            gate.check(path, gate.DEFAULT_BASELINE_MANIFEST)


if __name__ == "__main__":
    with contextlib.redirect_stdout(io.StringIO()):
        unittest.main()
