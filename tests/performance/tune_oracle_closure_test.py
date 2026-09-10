"""Collector artifact selection tests; synthetic files are not performance evidence."""

import ast
import importlib.util
import inspect
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

REPO = Path(__file__).resolve().parents[2]


def load(name, relative):
    specification = importlib.util.spec_from_file_location(name, REPO / relative)
    module = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(module)
    return module


measure = load("oracle_closure_measure", "scripts/measure-v014-performance.py")
gate = load("oracle_closure_gate", "scripts/check-native-performance.py")


class PartitionedOracleClosureTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="ckc-oracle-closure-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.evidence = self.root / "evidence"
        self.evidence.mkdir()
        self.cases = measure.parse_cases()
        with (REPO / "benches/oracles/tune/manifest.toml").open("rb") as source:
            self.manifest = measure.tomllib.load(source)
        for relative in [
            "benches/oracles/tune/manifest.toml", "benches/oracles/tune/c/tune_oracle.c",
            "benches/oracles/tune/rust/tune_oracle.rs",
        ]:
            destination = self.root / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes((REPO / relative).read_bytes())
        repo_patch = patch.object(measure, "REPO", self.root)
        repo_patch.start()
        self.addCleanup(repo_patch.stop)
        self.retained = {}
        for key in ["clang", "rustc", "systemLinker"]:
            relative = f"toolchain/{key}.bin"
            target = self.evidence / relative
            target.parent.mkdir(exist_ok=True)
            target.write_bytes(f"synthetic {key}; never executed".encode())
            self.retained[key] = measure.evidence_identity(self.evidence, relative)
            self.retained[key + "Original"] = str(target)
        for key, relative in [
            ("oracleManifest", "benches/oracles/tune/manifest.toml"),
            ("cOracle", "benches/oracles/tune/c/tune_oracle.c"),
            ("rustOracle", "benches/oracles/tune/rust/tune_oracle.rs"),
        ]:
            self.retained[key] = measure.repository_identity(relative)

    def collect(self):
        build = getattr(measure, "build_case_oracles", None)
        self.assertTrue(callable(build), "collector lacks partition-aware oracle construction")
        records = {}

        def external_compile(command, evidence, executable_override):
            # Replace only external compilation. The real builder still chooses
            # flags/paths, emits file identities and constructs closed records.
            output = self.root / command["argv"][command["argv"].index("-o") + 1]
            output.resolve().relative_to(evidence.resolve())
            output.write_bytes(b"synthetic oracle output; never loaded")
            return 1, ""

        with patch.object(measure, "run_command", side_effect=external_compile):
            for case in self.cases:
                records[case["case"]] = build(case, self.evidence, self.retained, self.manifest)
        return records

    def test_all_seven_cases_generate_only_their_consumed_oracle_channels(self):
        records = self.collect()
        for case in self.cases:
            expected = ({"cSimd", "rustSimd"} if case["partition"] == "eligible"
                        else {"genericC", "genericRust"})
            with self.subTest(case=case["case"]):
                self.assertEqual(set(records[case["case"]]), expected)
        gate.schema9_check_evidence_closure(records, self.evidence)

    def test_unused_domain_simd_files_still_fail_the_original_closure_check(self):
        records = self.collect()
        gate.schema9_check_evidence_closure(records, self.evidence)
        for case in ["contract-fixed-length", "contract-noalias"]:
            for kind in ["c", "rust"]:
                (self.evidence / f"oracles/{case}/{kind}-simd.so").write_bytes(b"unused oracle")
        with self.assertRaisesRegex(ValueError, "closure mismatch.*unknown=.*c-simd"):
            gate.schema9_check_evidence_closure(records, self.evidence)

    def test_missing_required_domain_oracle_still_fails_the_original_closure_check(self):
        records = self.collect()
        (self.evidence / "oracles/contract-noalias/c-generic.so").unlink()
        with self.assertRaisesRegex(ValueError, "closure mismatch.*missing=.*c-generic"):
            gate.schema9_check_evidence_closure(records, self.evidence)

    def test_unknown_partition_is_rejected_before_oracle_generation(self):
        build = getattr(measure, "build_case_oracles", None)
        self.assertTrue(callable(build), "collector lacks partition-aware oracle construction")
        case = {**self.cases[0], "partition": "unrecognized"}
        with self.assertRaisesRegex(ValueError, "partition"):
            build(case, self.evidence, self.retained, self.manifest)
        self.assertFalse((self.evidence / "oracles").exists())

    def test_full_collector_uses_the_partition_boundary_without_unconditional_oracle_builds(self):
        calls = [node.func.id for node in ast.walk(ast.parse(inspect.getsource(measure.full_report)))
                 if isinstance(node, ast.Call) and isinstance(node.func, ast.Name)]
        self.assertEqual(calls.count("build_case_oracles"), 1)
        self.assertNotIn("build_oracle", calls)

    def test_required_runtime_and_validation_channels_remain_unchanged(self):
        self.assertEqual(measure.MAIN_CHANNELS,
                         ["tuned", "v014Ordinary", "v013Ordinary", "v013Pgo", "cSimd", "rustSimd"])
        self.assertEqual(measure.VALIDATION_CHANNELS,
                         ["tuned", "v014Ordinary", "v013Ordinary", "v013Pgo"])
        self.assertEqual(measure.DOMAIN_CHANNELS, ["tuned", "genericC", "genericRust"])


if __name__ == "__main__":
    unittest.main()
