"""Read-only diagnostic routing tests; synthetic files are not performance evidence."""

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


REPO = Path(__file__).resolve().parents[2]
RESOLVER = REPO / "scripts/resolve-performance-diagnostic-report.py"


def record(path):
    data = path.read_bytes()
    return {"file": path.name, "bytes": len(data),
            "sha256": hashlib.sha256(data).hexdigest()}


def write_file(path, data=b"synthetic diagnostic-only bytes; never executable"):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    return record(path)


class HistoricalReportResolver(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="ckc-diagnostic-report-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.report_path = self.root / "schema8/v0.13-results.json"
        self.evidence = self.report_path.parent / "v013-measurement-123-456"
        self.cumulative = self.evidence / "results-schema7.json"
        cumulative_record = write_file(self.cumulative, b'{"schemaVersion":7}\n')
        self.report = {"schemaVersion": 8, "candidateVersion": "0.13.0",
                       "evidenceDirectory": self.evidence.name,
                       "cumulativeSchemaSeven": cumulative_record}
        self.save_report()

    def save_report(self):
        self.report_path.write_text(json.dumps(self.report), encoding="utf-8")

    def resolve(self):
        return subprocess.run(
            [sys.executable, "-B", str(RESOLVER), str(self.report_path)],
            text=True, capture_output=True, check=False,
        )

    def assert_rejected(self, message):
        result = self.resolve()
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn(message, result.stderr)
        self.assertEqual(result.stdout, "")

    def test_retained_report_resolves_without_successful_replay_manifest(self):
        before = {path: path.read_bytes() for path in [self.report_path, self.cumulative]}
        result = self.resolve()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), str(self.cumulative))
        self.assertFalse((self.root / "replay.tsv").exists())
        self.assertEqual(before, {path: path.read_bytes() for path in before})

    def test_changed_cumulative_bytes_are_rejected(self):
        self.cumulative.write_bytes(b"different but same selected path")
        self.assert_rejected("cumulative schema-7 identity mismatch")

    def test_same_size_changed_cumulative_bytes_are_rejected(self):
        original = self.cumulative.read_bytes()
        self.cumulative.write_bytes(original.replace(b"7", b"6"))
        self.assert_rejected("cumulative schema-7 identity mismatch")

    def test_missing_cumulative_report_is_rejected(self):
        self.cumulative.unlink()
        self.assert_rejected("cumulative schema-7 report must be a nonempty regular file")

    def test_missing_schema8_report_is_rejected(self):
        self.report_path.unlink()
        self.assert_rejected("historical schema-8 report must be a nonempty regular file")

    def test_evidence_directory_escape_is_rejected(self):
        for value in ["../v013-measurement-123-456", "/tmp/evidence", "v013-measurement-1-2/child", None]:
            with self.subTest(value=value):
                self.report["evidenceDirectory"] = value
                self.save_report()
                self.assert_rejected("unsafe historical evidence directory")

    def test_cumulative_basename_is_fixed(self):
        for value in ["../results-schema7.json", "other.json", "results-schema7.json\n", None]:
            with self.subTest(value=value):
                self.report["cumulativeSchemaSeven"]["file"] = value
                self.save_report()
                self.assert_rejected("invalid cumulative schema-7 record")

    def test_cumulative_record_types_and_keys_are_closed(self):
        original = dict(self.report["cumulativeSchemaSeven"])
        for update in [{"bytes": True}, {"bytes": 0}, {"bytes": "22"},
                       {"sha256": "not-a-hash"}, {"extra": "ignored?"}]:
            with self.subTest(update=update):
                self.report["cumulativeSchemaSeven"] = {**original, **update}
                self.save_report()
                self.assert_rejected("invalid cumulative schema-7 record")

    def test_missing_cumulative_record_is_rejected(self):
        del self.report["cumulativeSchemaSeven"]
        self.save_report()
        self.assert_rejected("invalid cumulative schema-7 record")

    def test_non_schema8_report_is_rejected(self):
        self.report["schemaVersion"] = 9
        self.save_report()
        self.assert_rejected("historical diagnostic requires schema 8")

    def test_symlinked_cumulative_report_is_rejected(self):
        retained = self.root / "retained.json"
        self.cumulative.rename(retained)
        self.cumulative.symlink_to(retained)
        self.assert_rejected("cumulative schema-7 report must be a nonempty regular file")

    def test_symlinked_evidence_directory_is_rejected(self):
        retained = self.root / "retained-evidence"
        self.evidence.rename(retained)
        self.evidence.symlink_to(retained, target_is_directory=True)
        self.assert_rejected("historical evidence directory must be a real directory")

    def test_symlinked_schema8_report_is_rejected(self):
        retained = self.root / "retained-schema8.json"
        self.report_path.rename(retained)
        self.report_path.symlink_to(retained)
        self.assert_rejected("historical schema-8 report must be a nonempty regular file")

    def test_symlinked_schema8_directory_is_rejected(self):
        retained = self.root / "retained-schema8"
        self.report_path.parent.rename(retained)
        self.report_path.parent.symlink_to(retained, target_is_directory=True)
        self.assert_rejected("historical schema-8 directory must be a real directory")

    def test_malformed_json_is_reported_without_traceback(self):
        self.report_path.write_text("{broken", encoding="utf-8")
        self.assert_rejected("performance diagnostic report resolution failed:")
        self.assertNotIn("Traceback", self.resolve().stderr)


class DiagnosticStageRouting(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="ckc-diagnostic-stage-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        subprocess.run(["git", "init", "-q", self.root], check=True, capture_output=True)
        subprocess.run(
            ["git", "-c", "user.name=Diagnostic test", "-c", "user.email=diagnostic@example.invalid",
             "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null",
             "commit", "-qm", "diagnostic fixture", "--allow-empty"],
            cwd=self.root, check=True, capture_output=True,
        )
        scripts = self.root / "scripts"
        scripts.mkdir()
        self.script = scripts / "diagnose-native-performance.sh"
        shutil.copyfile(REPO / "scripts/diagnose-native-performance.sh", self.script)
        if RESOLVER.is_file():
            shutil.copyfile(RESOLVER, scripts / RESOLVER.name)
        self.bundle = self.root / "historical-v013"
        self.schema8 = self.bundle / "schema8"
        self.evidence = self.schema8 / "v013-measurement-123-456"
        candidate = write_file(self.evidence / "proof_loop-unchecked-native.so")
        candidate.update({"case": "proof_loop", "mode": "unchecked", "channel": "candidateNative"})
        replay_record = {"case": "proof_loop", "mode": "unchecked",
                         **record(self.evidence / "proof_loop-unchecked-native.so")}
        replay_record["file"] = "proof_loop-unchecked.so"
        cumulative = {"schemaVersion": 7, "evidenceDirectory": "measurement-123-456",
                      "measuredArtifacts": [candidate],
                      "runtimeReplayV011": {"artifacts": [replay_record]},
                      "runtimeReplayV010": {"artifacts": [replay_record]}}
        measured = self.evidence / cumulative["evidenceDirectory"]
        write_file(measured / candidate["file"])
        schema7_record = write_file(self.evidence / "results-schema7.json", json.dumps(cumulative).encode())
        compiler_record = write_file(self.evidence / "candidate-ckc")
        archive_record = write_file(self.evidence / "candidate.tar.gz")
        report = {"schemaVersion": 8, "candidateVersion": "0.13.0",
                  "evidenceDirectory": self.evidence.name, "cumulativeSchemaSeven": schema7_record,
                  "candidateBinary": compiler_record, "trainingShards": [], "finalProfiles": [],
                  "variantObjects": [], "archiveSize": {"candidateFile": archive_record["file"],
                  "candidateBytes": archive_record["bytes"], "candidateSha256": archive_record["sha256"]}}
        write_file(self.schema8 / "v0.13-results.json", json.dumps(report).encode())
        self.environment = {**os.environ, "CKC_V013_RUNTIME_BUNDLE": str(self.bundle)}
        for version in ["012", "011", "010"]:
            historical = self.schema8 / f"replay-v{version}"
            for name in ["preparation.log", "replay.tsv", f"ckc-v{version}", "proof_loop-unchecked.so"]:
                write_file(historical / name)
            write_file(historical / f"ckc-v{version}-distribution.tar.gz")
            # Candidate bundles exist, so a missing environment variable cannot be the failure.
            decoy = self.root / f"candidate-replay-v{version}"
            shutil.copytree(historical, decoy)
            self.environment[f"CKC_V{version}_RUNTIME_BUNDLE"] = str(decoy)
        prefix = self.root / "disassembler-only"
        objdump = prefix / "bin/llvm-objdump"
        write_file(objdump, b'#!/bin/sh\nprintf "disassembler-input: %s\\n" "$@"\n')
        objdump.chmod(0o755)
        self.environment["CKC_LLVM_PREFIX"] = str(prefix)

    def run_diagnostics(self, stage):
        arguments = [] if stage is None else [stage]
        return subprocess.run(["bash", str(self.script), *arguments], cwd=self.root,
                              env=self.environment, text=True, capture_output=True, check=False)

    def prepare_current_report(self):
        current = self.root / "target/ckc-perf"
        current.mkdir(parents=True)
        shutil.copytree(self.evidence, current / self.evidence.name)
        shutil.copytree(self.evidence / "measurement-123-456", current / "measurement-123-456")
        shutil.copyfile(self.evidence / "results-schema7.json", current / "results.json")
        shutil.copyfile(self.schema8 / "v0.13-results.json", current / "v0.13-results.json")
        return current

    def test_default_candidate_stage_preserves_existing_report_and_bundle_routing(self):
        current = self.prepare_current_report()
        result = self.run_diagnostics(None)
        self.assertEqual(result.returncode, 0, result.stderr)
        output = self.root / "target/performance-diagnostics"
        libraries = (output / "measured-libraries.tsv").read_text()
        self.assertIn(str(current / "measurement-123-456"), libraries)
        self.assertIn("candidate-replay-v011", libraries)
        self.assertNotIn(str(self.bundle), libraries)
        self.assertEqual((output / "diagnostic-stage.txt").read_text(), "candidate\n")

    def test_explicit_candidate_stage_does_not_require_historical_evidence(self):
        self.prepare_current_report()
        (self.schema8 / "v0.13-results.json").unlink()
        del self.environment["CKC_V013_RUNTIME_BUNDLE"]
        result = self.run_diagnostics("candidate")
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_historical_failure_uses_retained_reports_and_bundles(self):
        result = self.run_diagnostics("historical-v013")
        self.assertEqual(result.returncode, 0, result.stderr)
        output = self.root / "target/performance-diagnostics"
        libraries = (output / "measured-libraries.tsv").read_text()
        self.assertIn(str(self.evidence / "measurement-123-456"), libraries)
        self.assertIn(str(self.schema8 / "replay-v011"), libraries)
        self.assertIn(str(self.schema8 / "replay-v010"), libraries)
        self.assertNotIn("candidate-replay", libraries)
        self.assertIn("historical-v013", (output / "diagnostic-stage.txt").read_text())
        self.assertEqual(len((output / "schema8-files.tsv").read_text().splitlines()), 3)
        self.assertFalse((self.bundle / "replay.tsv").exists())
        self.assertFalse((self.root / "target/ckc-perf/results.json").exists())

    def test_historical_stage_never_falls_back_to_a_current_report(self):
        write_file(self.root / "target/ckc-perf/results.json", b'{"wrong":"cohort"}')
        result = self.run_diagnostics("historical-v013")
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_missing_historical_report_keeps_metadata_and_fails(self):
        (self.schema8 / "v0.13-results.json").unlink()
        result = self.run_diagnostics("historical-v013")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("historical schema-8 report must be a nonempty regular file", result.stderr)
        self.assertTrue((self.root / "target/performance-diagnostics/host.txt").is_file())
        self.assertFalse((self.bundle / "replay.tsv").exists())

    def test_unknown_stage_is_rejected(self):
        result = self.run_diagnostics("unknown-stage")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unknown diagnostic stage", result.stderr)


if __name__ == "__main__":
    unittest.main()
