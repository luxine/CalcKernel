"""Schema-9 contract mutation tests; contract fixtures are never performance evidence."""

from __future__ import annotations

import copy
import importlib.util
import inspect
import json
import os
import pathlib
import stat
import tempfile
import unittest
from unittest.mock import Mock, patch

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
                "ordinaryRuntimeCaseMaximumNum", 104),
            "thresholds",
        )
        self.reject(lambda report: report["recipe"]["files"].reverse(), "set/order")
        self.reject(lambda report: report["recipe"].__setitem__("digest", "a" * 64),
                    "recipe digest")
        self.reject(
            lambda report: report["recipe"]["files"][0].__setitem__("bytes", 1),
            "byte count",
        )

    def test_checker_does_not_hard_compare_unprofiled_tuning_with_v013_pgo(self):
        source = inspect.getsource(gate.schema9_check_runtime_gates_v2)
        unequal_comparator = (
            'min(row["mediansNs"]["v013Ordinary"], '
            'row["mediansNs"]["v013Pgo"])'
        )

        self.assertNotIn(unequal_comparator, source)
        self.assertIn('"tuned", "v014Ordinary"', source)
        self.assertIn('"v014Ordinary", "v013Ordinary"', source)

    @staticmethod
    def runtime_fixture(
            *, selected=("branch-layout", "call-constant-length"),
            gains=("branch-layout", "call-constant-length")):
        def identity(case, channel, same_as_ordinary):
            discriminator = "ordinary" if same_as_ordinary else channel
            digest = gate.hashlib.sha256(f"{case}:{discriminator}".encode()).hexdigest()
            return {"path": f"{case}-{channel}.bin", "bytes": 4096, "sha256": digest}

        def row(case):
            tuned = 95 if case in gains else 100
            is_fallback = case not in selected
            return {
                "case": case,
                "mediansNs": {
                    "tuned": tuned,
                    "v014Ordinary": 100,
                    "v013Ordinary": 100,
                    "v013Pgo": 20,
                },
                "samplesNs": {
                    "tuned": [tuned] * 20,
                    "v014Ordinary": [100] * 20,
                    "v013Ordinary": [100] * 20,
                    "v013Pgo": [20] * 20,
                },
                "artifacts": {
                    "tuned": identity(case, "tuned", is_fallback),
                    "v014Ordinary": identity(case, "v014Ordinary", True),
                },
            }

        decisions = {
            case: {"selectionReason": "tuned" if case in selected else "no-candidate"}
            for case in gate.SCHEMA9_CASES
        }
        main = [row(case) for case in sorted(gate.SCHEMA9_MAIN_CASES)]
        validation = [row(case) for case in sorted(gate.SCHEMA9_CASES)]
        return main, validation, decisions

    def test_revision_two_is_like_for_like_and_pgo_is_observational(self):
        main, validation, decisions = self.runtime_fixture()
        gate.schema9_check_runtime_gates(
            main, validation, decisions, 2, gate.SCHEMA9_THRESHOLDS)

        for row in [*main, *validation]:
            row["mediansNs"]["v013Pgo"] = 1
            row["samplesNs"]["v013Pgo"] = [1] * 20
        gate.schema9_check_runtime_gates(
            main, validation, decisions, 2, gate.SCHEMA9_THRESHOLDS)

        with self.assertRaisesRegex(ValueError, "held-out geometric performance"):
            gate.schema9_check_runtime_gates_v1(
                main, validation, decisions, gate.SCHEMA9_THRESHOLDS_V1)

    def test_revision_two_requires_two_credible_held_out_gains(self):
        main, validation, decisions = self.runtime_fixture(
            selected=("branch-layout",), gains=("branch-layout",))
        with self.assertRaisesRegex(ValueError, "at least 2 credible tuned gains"):
            gate.schema9_check_runtime_gates(
                main, validation, decisions, 2, gate.SCHEMA9_THRESHOLDS)

    def test_revision_two_rejects_credible_tuned_and_ordinary_regressions(self):
        main, validation, decisions = self.runtime_fixture()
        target = next(row for row in main if row["case"] == "compute-bound")
        target["mediansNs"]["tuned"] = 104
        target["samplesNs"]["tuned"] = [104] * 20
        with self.assertRaisesRegex(ValueError, "tuned runtime credible regression exceeds 3%"):
            gate.schema9_check_runtime_gates(
                main, validation, decisions, 2, gate.SCHEMA9_THRESHOLDS)

    def test_revision_two_rejects_credible_geometric_mean_regressions(self):
        main, validation, decisions = self.runtime_fixture()
        for row in main:
            row["mediansNs"]["v014Ordinary"] = 101
            row["samplesNs"]["v014Ordinary"] = [101] * 20
        with self.assertRaisesRegex(ValueError, "ordinary runtime.*geometric-mean regression"):
            gate.schema9_check_runtime_gates(
                main, validation, decisions, 2, gate.SCHEMA9_THRESHOLDS)

        main, validation, decisions = self.runtime_fixture(
            selected=tuple(gate.SCHEMA9_CASES), gains=tuple(gate.SCHEMA9_CASES))
        for index, row in enumerate(main):
            tuned = 97 if index < 2 else 103
            row["mediansNs"]["tuned"] = tuned
            row["samplesNs"]["tuned"] = [tuned] * 20
            if index >= 2:
                row["samplesNs"]["tuned"] = [tuned] * 10 + [96] * 10
        with self.assertRaisesRegex(ValueError, "tuned runtime geometric-mean parity"):
            gate.schema9_check_runtime_gates(
                main, validation, decisions, 2, gate.SCHEMA9_THRESHOLDS)

        main, validation, decisions = self.runtime_fixture()
        target = next(row for row in validation if row["case"] == "contract-noalias")
        target["mediansNs"]["v014Ordinary"] = 104
        target["samplesNs"]["v014Ordinary"] = [104] * 20
        with self.assertRaisesRegex(ValueError, "ordinary runtime credible regression exceeds 3%"):
            gate.schema9_check_runtime_gates(
                main, validation, decisions, 2, gate.SCHEMA9_THRESHOLDS)

    def test_revision_two_requires_reliable_validation_or_ordinary_fallback(self):
        main, validation, decisions = self.runtime_fixture()
        target = next(row for row in validation if row["case"] == "branch-layout")
        target["mediansNs"]["tuned"] = 100
        target["samplesNs"]["tuned"] = [100] * 20
        with self.assertRaisesRegex(ValueError, "selected candidate lacks a credible validation gain"):
            gate.schema9_check_runtime_gates(
                main, validation, decisions, 2, gate.SCHEMA9_THRESHOLDS)

        main, validation, decisions = self.runtime_fixture()
        target = next(row for row in main if row["case"] == "compute-bound")
        target["artifacts"]["tuned"]["sha256"] = "f" * 64
        with self.assertRaisesRegex(ValueError, "fallback artifact differs"):
            gate.schema9_check_runtime_gates(
                main, validation, decisions, 2, gate.SCHEMA9_THRESHOLDS)

    def test_current_contract_uses_recipe_revision_two_and_like_for_like_validation(self):
        self.assertEqual(self.report["recipe"]["schema"], 2)
        self.assertEqual(
            self.report["sampling"]["validationProtocol"],
            "rotating-four-channel-v2",
        )
        self.assertEqual(
            self.report["sampling"]["validationChannels"],
            ["tuned", "v014Ordinary", "v013Ordinary", "v013Pgo"],
        )

    def test_recipe_revision_one_contract_remains_readable(self):
        report = copy.deepcopy(self.report)
        thresholds = gate.SCHEMA9_THRESHOLDS_V1
        threshold_values = [
            measure.text(name) + value.to_bytes(8, "big")
            for name, value in sorted(thresholds.items())
        ]
        report["recipe"].update({
            "schema": 1,
            "thresholds": thresholds,
            "digest": measure.p(
                b"CK-V014-PERF-RECIPE\0",
                (1).to_bytes(4, "big"),
                measure.list_value([
                    measure.file_value(item) for item in report["recipe"]["files"]
                ]),
                measure.list_value(threshold_values),
            ),
        })
        report["sampling"]["validationProtocol"] = "rotating-three-channel-v1"
        report["sampling"]["validationChannels"] = [
            "tuned", "v013Ordinary", "v013Pgo",
        ]

        self.check(report)

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

    def test_x86_64_v4_requires_avx512cd(self):
        hardware = copy.deepcopy(self.report["hardware"])
        hardware.update({
            "target": "x86_64-unknown-linux-gnu", "arch": "x86_64", "os": "linux",
            "requiredTier": "x86-64-v4", "availableTiers": ["baseline", "x86-64-v4"],
            "features": ["avx512bw", "avx512cd", "avx512dq", "avx512f", "avx512vl"],
        })

        def seal(value):
            material = [gate.schema9_text(value[key]) for key in [
                "target", "arch", "os", "osBuild", "kernel", "cpuModel",
            ]]
            material += [value[key].to_bytes(4, "big") for key in [
                "logicalCpus", "physicalCpus", "numaNodes",
            ]]
            material += [
                gate.schema9_list([gate.schema9_text(item) for item in value["features"]]),
                gate.schema9_text(value["requiredTier"]),
                gate.schema9_list([gate.schema9_text(item) for item in value["availableTiers"]]),
                gate.schema9_text(value["osState"]),
            ]
            value["capabilityDigest"] = gate.schema9_digest(
                b"CK-V014-PERF-HARDWARE\0", *material)

        seal(hardware)
        gate.check_schema9_hardware(hardware, False)
        hardware["features"].remove("avx512cd")
        seal(hardware)
        with self.assertRaisesRegex(ValueError, "required hardware features"):
            gate.check_schema9_hardware(hardware, False)

    def test_missing_required_tier_is_an_actionable_runner_capability_failure(self):
        cpuinfo = "model name : test v3 runner\nflags : avx2 fma bmi2\n"
        with patch.object(measure.platform, "system", return_value="Linux"), \
                patch.object(measure.platform, "machine", return_value="x86_64"), \
                patch.object(measure.pathlib.Path, "read_text", return_value=cpuinfo), \
                patch.object(measure.pathlib.Path, "glob", return_value=[]), \
                patch.object(measure.os, "cpu_count", return_value=8):
            with self.assertRaisesRegex(
                ValueError,
                r"schema-9 infrastructure failure: runner capability.*"
                r"required tier=x86-64-v4.*missing features=.*avx512f.*"
                r"cpu=test v3 runner",
            ):
                measure.full_hardware("a" * 40, "x86_64-unknown-linux-gnu")

    def test_full_collection_checks_hardware_before_tuning_setup(self):
        events = []

        def reject_hardware(*_arguments):
            events.append("hardware")
            raise ValueError("missing required hardware")

        def stop_before_tuning():
            events.append("tuning setup")
            raise ValueError("unexpected tuning setup")

        retained = {
            "target": "x86_64-unknown-linux-gnu",
            "candidate": {"path": "candidate"},
            "replay": {"compiler": {"path": "replay"}},
        }
        with patch.object(measure.platform, "system", return_value="Linux"), \
                patch.object(measure, "git_sha", return_value="a" * 40), \
                patch.object(measure, "prepare_full_retained", return_value=retained), \
                patch.object(measure, "full_hardware", side_effect=reject_hardware), \
                patch.object(measure, "parse_cases", side_effect=stop_before_tuning):
            with self.assertRaises(ValueError):
                measure.full_report(pathlib.Path(self.temporary.name) / "full/results.json")
        self.assertEqual(events, ["hardware"])

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

    def test_evidence_closure_rejects_unidentified_files(self):
        evidence = pathlib.Path(self.temporary.name) / self.report["evidenceDirectory"]
        gate.schema9_check_evidence_closure(self.report, evidence)
        (evidence / "unidentified.bin").write_bytes(b"not evidence")
        with self.assertRaisesRegex(ValueError, "closure mismatch"):
            gate.schema9_check_evidence_closure(self.report, evidence)

    def test_decoded_decision_output_bytes_remain_json_u64_numbers(self):
        inspection = (REPO / "tests/fixtures/tune/decision-schema1-inspection.json").read_text(
            encoding="utf-8"
        )
        completed = Mock(returncode=0, stdout=inspection)
        with patch.object(gate.subprocess, "run", return_value=completed):
            summary, _ = gate.schema9_inspect_decision(
                pathlib.Path("ckc"), pathlib.Path("decision.cktune"), "fixture"
            )

        self.assertTrue(summary["outputRecords"])
        self.assertIs(type(summary["outputRecords"][0]["bytes"]), int)

    def test_command_inputs_use_normative_repository_before_evidence_order(self):
        source = self.report["workload"]["sources"][0]
        candidate = self.report["candidateBinary"]

        command = measure.command_record(
            ["fixture"], candidate, [candidate, source], [],
        )

        self.assertEqual(
            [item["root"] for item in command["inputs"]],
            ["repository", "evidence"],
        )

    def test_historical_checker_receives_an_absolute_retained_report(self):
        evidence = pathlib.Path(self.temporary.name) / "relative-evidence"
        replay_root = evidence / "replay-v013"
        replay_root.mkdir(parents=True)
        manifest = replay_root / "v0_13_replay.toml"
        manifest.write_bytes(gate.V013_REPLAY_MANIFEST.read_bytes())
        checker = replay_root / "check-native-performance-v013.py"
        checker.write_bytes(b"pinned historical checker\n")
        historical_report = replay_root / "schema8/v0.13-results.json"
        historical_report.parent.mkdir()
        historical_report.write_text("{}\n", encoding="utf-8")
        commit = gate.tomllib.loads(manifest.read_text(encoding="utf-8"))["commit"]
        replay = {
            "commit": commit,
            "manifest": {"path": "replay-v013/v0_13_replay.toml"},
            "compiler": {"path": "replay-v013/ckc-v013"},
            "archive": {"path": "replay-v013/ckc-v013-distribution.tar.gz"},
            "schemaEight": {"path": "replay-v013/schema8/v0.13-results.json"},
            "checker": {"path": "replay-v013/check-native-performance-v013.py"},
            "evidenceFiles": [],
        }
        report = {"v013ReplayBundle": replay, "v013ReplayCommit": commit}
        calls = []
        environments = []

        def run(command, **kwargs):
            calls.append(command)
            environments.append(kwargs.get("env"))
            if command[:2] == ["git", "show"]:
                return Mock(returncode=0, stdout=checker.read_bytes(), stderr=b"")
            if command[:2] in (["git", "clone"], ["git", "checkout"]):
                return Mock(returncode=0, stdout="", stderr="")
            return Mock(returncode=1, stdout="historical sentinel", stderr="")

        relative_evidence = pathlib.Path(os.path.relpath(evidence, REPO))
        with patch.object(gate, "check_schema9_file"), \
                patch.object(gate, "schema9_check_tree"), \
                patch.object(gate, "schema9_check_v013_replay_receipt",
                             return_value=(replay_root / "adapter.patch", "a" * 64)), \
                patch.object(gate, "schema9_prepare_historical_measurement"), \
                patch.object(gate.subprocess, "run", side_effect=run):
            with self.assertRaisesRegex(ValueError, "historical sentinel"):
                gate.schema9_check_replay(report, relative_evidence)

        historical_command = calls[-1]
        self.assertTrue(
            pathlib.Path(historical_command[-1]).is_absolute(),
            "the detached historical checker must not resolve evidence relative to its checkout",
        )
        self.assertEqual(
            environments[-1]["GITHUB_SHA"],
            commit,
            "the detached historical checker must receive the historical checkout identity",
        )
        for environment_name, retained_name in [
            ("CKC_V012_RUNTIME_BUNDLE", "replay-v012"),
            ("CKC_V011_RUNTIME_BUNDLE", "replay-v011"),
            ("CKC_V010_RUNTIME_BUNDLE", "replay-v010"),
        ]:
            self.assertEqual(
                environments[-1].get(environment_name),
                str((historical_report.parent / retained_name).resolve()),
                "the historical checker must resolve its retained replay dependencies",
            )

    @unittest.skipUnless(os.name == "posix", "POSIX cache mode contract")
    def test_cache_snapshot_creates_an_owner_only_namespace(self):
        evidence = pathlib.Path(self.temporary.name) / "cache-evidence"
        evidence.mkdir()
        namespace = evidence / "cache/branch-layout/cold-one/ckc"

        measure.snapshot_cache(evidence, namespace)

        self.assertEqual(stat.S_IMODE(namespace.stat().st_mode), 0o700)

    def test_integer_product_thresholds_and_strict_domain_gate(self):
        self.assertTrue(gate.schema9_ratio_le([95, 95], [100, 100], 95, 100))
        self.assertFalse(gate.schema9_ratio_le([96, 95], [100, 100], 95, 100))
        self.assertTrue(gate.schema9_throughput_ge([100], [108], 108, 100))
        self.assertFalse(gate.schema9_throughput_ge([100], [108], 108, 100, strict=True))
        self.assertTrue(gate.schema9_throughput_ge([99], [108], 108, 100, strict=True))

    def test_steady_timing_uses_the_empty_environment_native_runner_protocol(self):
        call = inspect.getsource(measure.ExternalPerformanceKernel.call)
        timed = inspect.getsource(measure.timed_call)
        self.assertIn('"--ck-perf"', call)
        self.assertIn("env={}", call)
        self.assertNotIn("perf_counter", timed)
        self.assertNotIn("kernel.run", timed)

    def test_predicated_update_assets_and_runner_protocol_are_frozen(self):
        expected = {
            "benches/fixtures/tune/predicated-update-training.tsv":
                "ckc-predicated-inputs\t1\ttraining\n"
                "predicated-update\ttrain-floyd-128\t128\t113\n",
            "benches/fixtures/tune/predicated-update-validation.tsv":
                "ckc-predicated-inputs\t1\tvalidation\n"
                "predicated-update\tvalidate-floyd-256\t256\t127\n",
            "benches/fixtures/tune/predicated-update-release.tsv":
                "ckc-predicated-inputs\t1\trelease-held-out\n"
                "predicated-update\trelease-floyd-1024\t1024\t131\n",
        }
        for relative, contents in expected.items():
            self.assertEqual((REPO / relative).read_text(encoding="utf-8"), contents)
        manifest = (REPO / "benches/tune/workloads/predicated-update.cktune.toml").read_text(
            encoding="utf-8")
        self.assertIn('args = ["--ck-predicated-tune"]', manifest)
        self.assertNotIn("release-held-out", manifest)
        runner = (REPO / "benches/tune/runner.rs").read_text(encoding="utf-8")
        for protocol in [
            "--ck-predicated-tune", "--ck-predicated-profile",
            "--ck-predicated-oracle", "--ck-predicated-perf",
            "CLOCK_MONOTONIC_RAW", "unsafe extern \"C\" fn(*mut f64, u32, u32)",
        ]:
            self.assertIn(protocol, runner)
        self.assertNotIn("releaseMaximumNum", runner)
        self.assertNotIn("validationMaximumNum", runner)

    def test_oracle_builds_bind_the_explicit_retained_linker_chain(self):
        build = inspect.getsource(measure.build_oracle)
        self.assertIn('retained["systemLinkerOriginal"]', build)
        self.assertIn('f"--ld-path={system_linker}"', build)
        self.assertIn('f"linker={retained[\'clangOriginal\']}"', build)
        self.assertIn('retained["systemLinker"]', build)
        self.assertIn('retained["clang"]', build)
        self.assertIn("command_record(argv, compiler, inputs, [])", build)
        self.reject(lambda report: report["toolchain"].pop("systemLinker"), "missing")

    def test_generic_oracle_build_record_is_not_misparsed_as_a_ck_command(self):
        evidence = pathlib.Path(self.temporary.name) / self.report["evidenceDirectory"]
        executable = self.report["toolchain"]["clangBinary"]
        source = self.report["workload"]["cOracle"]
        output = measure.retained_marker(evidence, "contract/oracle.so", b"oracle\n")
        argv = [
            f"fixture/{executable['path']}", "-std=c11", source["path"], "-o",
            f"fixture/{output['path']}",
        ]
        build = measure.build_record(
            measure.command_record(argv, executable, [source], []),
            None,
            [{"role": "primary", "file": output}],
        )
        gate.schema9_check_build(build, evidence, "oracle", tuned=None)
        with self.assertRaisesRegex(ValueError, "--out|ordinary build"):
            gate.schema9_check_build(build, evidence, "oracle", tuned=False)

    def test_profile_inspection_uses_the_frozen_flat_compiler_source_field(self):
        fixture = json.loads(
            (REPO / "tests/fixtures/profile/inspection-schema1.json").read_text(encoding="utf-8")
        )
        self.assertEqual(
            measure.inspected_profile_compiler_source(fixture),
            "1" * 64,
        )
        fixture["identity"]["compiler"] = {"source": fixture["identity"].pop("compilerSource")}
        with self.assertRaisesRegex(ValueError, "compiler source identity"):
            measure.inspected_profile_compiler_source(fixture)

    def test_event_receipts_reject_reorder_count_and_digest_mutation(self):
        evidence = pathlib.Path(self.temporary.name) / self.report["evidenceDirectory"]
        plan = "a" * 64
        summary = {"planDigest": plan}
        event, _ = measure.derived_event_log(
            evidence, "mutation/events.tsv", summary,
            {"compiled": 2, "measured": 1}, False,
        )
        counts = gate.schema9_check_events(event, evidence, "events", plan, warm=False)
        self.assertEqual(counts["compile-attempt"], 2)
        path = evidence / event["path"]
        original = path.read_text(encoding="utf-8")
        path.write_text(original.replace("0\tcache-miss", "0\tpublication", 1), encoding="utf-8")
        mutated = measure.evidence_identity(evidence, event["path"])
        with self.assertRaisesRegex(ValueError, "publication|cold event"):
            gate.schema9_check_events(mutated, evidence, "events", plan, warm=False)

    def test_compile_receipts_bind_each_sample_to_its_command(self):
        root = REPO / "target"
        root.mkdir(exist_ok=True)
        temporary = tempfile.TemporaryDirectory(prefix="ckc-schema9-compile-", dir=root)
        self.addCleanup(temporary.cleanup)
        output = pathlib.Path(temporary.name) / "report.json"
        report = measure.contract_report(output)
        evidence = output.parent / report["evidenceDirectory"]
        compiler = report["candidateBinary"]
        table = gate.schema9_case_table()
        sources = report["workload"]["sources"]
        report["v013ReplayBundle"] = {"compiler": compiler}
        report["tuningDecisions"] = [
            {"case": case, "file": compiler} for case in sorted(gate.SCHEMA9_CASES)
        ]
        rows = []
        for case in sorted(gate.SCHEMA9_CASES):
            source = next(item for item in sources if item["path"] == table[case]["source"])
            commands = {channel: [] for channel in ["tuneUse", "v014Ordinary"]}
            samples = {channel: [] for channel in commands}
            for channel in commands:
                for index in range(18):
                    base = evidence / f"compile/{case}/{channel}-{index}/artifact"
                    output_arg = base.relative_to(REPO).as_posix()
                    argv = [
                        (evidence / compiler["path"]).relative_to(REPO).as_posix(),
                        "build", source["path"], "--out", output_arg, "--kind", "dynamic",
                        "--cpu", "native", "-O3", "--overflow", "unchecked", "--bounds",
                        "unchecked",
                    ]
                    inputs = [source]
                    if channel == "tuneUse":
                        argv += ["--tune-use", (evidence / compiler["path"]).relative_to(REPO).as_posix()]
                        inputs.append(compiler)
                    receipt = {
                        "command": {"argv": argv, "workingDirectory": "repository",
                                    "executable": compiler, "inputs": inputs,
                                    "environment": [{
                                        "name": "XDG_CACHE_HOME",
                                        "value": str(evidence / f"cache/{case}/{channel}-{index}"),
                                        "references": [],
                                    }], "environmentDigest": "0" * 64},
                        "elapsedNs": 100 + index,
                    }
                    commands[channel].append(receipt)
                    if index >= 3:
                        samples[channel].append(100 + index)
            orders = [["tuneUse", "v014Ordinary"] if index % 2 == 0
                      else ["v014Ordinary", "tuneUse"] for index in range(18)]
            rows.append({
                "case": case, "warmupOrder": orders[:3], "sampleOrder": orders[3:],
                "samplesNs": samples,
                "mediansNs": {channel: sorted(values)[7] for channel, values in samples.items()},
                "commands": commands,
            })
        with patch.object(gate, "schema9_check_command", lambda *args, **kwargs: "0" * 64), \
                patch.object(gate, "schema9_check_ck_environment", lambda *args, **kwargs: None):
            gate.schema9_check_compile_rows(rows, evidence, "compile", report,
                                            "tuneUse", "v014Ordinary")
            mutated = copy.deepcopy(rows)
            mutated[0]["commands"]["tuneUse"][3]["elapsedNs"] += 1
            with self.assertRaisesRegex(ValueError, "samples do not equal retained receipts"):
                gate.schema9_check_compile_rows(mutated, evidence, "compile", report,
                                                "tuneUse", "v014Ordinary")


if __name__ == "__main__":
    unittest.main()
