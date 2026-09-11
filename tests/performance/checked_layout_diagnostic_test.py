"""Contracts for the separate, bounded AArch64 checked-kernel comparison."""

import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/diagnose-checked-aarch64.py"


class CheckedLayoutDiagnosticTests(unittest.TestCase):
    def setUp(self):
        self.assertTrue(SCRIPT.is_file(), "bounded checked-layout diagnostic is missing")
        spec = importlib.util.spec_from_file_location("checked_layout_diagnostic", SCRIPT)
        self.diagnostic = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.diagnostic)

    def fixture(self, root):
        directory = root / "measurement-123-456"
        directory.mkdir()
        artifacts = []
        for channel in ("candidate", "cSimd", "rustSimd"):
            name = f"vector-specialized_length-checked-{channel}.so"
            data = channel.encode()
            (directory / name).write_bytes(data)
            artifacts.append({"suite": "vector", "case": "specialized_length",
                              "mode": "checked", "channel": channel, "file": name,
                              "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()})
        report = {"schemaVersion": 7, "evidenceDirectory": directory.name,
                  "oracleArtifacts": artifacts}
        path = root / "results.json"
        path.write_text(json.dumps(report))
        return path, report

    def test_resolves_only_exact_three_report_libraries(self):
        with tempfile.TemporaryDirectory() as temporary:
            path, _ = self.fixture(Path(temporary))
            records = self.diagnostic.resolve_libraries(path)
            self.assertEqual([record["channel"] for record in records],
                             ["candidate", "cSimd", "rustSimd"])

    def test_rejects_tampered_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            path, report = self.fixture(Path(temporary))
            artifact = path.parent / report["evidenceDirectory"] / report["oracleArtifacts"][0]["file"]
            artifact.write_bytes(b"tampered")
            with self.assertRaisesRegex(ValueError, "identity"):
                self.diagnostic.resolve_libraries(path)

    def test_rejects_duplicate_report_keys(self):
        with tempfile.TemporaryDirectory() as temporary:
            path, _ = self.fixture(Path(temporary))
            path.write_text(path.read_text().replace('"schemaVersion": 7',
                                                    '"schemaVersion": 7, "schemaVersion": 7'))
            with self.assertRaisesRegex(ValueError, "duplicate"):
                self.diagnostic.resolve_libraries(path)

    def test_rejects_escaping_or_linked_evidence(self):
        for change in ("directory", "file", "symlink", "duplicate", "missing"):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as temporary:
                path, report = self.fixture(Path(temporary))
                if change == "directory":
                    report["evidenceDirectory"] = "../measurement-123-456"
                elif change == "file":
                    report["oracleArtifacts"][0]["file"] = "../candidate.so"
                elif change == "duplicate":
                    report["oracleArtifacts"].append(report["oracleArtifacts"][0])
                elif change == "missing":
                    report["oracleArtifacts"].pop()
                else:
                    artifact = path.parent / report["evidenceDirectory"] / report["oracleArtifacts"][0]["file"]
                    real = artifact.with_suffix(".real")
                    artifact.rename(real)
                    artifact.symlink_to(real)
                path.write_text(json.dumps(report))
                with self.assertRaises(ValueError):
                    self.diagnostic.resolve_libraries(path)

    def rows(self):
        rows = []
        sequence = 0
        input_digest, output_digest = self.diagnostic.expected_digests()
        for warmup, rounds, repetitions in ((True, 3, 1), (False, 20, 7)):
            for round_index in range(rounds):
                for repetition in range(repetitions):
                    for layout_offset in range(4):
                        layout = (round_index + repetition + layout_offset) % 4
                        for channel_offset in range(3):
                            channel = (round_index + repetition + channel_offset) % 3
                            rows.append({"type": "batch", "sequence": sequence,
                                         "warmup": warmup, "round": round_index,
                                         "repetition": repetition, "layout": layout,
                                         "channel": channel, "calls": 5000, "elements": 20000000,
                                         "threadCpuNs": 1000 + layout, "wallNs": 1100 + layout,
                                         "cpuBefore": 1, "cpuAfter": 1,
                                         "inputDigest": input_digest,
                                         "outputDigest": output_digest,
                                         "pmu": None})
                            sequence += 1
        return rows

    def test_requires_every_unmodified_raw_row_in_order(self):
        rows = self.rows()
        summary = self.diagnostic.analyze_rows(rows)
        self.assertEqual(summary["rawRows"], 1716)
        self.assertFalse(summary["acceptance"])

    def test_rejects_missing_duplicate_reordered_or_changed_work(self):
        original = self.rows()
        for mutation in ("missing", "duplicate", "reordered", "work", "digest", "cpu"):
            rows = copy.deepcopy(original)
            if mutation == "missing":
                rows.pop()
            elif mutation == "duplicate":
                rows.append(rows[-1])
            elif mutation == "reordered":
                rows[40], rows[41] = rows[41], rows[40]
            elif mutation == "work":
                rows[40]["calls"] = 4999
            elif mutation == "digest":
                rows[40]["outputDigest"] = "0000000000000000"
            else:
                rows[40]["cpuAfter"] = 2
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                self.diagnostic.analyze_rows(rows)

    def test_preserves_unavailable_pmu_instead_of_zero_counters(self):
        summary = self.diagnostic.analyze_rows(self.rows())
        self.assertEqual(summary["pmuAvailableRows"], 0)
        self.assertIsNone(summary["layouts"][0]["channels"][0]["cyclesMedian"])

    def test_compiler_uses_pinned_oracle_not_the_clang_free_release_prefix(self):
        self.assertTrue(hasattr(self.diagnostic, "compiler_path"),
                        "comparison must resolve the distinct pinned oracle compiler")
        self.assertEqual(self.diagnostic.compiler_path({"CKC_LLVM_PREFIX": "/release",
                                                       "CKC_CLANG_ORACLE": "/oracle/bin/clang"}),
                         "/oracle/bin/clang")
        self.assertEqual(self.diagnostic.compiler_path({"CKC_LLVM_PREFIX": "/release"}), "cc")

    def test_rejects_zero_and_multiplexed_counters(self):
        for mutation in ("zero", "multiplexed"):
            rows = self.rows()
            rows[40]["pmu"] = {"cycles": 100, "instructions": 100, "branchMisses": 0,
                               "enabled": 1000, "running": 1000}
            rows[40]["pmu"]["cycles" if mutation == "zero" else "running"] = 0
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                self.diagnostic.analyze_rows(rows)

    def test_comparison_runs_only_after_the_unmodified_original_gate(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        step = "      - name: Compare checked AArch64 code layouts separately"
        self.assertIn(step, workflow)
        comparison = workflow.split(step, 1)[1].split("      - name:", 1)[0]
        self.assertIn("if: always() && matrix.arch == 'AArch64'", comparison)
        self.assertIn("timeout-minutes: 5", comparison)
        self.assertIn("scripts/diagnose-checked-aarch64.py", comparison)
        self.assertGreater(workflow.index(step), workflow.index("checker-schema8.log"))
        self.assertLess(workflow.index(step), workflow.index("      - name: Upload performance evidence"))
        self.assertNotIn("continue-on-error", comparison)
        self.assertNotIn("check-native-performance.py", comparison)


if __name__ == "__main__":
    unittest.main()
