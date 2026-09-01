"""Schema-7 acceptance tests; synthetic artifact bytes are never executed."""

import contextlib
import copy
import hashlib
import importlib.util
import io
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
SPEC = importlib.util.spec_from_file_location(
    "gate", REPO / "scripts/check-native-performance.py"
)
gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gate)
V010_PATH = REPO / "benches/baselines/v0_10_compiler.toml"
V010 = tomllib.loads(V010_PATH.read_text())
V011_PATH = REPO / "benches/baselines/v0_11_compiler.toml"
V011 = tomllib.loads(V011_PATH.read_text())
SCALAR_CASES = ["branch_mix", "integer_accumulate", "proof_loop", "remainder_chain"]
VECTOR_CASES = [
    "map_u32", "zip_u32", "strict_f64", "integer_cast", "modular_reduction",
    "slp_quad", "runtime_noalias", "specialized_length",
]
DOMAIN_CASES = ["contract_noalias", "contract_fixed_length"]
CHANNELS = [
    f"{kind}{mode}"
    for kind in [
        "candidateNative", "currentClang", "replayV011Native",
        "replayV011Clang", "replayV010Native", "replayV010Clang",
    ]
    for mode in ["Unchecked", "Checked"]
]


def digest(data):
    return hashlib.sha256(data).hexdigest()


def named_digest(paths):
    return digest(b"".join(
        name.encode() + b"\0" + digest((REPO / name).read_bytes()).encode() + b"\n"
        for name in sorted(paths)
    ))


def order(width, count):
    return [[(row + offset) % width for offset in range(width)] for row in range(count)]


def stream(record, prefix, value=100, count=20):
    record[prefix + "MedianNs"] = value
    record[prefix + "SamplesNs"] = [value] * count


class GateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="ckc-schema7-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.prefix = self.root / "prefix"
        component = self.prefix / "share/ckc/llvm-build.toml"
        component.parent.mkdir(parents=True)
        component.write_bytes(b"synthetic pinned component manifest\n")
        self.evidence = self.root / "measurement-1-2"
        self.evidence.mkdir()
        self.bundles = {}
        replays = {}
        for generation, baseline, manifest_path in [
            ("v011", V011, V011_PATH), ("v010", V010, V010_PATH)
        ]:
            bundle = self.root / generation
            bundle.mkdir()
            compiler_name = f"ckc-{generation}"
            compiler = f"synthetic {generation} compiler".encode()
            (bundle / compiler_name).write_bytes(compiler)
            metadata = {
                "commit": baseline["commit"],
                "compilerIdentity": baseline["compiler_identity"],
                "compilerSha256": digest(compiler),
                "compilerBytes": str(len(compiler)),
                "llvmVersion": "22.1.8",
                "target": "linux-x86_64",
                "cpuPolicy": "baseline",
                "recipeSha256": named_digest([
                    "scripts/prepare-performance-replay.py",
                    "scripts/audit-performance-oracles.py",
                    "benches/runtime_replay.rs", "benches/ckc_perf.rs",
                    "benches/vector_perf.rs", "benches/oracles/manifest.toml",
                ]),
                "adapterSetSha256": (
                    named_digest([
                        "benches/baselines/v0_10_linux_cpp_runtime_harness.patch",
                        "benches/baselines/v0_10_clang_cpu_harness.patch",
                        "benches/baselines/v0_10_mir_optimizer_harness.patch",
                        "benches/baselines/v0_10_proof_loop_harness.patch",
                    ]) if generation == "v010" else digest(b"")
                ),
                "sourceDiffSha256": "4" * 64,
                "baselineManifestSha256": digest(manifest_path.read_bytes()),
                "llvmComponentSha256": digest(component.read_bytes()),
            }
            artifacts = []
            lines = [f"ckc-{generation}-runtime-replay\t1"]
            lines.extend(f"{key}\t{value}" for key, value in metadata.items())
            for mode in ["unchecked", "checked"]:
                for name in SCALAR_CASES:
                    filename = f"{name}-{mode}.so"
                    data = f"{generation}/{name}/{mode}".encode()
                    (bundle / filename).write_bytes(data)
                    item = dict(case=name, mode=mode, file=filename,
                                bytes=len(data), sha256=digest(data))
                    artifacts.append(item)
                    lines.append("\t".join(["artifact", mode, name, filename,
                                             str(len(data)), digest(data)]))
            manifest = "\n".join(lines) + "\n"
            (bundle / "replay.tsv").write_text(manifest)
            replays[generation] = dict(
                metadata=metadata, manifestSha256=digest(manifest.encode()), artifacts=artifacts
            )
            self.bundles[generation] = bundle

        measured = []
        suites = []
        for mode in ["unchecked", "checked"]:
            cases = []
            for name in SCALAR_CASES:
                historical = next(
                    row for row in V010["runtime"]
                    if row["target"] == "linux-x86_64" and row["mode"] == mode
                    and row["case"] == name
                )
                case = dict(
                    name=name, referenceEquivalent=True, proofLoop=name == "proof_loop",
                    v010MedianNs=historical["median_ns"],
                    v010ClangMedianNs=historical["clang_median_ns"],
                    nativeCompileNs=100, clangCCompileNs=100, nativeColdNs=100,
                    clangCColdNs=100, peakMemoryBytes=1024, batchIterations=20_000_000,
                    result=17, warmupOrder=order(12, 3), sampleOrder=order(12, 20),
                )
                for prefix in ["native", "clangC", "replayV011Native", "replayV011Clang",
                               "replayV010Native", "replayV010Clang"]:
                    stream(case, prefix)
                endings = {
                    "candidateNative": "native", "currentClang": "clang",
                    "replayV011Clang": "replay-v011-clang",
                    "replayV010Clang": "replay-v010-clang",
                }
                for channel, ending in endings.items():
                    filename = f"{name}-{mode}-{ending}.so"
                    data = f"measured/{channel}/{name}/{mode}".encode()
                    (self.evidence / filename).write_bytes(data)
                    measured.append(dict(case=name, mode=mode, channel=channel,
                                         file=filename, bytes=len(data), sha256=digest(data)))
                    if channel == "candidateNative":
                        case["nativeArtifactBytes"] = len(data)
                    elif channel == "currentClang":
                        case["clangCArtifactBytes"] = len(data)
                cases.append(case)
            suites.append(dict(mode=mode, cases=cases))

        oracle_artifacts = []

        def oracle_suites(names, domain):
            suites = []
            for mode in ["unchecked", "checked"]:
                cases = []
                for name in names:
                    case = dict(name=name, referenceEquivalent=True, validDomain=True,
                                resultDigest="a" * 64, batchIterations=20_000_000,
                                warmupOrder=order(3, 3), sampleOrder=order(3, 20))
                    prefixes = (["candidate", "cGeneric", "rustGeneric"] if domain else
                                ["candidate", "cSimd", "rustSimd"])
                    for prefix in prefixes:
                        stream(case, prefix, 90 if domain and prefix == "candidate" else 100)
                        filename = f"{('domain' if domain else 'vector')}-{name}-{mode}-{prefix}.so"
                        data = f"oracle/{prefix}/{name}/{mode}".encode()
                        (self.evidence / filename).write_bytes(data)
                        oracle_artifacts.append(dict(
                            suite="domain" if domain else "vector", case=name, mode=mode,
                            channel=prefix, file=filename, bytes=len(data), sha256=digest(data)
                        ))
                    cases.append(case)
                suites.append(dict(mode=mode, cases=cases))
            return suites

        sizes = []
        compile_times = []
        for mode in ["unchecked", "checked"]:
            for name in VECTOR_CASES:
                sizes.append(dict(case=name, mode=mode, sourceSha256="b" * 64,
                                  candidateBytes=100, replayV011Bytes=100))
                row = dict(case=name, mode=mode, sourceSha256="b" * 64,
                           warmupOrder=order(2, 3), sampleOrder=order(2, 15))
                stream(row, "candidate", 100, 15)
                stream(row, "replayV011", 100, 15)
                compile_times.append(row)

        self.report = dict(
            schemaVersion=7, candidateVersion="0.12.0", cpuPolicy="baseline",
            fastMath=False, clangVersion="22.1.8", rustVersion="1.90.0", warmup=3,
            sampleRepetitions=7, samplingProtocol="rotating-twelve-channel-v1",
            channelNames=CHANNELS,
            targetProfile=dict(digest="d" * 64, costSchema=1, proofSchema=1, budgetSchema=1),
            runtimeReplayV011=replays["v011"], runtimeReplayV010=replays["v010"],
            evidenceDirectory=self.evidence.name, measuredArtifacts=measured, suites=suites,
            vectorSuites=oracle_suites(VECTOR_CASES, False),
            domainFactSuites=oracle_suites(DOMAIN_CASES, True),
            oracleIdentity=dict(manifestSha256="c" * 64, clangVersion="22.1.8",
                                rustVersion="1.90.0", fastMath=False,
                                contraction=False, differentialAudit=True, ubAudit=True),
            oracleArtifacts=oracle_artifacts,
            artifactSizeComparisons=sizes, compileTimeComparisons=compile_times,
            baselineV010=dict(
                commit=V010["commit"], compilerIdentity=V010["compiler_identity"],
                llvmVersion=V010["llvm_version"], target="linux-x86_64",
                harness=V010["harness"], statistics=V010["statistics"],
                sourceDigestCount=sum(key.startswith("source_digest_") for key in V010),
                sourceDigests={key.removeprefix("source_digest_"): value
                               for key, value in V010.items() if key.startswith("source_digest_")},
            ),
            optimizerComparisons=[
                dict(case=row["case"], kirMedianNs=row["median_ns"],
                     v010MirMedianNs=row["median_ns"])
                for row in V010["optimizer"] if row["target"] == "linux-x86_64"
            ],
        )
        env = dict(CKC_V011_RUNTIME_BUNDLE=str(self.bundles["v011"]),
                   CKC_V010_RUNTIME_BUNDLE=str(self.bundles["v010"]),
                   CKC_LLVM_PREFIX=str(self.prefix))
        self.environment = patch.dict(os.environ, env)
        self.environment.start()
        self.addCleanup(self.environment.stop)
        self.host = patch.object(gate, "host_target_name", return_value="linux-x86_64")
        self.host.start()
        self.addCleanup(self.host.stop)
        if hasattr(gate, "ORACLE_MANIFEST_SHA256"):
            self.oracle = patch.object(gate, "ORACLE_MANIFEST_SHA256", "c" * 64)
            self.oracle.start()
            self.addCleanup(self.oracle.stop)

    def check(self, report=None):
        path = self.root / "results.json"
        path.write_text(json.dumps(self.report if report is None else report))
        with contextlib.redirect_stdout(io.StringIO()):
            gate.check(path, V010_PATH)

    def reject(self, mutate, message):
        report = copy.deepcopy(self.report)
        mutate(report)
        with self.assertRaisesRegex(ValueError, message):
            self.check(report)

    def test_complete_schema_seven_passes(self):
        self.check()

    def test_identity_samples_orders_and_artifacts_fail_closed(self):
        for field, value, message in [
            ("schemaVersion", 6, "schemaVersion"), ("candidateVersion", "0.11.0", "candidate"),
            ("cpuPolicy", "native", "baseline"), ("fastMath", True, "fast-math"),
            ("clangVersion", "23", "Clang"), ("rustVersion", "1.89.0", "Rust"),
            ("samplingProtocol", "old", "protocol"), ("channelNames", CHANNELS[::-1], "channel"),
        ]:
            with self.subTest(field=field):
                self.reject(lambda r, f=field, v=value: r.__setitem__(f, v), message)
        self.reject(lambda r: r["targetProfile"].__setitem__("digest", "D" * 64), "profile")
        self.reject(lambda r: r["suites"][0]["cases"][0].__setitem__("sampleOrder", order(12, 19)), "order")
        self.reject(lambda r: r["vectorSuites"][0]["cases"][0].__setitem__("candidateSamplesNs", [100] * 19), "20 samples")
        self.reject(lambda r: r["measuredArtifacts"][0].__setitem__("sha256", "f" * 64), "artifact")
        self.reject(lambda r: r["runtimeReplayV011"]["artifacts"].pop(), "eight artifact")
        self.reject(lambda r: r["oracleIdentity"].__setitem__("ubAudit", False), "UB audit")

    def test_scalar_vector_domain_size_and_compile_thresholds_are_independent(self):
        self.reject(lambda r: stream(r["suites"][0]["cases"][0], "native", 111), "10%")
        self.reject(lambda r: stream(r["suites"][0]["cases"][0], "replayV011Native", 92), "8%")
        self.reject(lambda r: [stream(case, "replayV011Native", 96)
                              for suite in r["suites"] for case in suite["cases"]], "3%")
        self.reject(lambda r: stream(r["vectorSuites"][0]["cases"][0], "candidate", 112), "90%")
        self.reject(lambda r: [stream(case, "candidate", 106)
                              for suite in r["vectorSuites"] for case in suite["cases"]], "95%")
        self.reject(lambda r: [stream(case, "candidate", 96)
                              for suite in r["domainFactSuites"] for case in suite["cases"]], "5%")
        self.reject(lambda r: r["artifactSizeComparisons"][0].__setitem__("candidateBytes", 251), "2.5x")
        self.reject(lambda r: [row.__setitem__("candidateBytes", 136)
                              for row in r["artifactSizeComparisons"]], "35%")
        self.reject(lambda r: stream(r["compileTimeComparisons"][0], "candidate", 201, 15), "2x")
        self.reject(lambda r: [stream(row, "candidate", 151, 15)
                              for row in r["compileTimeComparisons"]], "1.5")

    def test_unknown_duplicate_missing_or_redirected_evidence_is_rejected(self):
        report = copy.deepcopy(self.report)
        report["unknown"] = 1
        with self.assertRaisesRegex(ValueError, "unknown"):
            self.check(report)
        self.reject(lambda r: r["vectorSuites"][0]["cases"].pop(), "corpus")
        self.reject(lambda r: r["domainFactSuites"].pop(), "separately")
        self.reject(lambda r: r["artifactSizeComparisons"].append(r["artifactSizeComparisons"][0]), "duplicate")
        artifact = self.evidence / self.report["measuredArtifacts"][0]["file"]
        original = artifact.read_bytes()
        artifact.write_bytes(original + b"changed")
        try:
            with self.assertRaisesRegex(ValueError, "artifact"):
                self.check()
        finally:
            artifact.write_bytes(original)

    def test_duplicate_json_keys_and_missing_bundles_are_rejected(self):
        path = self.root / "results.json"
        path.write_text(json.dumps(self.report).replace(
            '"schemaVersion": 7', '"schemaVersion": 7, "schemaVersion": 7'
        ))
        with self.assertRaisesRegex(ValueError, "duplicate"):
            gate.check(path, V010_PATH)
        with patch.dict(os.environ, CKC_V011_RUNTIME_BUNDLE=""):
            with self.assertRaisesRegex(ValueError, "CKC_V011_RUNTIME_BUNDLE"):
                self.check()


if __name__ == "__main__":
    unittest.main()
