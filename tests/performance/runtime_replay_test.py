"""Executable preparation-contract tests; never build or execute a fake compiler."""

import importlib.util
import pathlib
import shutil
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True

REPO = pathlib.Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "prepare_replay", REPO / "scripts/prepare-performance-replay.py"
)
PREPARE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PREPARE)


class ReplayPreparation(unittest.TestCase):
    def test_preparation_must_not_overwrite_an_existing_target(self):
        with tempfile.TemporaryDirectory(prefix="ckc-replay-owned-") as directory:
            root = pathlib.Path(directory)
            sentinel = root / "user-owned.txt"
            sentinel.write_text("keep me", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "already exists"):
                PREPARE.prepare(REPO, root)
            self.assertEqual(sentinel.read_text(encoding="utf-8"), "keep me")

    def test_exact_pins_are_accepted(self):
        manifest = PREPARE.validate_pins(REPO)
        self.assertEqual(manifest["commit"], PREPARE.BASELINE_COMMIT)
        self.assertEqual(manifest["llvm_version"], "22.1.8")
        legacy = PREPARE.validate_pins(REPO, "0.10")
        self.assertEqual(legacy["commit"], PREPARE.V010_COMMIT)

    def test_changed_runtime_source_or_adapter_is_rejected(self):
        with tempfile.TemporaryDirectory(prefix="ckc-replay-pins-") as directory:
            root = pathlib.Path(directory)
            baseline = root / "benches/baselines"
            baseline.mkdir(parents=True)
            original = REPO / "benches/baselines"
            for source in [original / "v0_10_compiler.toml", *(original / name for name, _ in PREPARE.ADAPTERS)]:
                shutil.copyfile(source, baseline / source.name)
            fixtures = root / "tests/fixtures/performance/native"
            fixtures.mkdir(parents=True)
            for case in PREPARE.RUNTIME_CASES:
                shutil.copyfile(REPO / "tests/fixtures/performance/native" / f"{case}.ck", fixtures / f"{case}.ck")
            PREPARE.validate_pins(root, "0.10")
            targets = [baseline / "v0_10_compiler.toml", *sorted(baseline.glob("*.patch")), *sorted(fixtures.glob("*.ck"))]
            for target in targets:
                with self.subTest(target=target.name):
                    original_bytes = target.read_bytes()
                    target.write_bytes(original_bytes + b"\n")
                    with self.assertRaises(ValueError):
                        PREPARE.validate_pins(root, "0.10")
                    target.write_bytes(original_bytes)
            (fixtures / "proof_loop.ck").unlink()
            with self.assertRaises((ValueError, OSError)):
                PREPARE.validate_pins(root, "0.10")

    def test_actual_verbose_compiler_identity_is_required(self):
        digest = "5" * 64
        output = f"ckc 0.11.0\nNative ABI: 1\nRuntime ABI: 2\nLLVM: 22.1.8\nLLVM manifest SHA-256: {digest}\nTarget: aarch64-apple-darwin\nCode generator: AArch64\nORC object layer: JITLink\n"
        PREPARE.validate_compiler_output(output, "aarch64-apple-darwin", digest)
        for old, new in [
            ("ckc 0.11.0", "ckc 0.10.0"),
            ("Native ABI: 1", "Native ABI: 2"),
            ("Runtime ABI: 2", "Runtime ABI: 1"),
            ("LLVM: 22.1.8", "LLVM: 22.1.7"),
            (digest, "6" * 64),
            ("aarch64-apple-darwin", "x86_64-apple-darwin"),
            ("LLVM: 22.1.8", "LLVM: 22.1.8\nLLVM: 22.1.7"),
        ]:
            with self.subTest(new=new), self.assertRaises(ValueError):
                PREPARE.validate_compiler_output(output.replace(old, new), "aarch64-apple-darwin", digest)

        legacy = output.replace("ckc 0.11.0", "ckc 0.10.0").replace(
            "Runtime ABI: 2", "Runtime ABI: 1"
        )
        PREPARE.validate_compiler_output(
            legacy, "aarch64-apple-darwin", digest, "0.10"
        )


if __name__ == "__main__":
    unittest.main()
