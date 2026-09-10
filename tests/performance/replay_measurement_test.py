"""Measurement ABI and replay identity tests; no synthetic performance acceptance."""

import ctypes
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

REPO = Path(__file__).resolve().parents[2]
PIN = "d85e0c786aaeeaa4dbaab9bffa01fcbd5f7c9f5a"
ADAPTER_NAME = "v0_13_void_return_harness.patch"
ADAPTER_SHA256 = "aed54b72fc04ad94e953a6e30100a3a83f9727fba9c80708815d79dd08d9de99"
MEASUREMENT_PATH = "scripts/measure-v013-performance.py"
POSTIMAGE_SHA256 = "7adc34501fc14f6f7e1a53e9ef02bbfa3b6eba8a738f8ac0ebc2d21e116e1f2f"
ABIS = (
    ("slice-branch-u64", ctypes.c_uint64),
    ("slice-fixed-u32", None),
    ("slice-map-u32", None),
    ("slice-zip-u32", None),
    ("slice-f64", None),
)


def load(name, path):
    specification = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(module)
    return module


collector = load("measurement_abi_collector", REPO / MEASUREMENT_PATH)
prepare = load("measurement_replay_preparer", REPO / "scripts/prepare-performance-replay.py")
gate = load("measurement_replay_checker", REPO / "scripts/check-native-performance.py")
DIFF_ARGUMENTS = (
    "diff", "--binary", "--full-index", "--no-ext-diff", "--no-textconv",
    "--no-renames", "--src-prefix=a/", "--dst-prefix=b/", "--no-color", "--unified=3", "HEAD",
)


def git(root, *arguments):
    return subprocess.run(
        ["git", *arguments], cwd=root, check=True,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
    ).stdout


def prototype(module, abi):
    kernel = object.__new__(module.Kernel)
    kernel.case = {"abi": abi}
    kernel.library_path = Path("prototype-not-loaded.so")
    # A real ctypes pointer exposes CDLL's default C-int prototype. Address 1
    # is never invoked; selected-direct tests replace the warmup invocation.
    kernel.function = ctypes.CDLL(None)._FuncPtr(1)
    kernel._bind_signature()
    return kernel


class SchemaEightPrototypeTests(unittest.TestCase):
    def test_all_kernel_return_types_match_the_native_abi(self):
        for abi, expected in ABIS:
            with self.subTest(abi=abi):
                self.assertIs(prototype(collector, abi).function.restype, expected)

    def test_selected_direct_preserves_each_kernel_return_type(self):
        for abi, expected in ABIS:
            with self.subTest(abi=abi):
                kernel = prototype(collector, abi)
                slot = ctypes.c_void_p(1)
                with patch.object(kernel, "invoke", return_value=None), \
                        patch.object(collector, "dispatch_symbol_values",
                                     return_value=(1, ctypes.addressof(slot))):
                    kernel.bind_selected_direct()
                self.assertIs(kernel.function.restype, expected)

    def test_unknown_kernel_abi_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "unsupported PGO ABI"):
            prototype(collector, "unknown")


class HistoricalMeasurementPatchTests(unittest.TestCase):
    def test_preparer_declares_exactly_the_pinned_measurement_adapter(self):
        self.assertEqual(
            prepare.baseline_identity("0.13")["adapters"],
            ((ADAPTER_NAME, ADAPTER_SHA256),),
        )

    def test_pinned_patch_changes_only_the_historical_python_prototype(self):
        adapter = REPO / "benches/baselines" / ADAPTER_NAME
        self.assertTrue(adapter.is_file(), "the exact historical measurement adapter is missing")
        self.assertEqual(hashlib.sha256(adapter.read_bytes()).hexdigest(), ADAPTER_SHA256)
        with tempfile.TemporaryDirectory(prefix="ckc-historical-abi-") as temporary:
            source = Path(temporary) / "source"
            git(REPO, "clone", "--quiet", "--shared", "--no-checkout", str(REPO), str(source))
            git(source, "-c", "core.autocrlf=false", "checkout", "--quiet", "--detach", PIN)
            self.assertEqual(git(source, "status", "--porcelain", "--untracked-files=all"), "")
            git(source, "apply", "--check", str(adapter))
            git(source, "apply", str(adapter))
            self.assertEqual(git(source, "rev-parse", "HEAD").strip(), PIN)
            self.assertEqual(git(source, "diff", "--name-only", "HEAD").splitlines(), [MEASUREMENT_PATH])
            self.assertEqual(
                hashlib.sha256((source / MEASUREMENT_PATH).read_bytes()).hexdigest(),
                POSTIMAGE_SHA256,
            )
            historical = load("historical_measurement_abi", source / MEASUREMENT_PATH)
            for abi, expected in ABIS:
                with self.subTest(abi=abi):
                    self.assertIs(prototype(historical, abi).function.restype, expected)


class DetachedReplayFixture:
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="ckc-replay-measurement-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.source = self.root / "source"
        self.adapter = REPO / "benches/baselines" / ADAPTER_NAME
        git(REPO, "clone", "--quiet", "--shared", "--no-checkout", str(REPO), str(self.source))
        git(self.source, "config", "core.autocrlf", "false")
        git(self.source, "checkout", "--quiet", "--detach", PIN)

    def apply_adapter(self):
        git(self.source, "apply", "--check", str(self.adapter))
        git(self.source, "apply", str(self.adapter))
        return hashlib.sha256(git(self.source, *DIFF_ARGUMENTS).encode()).hexdigest()

    def require_function(self, module, name):
        function = getattr(module, name, None)
        self.assertTrue(callable(function), f"missing production integrity boundary: {name}")
        return function


class ReplaySourceStateTests(DetachedReplayFixture, unittest.TestCase):
    def state(self):
        function = self.require_function(prepare, "replay_source_state")

        def run(command, cwd):
            return subprocess.run(command, cwd=cwd, check=True, text=True,
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE).stdout

        return function(self.source, prepare.baseline_identity("0.13"), run)

    def test_exact_adapted_source_is_accepted_without_compiler_changes(self):
        expected = self.apply_adapter()
        _, source_diff = self.state()
        self.assertEqual(source_diff, expected)
        self.assertEqual(git(self.source, "diff", "HEAD", "--", "src", "native", "Cargo.toml"), "")

    def test_unadapted_historical_measurement_source_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "fixed version-specific adapters"):
            self.state()

    def test_changed_measurement_postimage_is_rejected(self):
        self.apply_adapter()
        path = self.source / MEASUREMENT_PATH
        path.write_bytes(path.read_bytes() + b"\n# unauthorized measurement edit\n")
        with self.assertRaisesRegex(ValueError, "measurement postimage"):
            self.state()

    def test_additional_compiler_source_change_is_rejected(self):
        self.apply_adapter()
        with (self.source / "src/lib.rs").open("ab") as stream:
            stream.write(b"\n// unauthorized compiler edit\n")
        with self.assertRaisesRegex(ValueError, "fixed version-specific adapters"):
            self.state()

    def test_staged_compiler_source_change_is_rejected(self):
        self.apply_adapter()
        with (self.source / "src/lib.rs").open("ab") as stream:
            stream.write(b"\n// unauthorized staged compiler edit\n")
        git(self.source, "add", "src/lib.rs")
        with self.assertRaisesRegex(ValueError, "fixed version-specific adapters"):
            self.state()

    def test_untracked_source_input_is_rejected(self):
        self.apply_adapter()
        (self.source / "scripts/unexpected-measurement.py").write_bytes(b"unexpected input\n")
        with self.assertRaisesRegex(ValueError, "untracked baseline source"):
            self.state()

    def test_redirected_measurement_postimage_is_rejected(self):
        self.apply_adapter()
        path = self.source / MEASUREMENT_PATH
        target = self.root / "redirected.py"
        target.write_bytes(path.read_bytes())
        path.unlink()
        path.symlink_to(target)
        with self.assertRaisesRegex(ValueError, "measurement postimage"):
            self.state()

    def test_changed_or_redirected_repository_adapter_is_rejected(self):
        fixture = self.root / "recipe"
        names = ["benches/baselines/v0_13_replay.toml", *prepare.V013_SOURCES.values()]
        for name in names:
            destination = fixture / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(REPO / name, destination)
        adapter = fixture / "benches/baselines" / ADAPTER_NAME
        shutil.copyfile(self.adapter, adapter)
        prepare.validate_pins(fixture, "0.13")
        adapter.write_bytes(adapter.read_bytes() + b"\n")
        with self.assertRaisesRegex(ValueError, "adapter"):
            prepare.validate_pins(fixture, "0.13")
        adapter.unlink()
        adapter.symlink_to(self.adapter)
        with self.assertRaisesRegex(ValueError, "adapter"):
            prepare.validate_pins(fixture, "0.13")


class IndependentHistoricalAdapterTests(DetachedReplayFixture, unittest.TestCase):
    def setUp(self):
        super().setUp()
        self.source_diff = self.apply_adapter()
        git(self.source, "apply", "--reverse", str(self.adapter))
        self.evidence = self.root / "evidence"
        self.bundle = self.evidence / "replay-v013"
        self.bundle.mkdir(parents=True)

        def retained(relative, data):
            path = self.bundle / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
            return {"root": "evidence", "path": "replay-v013/" + relative,
                    "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}

        self.replay = {
            "commit": PIN,
            "manifest": retained("v0_13_replay.toml", gate.V013_REPLAY_MANIFEST.read_bytes()),
            "compiler": retained("ckc-v013", b"synthetic compiler; never executed"),
            "archive": retained("ckc-v013-distribution.tar.gz", b"synthetic archive; never loaded"),
            "schemaEight": retained("schema8/v0.13-results.json", b'{"synthetic":true}\n'),
            "checker": retained("check-native-performance-v013.py", subprocess.check_output(
                ["git", "show", PIN + ":scripts/check-native-performance.py"], cwd=REPO)),
        }
        self.retained_adapter = self.bundle / ADAPTER_NAME
        retained(ADAPTER_NAME, self.adapter.read_bytes())
        self.report = {"v013ReplayBundle": self.replay, "v013ReplayCommit": PIN,
                       "hardware": {"arch": "x86_64", "target": "x86_64-unknown-linux-gnu"},
                       "toolchain": {"componentManifest": {"sha256": "5" * 64}}}
        self.metadata = {
            "commit": PIN, "compilerIdentity": f"calckernel 0.13.0 ({PIN})",
            "compilerSha256": self.replay["compiler"]["sha256"],
            "compilerBytes": str(self.replay["compiler"]["bytes"]),
            "llvmVersion": "22.1.8", "target": "linux-x86_64", "cpuPolicy": "native",
            "llvmComponentSha256": "5" * 64,
            "recipeSha256": gate.named_digest(gate.RECIPE_FILES),
            "adapterSetSha256": prepare.named_digest(
                [("benches/baselines/" + ADAPTER_NAME, ADAPTER_SHA256)]),
            "sourceDiffSha256": self.source_diff,
            "baselineManifestSha256": prepare.V013_MANIFEST_SHA256,
        }
        self.records = []
        for role, key, relative in [
            ("distributionArchive", "archive", "ckc-v013-distribution.tar.gz"),
            ("historicalReport", "schemaEight", "schema8/v0.13-results.json"),
            ("historicalChecker", "checker", "check-native-performance-v013.py"),
        ]:
            record = self.replay[key]
            self.records.append(f"{role}\t{relative}\t{record['bytes']}\t{record['sha256']}")
        self.records.append(
            f"measurementAdapter\t{ADAPTER_NAME}\t{self.retained_adapter.stat().st_size}\t{ADAPTER_SHA256}")
        self.write_receipt()

    def write_receipt(self, lines=None):
        if lines is None:
            lines = ["ckc-v013-performance-replay\t3"]
            lines += [f"{key}\t{value}" for key, value in self.metadata.items()]
            lines += self.records
        (self.bundle / "replay.tsv").write_text("\n".join(lines) + "\n", encoding="utf-8")
        self.replay["evidenceFiles"] = [
            {"root": "evidence", "path": path.relative_to(self.evidence).as_posix(),
             "bytes": path.stat().st_size, "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
            for path in sorted(self.bundle.rglob("*")) if path.is_file()
        ]

    def check_receipt(self):
        function = self.require_function(gate, "schema9_check_v013_replay_receipt")
        return function(self.report, self.evidence)

    def test_exact_receipt_and_independent_adapted_checkout_match(self):
        adapter, source_diff = self.check_receipt()
        self.assertEqual(adapter, self.retained_adapter)
        self.assertEqual(source_diff, self.source_diff)
        function = self.require_function(gate, "schema9_prepare_historical_measurement")
        function(self.source, adapter, source_diff)
        self.assertEqual(git(self.source, "rev-parse", "HEAD").strip(), PIN)
        self.assertEqual(hashlib.sha256((self.source / MEASUREMENT_PATH).read_bytes()).hexdigest(),
                         POSTIMAGE_SHA256)

    def test_changed_receipt_metadata_is_rejected(self):
        for key in self.metadata:
            with self.subTest(key=key):
                original = self.metadata[key]
                self.metadata[key] = "INVALID"
                self.write_receipt()
                with self.assertRaises(ValueError):
                    self.check_receipt()
                self.metadata[key] = original

    def test_missing_duplicate_unknown_and_redirected_receipt_records_are_rejected(self):
        original = (self.bundle / "replay.tsv").read_text().splitlines()
        variants = [original[:-1], original + [original[-1]], original + ["unknown\tvalue"],
                    [line.replace(f"measurementAdapter\t{ADAPTER_NAME}",
                                  "measurementAdapter\t../escape.patch") for line in original]]
        for lines in variants:
            with self.subTest(lines=lines[-1]):
                self.write_receipt(lines)
                with self.assertRaises(ValueError):
                    self.check_receipt()

    def test_changed_retained_adapter_is_rejected_even_with_updated_file_identity(self):
        self.retained_adapter.write_bytes(self.retained_adapter.read_bytes() + b"\n")
        self.write_receipt()
        with self.assertRaisesRegex(ValueError, "adapter|measurementAdapter"):
            self.check_receipt()

    def test_non_lf_receipt_separators_are_rejected(self):
        original = (self.bundle / "replay.tsv").read_text().removesuffix("\n")
        for separator in ["\r\n", "\v", "\f", "\x85", "\u2028", "\u2029"]:
            with self.subTest(separator=repr(separator)):
                self.write_receipt([original.replace("\n", separator)])
                with self.assertRaises(ValueError):
                    self.check_receipt()

    def test_redirected_retained_adapter_is_rejected(self):
        self.retained_adapter.unlink()
        self.retained_adapter.symlink_to(self.adapter)
        self.write_receipt()
        with self.assertRaisesRegex(ValueError, "adapter|measurementAdapter"):
            self.check_receipt()

    def test_valid_but_wrong_source_diff_is_rejected_against_the_actual_checkout(self):
        self.metadata["sourceDiffSha256"] = "1" * 64
        self.write_receipt()
        adapter, source_diff = self.check_receipt()
        function = self.require_function(gate, "schema9_prepare_historical_measurement")
        with self.assertRaisesRegex(ValueError, "source.diff"):
            function(self.source, adapter, source_diff)

    def test_dirty_checkout_is_rejected_before_applying_the_adapter(self):
        (self.source / "scripts/foreign.py").write_bytes(b"not an approved input\n")
        function = self.require_function(gate, "schema9_prepare_historical_measurement")
        with self.assertRaisesRegex(ValueError, "clean"):
            function(self.source, self.retained_adapter, self.source_diff)

    def test_independent_checker_revalidates_the_adapted_source_after_use(self):
        function = self.require_function(gate, "schema9_prepare_historical_measurement")
        function(self.source, self.retained_adapter, self.source_diff)
        state = self.require_function(gate, "schema9_historical_measurement_source_diff")
        self.assertEqual(state(self.source), self.source_diff)
        with (self.source / "src/lib.rs").open("ab") as stream:
            stream.write(b"\n// unauthorized change after checking\n")
        with self.assertRaisesRegex(ValueError, "measurement source"):
            state(self.source)

    def run_real_replay_until_historical_checker(self, *, mutate_source=False):
        original_run = subprocess.run
        invocations = []

        def run(command, **arguments):
            if command[0] != gate.sys.executable:
                return original_run(command, **arguments)
            # Exercise every production file/receipt/Git guard, stopping only
            # where the original historical checker would execute. The fixture
            # compiler and archive must never be run or accepted as evidence.
            self.assertEqual(command[1], "-B")
            checkout = Path(arguments["cwd"])
            self.assertEqual(Path(command[2]), checkout / "scripts/check-native-performance.py")
            self.assertEqual(Path(command[3]), (self.bundle / "schema8/v0.13-results.json").resolve())
            self.assertEqual(arguments["env"]["GITHUB_SHA"], PIN)
            self.assertEqual(git(checkout, "rev-parse", "HEAD").strip(), PIN)
            self.assertEqual(git(checkout, "diff", "--name-only", "HEAD").splitlines(),
                             [MEASUREMENT_PATH])
            self.assertEqual(hashlib.sha256((checkout / MEASUREMENT_PATH).read_bytes()).hexdigest(),
                             POSTIMAGE_SHA256)
            self.assertEqual((checkout / "scripts/check-native-performance.py").read_bytes(),
                             (self.bundle / "check-native-performance-v013.py").read_bytes())
            invocations.append(command)
            if mutate_source:
                with (checkout / "src/lib.rs").open("ab") as stream:
                    stream.write(b"\n// unexpected mutation during checking\n")
                return subprocess.CompletedProcess(command, 0, "")
            return subprocess.CompletedProcess(command, 37, "intentional historical boundary sentinel")

        with patch.object(gate.subprocess, "run", side_effect=run):
            expected = "measurement source" if mutate_source else "historical boundary sentinel"
            with self.assertRaisesRegex(ValueError, expected):
                gate.schema9_check_replay(self.report, self.evidence)
        self.assertEqual(len(invocations), 1)

    def test_full_independent_replay_verifies_and_applies_adapter_before_historical_execution(self):
        self.run_real_replay_until_historical_checker()

    def test_full_independent_replay_rejects_source_mutation_after_historical_execution(self):
        self.run_real_replay_until_historical_checker(mutate_source=True)

    def test_full_independent_replay_rejects_wrong_receipt_diff_before_historical_execution(self):
        self.metadata["sourceDiffSha256"] = "1" * 64
        self.write_receipt()
        original_run = subprocess.run

        def run(command, **arguments):
            self.assertEqual(command[0], "git", "historical checker must not run on a wrong source diff")
            return original_run(command, **arguments)

        with patch.object(gate.subprocess, "run", side_effect=run):
            with self.assertRaisesRegex(ValueError, "source.diff"):
                gate.schema9_check_replay(self.report, self.evidence)


if __name__ == "__main__":
    unittest.main()
