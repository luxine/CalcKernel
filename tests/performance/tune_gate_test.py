"""Schema-9 contract mutation tests; contract fixtures are never performance evidence."""

from __future__ import annotations

import copy
import importlib.util
import json
import pathlib
import tempfile
import unittest
from unittest.mock import patch

REPO = pathlib.Path(__file__).resolve().parents[2]


def load(name: str, path: pathlib.Path):
    specification = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(module)
    return module


gate = load("ckc_schema9_gate", REPO / "scripts/check-native-performance.py")
measure = load("ckc_schema9_measure", REPO / "scripts/measure-v014-performance.py")


class SchemaNineContractTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="ckc-schema9-")
        self.addCleanup(self.temporary.cleanup)
        self.output = pathlib.Path(self.temporary.name) / "v0.14-contract.json"
        self.report = measure.contract_report(self.output)
        self.candidate_sha = self.report["candidateSha"]
        self.sha_patch = patch.object(gate, "current_candidate_sha", lambda: self.candidate_sha)
        self.sha_patch.start()
        self.addCleanup(self.sha_patch.stop)

    def write(self, report=None, *, canonical=True):
        value = self.report if report is None else report
        if canonical:
            text = json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n"
        else:
            text = json.dumps(value, indent=2) + "\n"
        self.output.write_text(text, encoding="utf-8")

    def check(self, report=None):
        self.write(report)
        gate.check(self.output, gate.DEFAULT_BASELINE_MANIFEST, schema_only=True, schema=9)

    def reject(self, mutate, message):
        report = copy.deepcopy(self.report)
        mutate(report)
        self.write(report)
        with self.assertRaisesRegex(ValueError, message):
            gate.check(self.output, gate.DEFAULT_BASELINE_MANIFEST, schema_only=True, schema=9)

    def test_exact_contract_fixture_passes_but_never_full_acceptance(self):
        self.check()
        self.write()
        with self.assertRaisesRegex(ValueError, "not performance acceptance"):
            gate.check(self.output, gate.DEFAULT_BASELINE_MANIFEST, schema=9)

    def test_top_level_version_sha_and_canonical_json_fail_closed(self):
        self.reject(lambda report: report.pop("archiveSize"), "missing")
        self.reject(lambda report: report.__setitem__("unknown", 1), "unknown")
        self.reject(lambda report: report.__setitem__("candidateVersion", "0.14.1"), "version")
        self.reject(lambda report: report.__setitem__("candidateSha", "0" * 40), "candidateSha")
        self.write(canonical=False)
        with self.assertRaisesRegex(ValueError, "canonical"):
            gate.check(self.output, gate.DEFAULT_BASELINE_MANIFEST, schema_only=True, schema=9)
        raw = self.output.read_text().replace(
            '"schemaVersion": 9', '"schemaVersion": 9, "schemaVersion": 9', 1)
        self.output.write_text(raw)
        with self.assertRaisesRegex(ValueError, "duplicate JSON key"):
            gate.check(self.output, gate.DEFAULT_BASELINE_MANIFEST, schema_only=True, schema=9)

    def test_recipe_threshold_identity_and_order_fail_closed(self):
        self.reject(
            lambda report: report["recipe"]["thresholds"].__setitem__(
                "heldOutGeomeanMaximumNum", 96),
            "thresholds",
        )
        self.reject(lambda report: report["recipe"]["files"].reverse(), "set/order")
        self.reject(lambda report: report["recipe"].__setitem__("digest", "a" * 64),
                    "recipe digest")
        self.reject(
            lambda report: report["recipe"]["files"][0].__setitem__("bytes", 1),
            "byte count",
        )

    def test_hardware_sampling_and_partition_contract_fail_closed(self):
        self.reject(lambda report: report["hardware"].__setitem__("logicalCpus", 0),
                    "positive u32")
        self.reject(lambda report: report["hardware"].__setitem__("capabilityDigest", "b" * 64),
                    "capabilityDigest")
        self.reject(lambda report: report["sampling"].__setitem__("sampleRows", 19),
                    "sampling contract")
        self.reject(lambda report: report["workload"]["sources"].pop(), "set/order")
        self.reject(
            lambda report: report["workload"]["expectedResults"][0].__setitem__(
                "digest", "c" * 64),
            "expected result digest",
        )

    def test_path_root_symlink_and_unretained_claims_fail_closed(self):
        self.reject(
            lambda report: report["candidateBinary"].__setitem__("path", "../candidate"),
            "traversing",
        )
        self.reject(
            lambda report: report["candidateBinary"].__setitem__("root", "repository"),
            "wrong root",
        )
        self.reject(lambda report: report["tuningDecisions"].append({}), "measured evidence")
        report = copy.deepcopy(self.report)
        candidate = pathlib.Path(self.temporary.name) / report["evidenceDirectory"] \
            / report["candidateBinary"]["path"]
        candidate.unlink()
        candidate.symlink_to(REPO / "Cargo.toml")
        self.write(report)
        with self.assertRaisesRegex(ValueError, "non-symlink"):
            gate.check(self.output, gate.DEFAULT_BASELINE_MANIFEST, schema_only=True, schema=9)


if __name__ == "__main__":
    unittest.main()
