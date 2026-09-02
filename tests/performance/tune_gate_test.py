"""Schema-9 contract mutation tests; contract fixtures are never performance evidence."""

from __future__ import annotations

import copy
import importlib.util
import inspect
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
