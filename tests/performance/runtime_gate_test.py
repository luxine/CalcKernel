"""Schema-6 acceptance tests; synthetic artifact bytes are never loaded/executed."""

import copy
import contextlib
import hashlib
import io
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import tomllib
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
REPO = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("gate", REPO / "scripts/check-native-performance.py")
gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gate)
BASELINE_PATH = REPO / "benches/baselines/v0_10_compiler.toml"
BASELINE = tomllib.loads(BASELINE_PATH.read_text())
CASES = ["branch_mix", "integer_accumulate", "proof_loop", "remainder_chain"]
RECIPE = ["scripts/prepare-performance-replay.py", "benches/runtime_replay.rs", "benches/ckc_perf.rs"]
ADAPTERS = [f"benches/baselines/v0_10_{name}_harness.patch" for name in
            ["linux_cpp_runtime", "clang_cpu", "mir_optimizer", "proof_loop"]]
CHANNELS = [f"{kind}{mode}" for kind in
            ["candidateNative", "currentClang", "replayNative", "replayClang"]
            for mode in ["Unchecked", "Checked"]]


def digest(data):
    return hashlib.sha256(data).hexdigest()


def named_digest(paths):
    return digest(b"".join(name.encode() + b"\0" + digest((REPO / name).read_bytes()).encode()
                           + b"\n" for name in sorted(paths)))


def order(count):
    return [[(r + offset) % 8 for offset in range(8)] for r in range(count)]


class GateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="ckc-replay-gate-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.bundle = self.root / "bundle"
        self.bundle.mkdir()
        self.evidence = self.root / "measurement-1-2"
        self.evidence.mkdir()
        prefix = self.root / "prefix"
        component = prefix / "share/ckc/llvm-build.toml"
        component.parent.mkdir(parents=True)
        component.write_bytes(b"synthetic component manifest, never used for compilation\n")
        compiler = b"synthetic independently prepared 0.10 compiler, never executed"
        (self.bundle / "ckc-v010").write_bytes(compiler)
        metadata = {
            "commit": BASELINE["commit"], "compilerIdentity": BASELINE["compiler_identity"],
            "compilerSha256": digest(compiler), "compilerBytes": str(len(compiler)),
            "llvmVersion": "22.1.8", "target": "linux-x86_64", "cpuPolicy": "baseline",
            "recipeSha256": named_digest(RECIPE), "adapterSetSha256": named_digest(ADAPTERS),
            "sourceDiffSha256": "a" * 64, "baselineManifestSha256": digest(BASELINE_PATH.read_bytes()),
            "llvmComponentSha256": digest(component.read_bytes()),
        }
        artifacts = []
        measured = []
        suites = []
        for mode in ["unchecked", "checked"]:
            cases = []
            for name in CASES:
                filename = f"{name}-{mode}.so"
                data = f"synthetic baseline library {name}/{mode}".encode()
                (self.bundle / filename).write_bytes(data)
                artifacts.append(dict(case=name, mode=mode, file=filename, bytes=len(data), sha256=digest(data)))
                sizes = {}
                for channel, suffix in [("candidateNative", "native"), ("currentClang", "clang"),
                                        ("replayClang", "replay-clang")]:
                    file = f"{name}-{mode}-{suffix}.so"
                    data = f"synthetic {channel} {name}/{mode}".encode()
                    (self.evidence / file).write_bytes(data)
                    measured.append(dict(case=name, mode=mode, channel=channel, file=file,
                                         bytes=len(data), sha256=digest(data)))
                    sizes[channel] = len(data)
                historical = next(row for row in BASELINE["runtime"] if row["target"] == "linux-x86_64"
                                  and row["mode"] == mode and row["case"] == name)
                case = dict(name=name, referenceEquivalent=True, proofLoop=name == "proof_loop",
                            v010MedianNs=historical["median_ns"], v010ClangMedianNs=historical["clang_median_ns"],
                            nativeCompileNs=100, clangCCompileNs=100, nativeColdNs=100, clangCColdNs=100,
                            peakMemoryBytes=1024, nativeArtifactBytes=sizes["candidateNative"],
                            clangCArtifactBytes=sizes["currentClang"], batchIterations=20_000_000, result=17,
                            warmupOrder=order(3), sampleOrder=order(20))
                for stream in ["native", "clangC", "replayNative", "replayClang"]:
                    self.set_stream(case, stream, 100)
                cases.append(case)
            suites.append(dict(mode=mode, cases=cases))
        lines = ["ckc-v010-runtime-replay\t1"]
        lines += [f"{key}\t{value}" for key, value in metadata.items()]
        lines += ["\t".join(["artifact", item["mode"], item["case"], item["file"], str(item["bytes"]), item["sha256"]])
                  for item in artifacts]
        manifest = "\n".join(lines) + "\n"
        (self.bundle / "replay.tsv").write_text(manifest)
        self.report = dict(schemaVersion=6, candidateVersion="0.11.0", cpuPolicy="baseline", fastMath=False,
                           clangVersion="22.1.8", warmup=3, sampleRepetitions=7,
                           samplingProtocol="rotating-eight-channel-v1", channelNames=CHANNELS,
                           runtimeReplay=dict(metadata=metadata, manifestSha256=digest(manifest.encode()), artifacts=artifacts),
                           evidenceDirectory=self.evidence.name, measuredArtifacts=measured, suites=suites,
                           baselineV010=dict(commit=BASELINE["commit"], compilerIdentity=BASELINE["compiler_identity"],
                                             llvmVersion=BASELINE["llvm_version"], target="linux-x86_64",
                                             harness=BASELINE["harness"], statistics=BASELINE["statistics"],
                                             sourceDigestCount=17, sourceDigests={key.removeprefix("source_digest_"): value
                                                 for key, value in BASELINE.items() if key.startswith("source_digest_")}),
                           optimizerComparisons=[dict(case=row["case"], kirMedianNs=row["median_ns"],
                                                      v010MirMedianNs=row["median_ns"])
                                                 for row in BASELINE["optimizer"] if row["target"] == "linux-x86_64"])
        self.environment = patch.dict(os.environ, CKC_V010_RUNTIME_BUNDLE=str(self.bundle), CKC_LLVM_PREFIX=str(prefix))
        self.environment.start()
        self.addCleanup(self.environment.stop)
        # Synthetic reports model the frozen x86 worker on every test host.
        self.host = patch.object(gate, "host_target_name", return_value="linux-x86_64")
        self.host.start()
        self.addCleanup(self.host.stop)

    @staticmethod
    def set_stream(case, stream, value):
        case[stream + "MedianNs"] = value
        case[stream + "SamplesNs"] = [value] * 20

    def check(self, report=None):
        path = self.root / "results.json"
        path.write_text(json.dumps(self.report if report is None else report))
        with contextlib.redirect_stdout(io.StringIO()):
            gate.check(path, BASELINE_PATH)

    def reject(self, mutate, message):
        report = copy.deepcopy(self.report)
        mutate(report)
        with self.assertRaisesRegex(ValueError, message):
            self.check(report)

    def test_complete_schema_six_passes(self):
        self.check()

    def test_i14_real_cross_worker_counterexample_uses_actual_replay(self):
        case = self.report["suites"][0]["cases"][1]
        self.assertEqual((case["v010MedianNs"], case["v010ClangMedianNs"]), (23000767, 27975716))
        for stream, value in [("native", 14671625), ("clangC", 14671807),
                              ("replayNative", 14665704), ("replayClang", 14666416)]:
            self.set_stream(case, stream, value)
        self.check()

    def test_common_mode_slowdown_is_calibrated(self):
        for stream in ["native", "clangC"]:
            self.set_stream(self.report["suites"][0]["cases"][0], stream, 120)
        self.check()

    def test_individual_and_geometric_gates_are_independent(self):
        def set_cases(report, streams, value, all_cases):
            for suite in report["suites"]:
                for case in (suite["cases"] if all_cases else suite["cases"][:1]):
                    for stream in streams:
                        self.set_stream(case, stream, value)
        for streams, value, all_cases, message in [
            (["native"], 111, False, "10%"),
            (["native", "replayNative"], 106, True, "95%"),
            (["native", "clangC", "replayClang"], 109, False, "8%"),
            (["native", "clangC", "replayClang"], 104, True, "3%"),
        ]:
            with self.subTest(message=message):
                self.reject(lambda r: set_cases(r, streams, value, all_cases), message)
        self.reject(lambda r: self.set_stream(r["suites"][1]["cases"][2], "native", 104), "97%")
        def optimizer(report, ratios):
            for row, ratio in zip(report["optimizerComparisons"], ratios):
                row["kirMedianNs"] = row["v010MirMedianNs"] * ratio
        self.reject(lambda r: optimizer(r, [3.1, 1, 1, 1, 1, 1]), "3x")
        self.reject(lambda r: optimizer(r, [.1, .1, 2.1, 2.1, 2.1, 2.1]), "2x")

    def test_sample_medians_stability_finiteness_and_batch_are_strict(self):
        for stream in ["native", "clangC", "replayNative", "replayClang"]:
            for field, value, message in [
                (stream + "MedianNs", 1, "sample array"),
                (stream + "SamplesNs", [100] * 19, "20 samples"),
                (stream + "SamplesNs", [100] * 15 + [180] * 5, "unstable"),
                (stream + "MedianNs", float("nan"), "finite"),
                (stream + "SamplesNs", [float("inf")] * 20, "finite"),
            ]:
                with self.subTest(stream=stream, field=field, value=str(value)):
                    self.reject(lambda r: r["suites"][0]["cases"][0].__setitem__(field, value), message)
        for field, value, message in [
            ("batchIterations", 200_000, "20000000"), ("referenceEquivalent", False, "equivalence"),
            ("v010MedianNs", 1, "frozen manifest"), ("v010ClangMedianNs", 1, "frozen manifest"),
            ("sampleOrder", order(19), "order"), ("warmupOrder", order(2), "order"),
        ]:
            with self.subTest(field=field):
                self.reject(lambda r: r["suites"][0]["cases"][0].__setitem__(field, value), message)
        self.reject(lambda r: r["suites"][0]["cases"][0]["sampleOrder"][1].reverse(), "order")

    def test_identity_corpus_and_missing_evidence_are_strict(self):
        for field, value, message in [
            ("schemaVersion", 5, "schemaVersion"), ("fastMath", True, "fast-math"),
            ("cpuPolicy", "native", "baseline"), ("clangVersion", "23.0.0", "Clang"),
            ("warmup", 2, "warmup"), ("sampleRepetitions", 3, "sampleRepetitions"),
            ("samplingProtocol", "unbalanced", "protocol"), ("channelNames", CHANNELS[::-1], "channel"),
            ("candidateVersion", "0.10.0", "candidate"), ("runtimeReplay", None, "replay"),
            ("measuredArtifacts", [], "artifact"), ("evidenceDirectory", "../escape", "evidence"),
        ]:
            with self.subTest(field=field):
                self.reject(lambda r: r.__setitem__(field, value), message)
        self.reject(lambda r: r["baselineV010"].__setitem__("compilerIdentity", "forged"), "identity")
        self.reject(lambda r: r["baselineV010"]["sourceDigests"].__setitem__("branch_mix", "f" * 64), "corpus")
        self.reject(lambda r: r["suites"][0]["cases"].pop(), "identical kernels")
        self.reject(lambda r: r["suites"][0]["cases"].append(r["suites"][0]["cases"][0]), "duplicate")
        self.reject(lambda r: r["optimizerComparisons"].pop(), "corpus")
        self.reject(lambda r: r["optimizerComparisons"][0].__setitem__("v010MirMedianNs", 1), "frozen manifest")
        self.reject(lambda r: r["runtimeReplay"]["metadata"].__setitem__("compilerIdentity", "calckernel 0.11.0"), "replay")
        self.reject(lambda r: r["runtimeReplay"].__setitem__("manifestSha256", "a" * 64), "replay")
        self.reject(lambda r: r["measuredArtifacts"][0].__setitem__("sha256", "b" * 64), "artifact")
        self.reject(lambda r: r["measuredArtifacts"].append(r["measuredArtifacts"][0]), "artifact")
        self.reject(lambda r: r["suites"][1]["cases"][0].__setitem__("result", 18), "results")
        self.reject(lambda r: r["suites"][0]["cases"][0].__setitem__("proofLoop", True), "proof-loop corpus")
        self.reject(lambda r: r["suites"].pop(), "separately")
        self.reject(lambda r: r["runtimeReplay"]["artifacts"].pop(), "eight artifact")
        self.reject(lambda r: r["runtimeReplay"]["artifacts"][0].__setitem__("sha256", "F" * 64), "artifact")
        self.reject(lambda r: r["measuredArtifacts"][0].__setitem__("file", "../elsewhere"), "artifact")

    def test_changed_bundle_identity_cannot_be_laundered_through_report_metadata(self):
        path = self.bundle / "replay.tsv"
        original = path.read_text()
        for field, replacement in [
            ("commit", "f" * 40), ("compilerIdentity", "calckernel 0.11.0"),
            ("llvmVersion", "23.0.0"), ("target", "linux-aarch64"), ("cpuPolicy", "native"),
            ("recipeSha256", "b" * 64), ("adapterSetSha256", "b" * 64),
            ("llvmComponentSha256", "b" * 64), ("baselineManifestSha256", "b" * 64),
        ]:
            with self.subTest(field=field):
                report = copy.deepcopy(self.report)
                metadata = report["runtimeReplay"]["metadata"]
                changed = original.replace(f"{field}\t{metadata[field]}\n", f"{field}\t{replacement}\n")
                self.assertNotEqual(original, changed)
                path.write_text(changed)
                metadata[field] = replacement
                report["runtimeReplay"]["manifestSha256"] = digest(changed.encode())
                try:
                    with self.assertRaisesRegex(ValueError, "replay"):
                        self.check(report)
                finally:
                    path.write_text(original)
        for extra in ["commit\tduplicate\n", "unknown\tfield\n", original.splitlines()[-1] + "\n"]:
            changed = original + extra
            report = copy.deepcopy(self.report)
            report["runtimeReplay"]["manifestSha256"] = digest(changed.encode())
            path.write_text(changed)
            try:
                with self.assertRaisesRegex(ValueError, "replay"):
                    self.check(report)
            finally:
                path.write_text(original)

    def test_frozen_manifest_bytes_must_not_change(self):
        path = self.root / "changed-baseline.toml"
        path.write_bytes(BASELINE_PATH.read_bytes() + b"\n# not the accepted baseline\n")
        report_path = self.root / "results.json"
        report_path.write_text(json.dumps(self.report))
        with self.assertRaisesRegex(ValueError, "frozen manifest SHA-256"):
            gate.check(report_path, path)

    def test_modified_missing_or_symlinked_files_and_duplicate_json_keys_fail(self):
        for path in [self.bundle / "ckc-v010", self.bundle / "branch_mix-unchecked.so",
                     self.evidence / "branch_mix-unchecked-native.so"]:
            original = path.read_bytes()
            with self.subTest(path=path):
                path.write_bytes(original + b"modified")
                with self.assertRaisesRegex(ValueError, "(artifact|replay).*(size|hash|SHA|mismatch)"):
                    self.check()
                path.unlink()
                with self.assertRaises((ValueError, OSError)):
                    self.check()
                path.write_bytes(original)
                if os.name != "nt":
                    saved = path.with_name(path.name + ".saved")
                    path.rename(saved)
                    path.symlink_to(saved)
                    try:
                        with self.assertRaisesRegex(ValueError, "(artifact|replay)"):
                            self.check()
                    finally:
                        path.unlink()
                        saved.rename(path)
        path = self.root / "results.json"
        path.write_text(json.dumps(self.report).replace('"schemaVersion": 6', '"schemaVersion": 6, "schemaVersion": 6'))
        with self.assertRaisesRegex(ValueError, "duplicate"):
            gate.check(path, BASELINE_PATH)
        with patch.dict(os.environ, CKC_V010_RUNTIME_BUNDLE=""):
            with self.assertRaisesRegex(ValueError, "CKC_V010_RUNTIME_BUNDLE"):
                self.check()


if __name__ == "__main__":
    unittest.main()
