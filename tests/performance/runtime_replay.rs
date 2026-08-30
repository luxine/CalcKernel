use std::{fs, path::PathBuf};

use sha2::{Digest, Sha256};

#[path = "../../benches/runtime_replay.rs"]
mod replay_api;

use replay_api::{
    BASELINE_COMMIT, BASELINE_COMPILER, BASELINE_MANIFEST_SHA256, ExpectedReplay, RUNTIME_CASES,
    load_replay,
};

struct Bundle {
    root: PathBuf,
    manifest: String,
}

#[test]
fn replay_preparation_should_validate_pinned_sources_and_actual_compiler_output() {
    let output = std::process::Command::new("python3")
        .arg(super::support::oracle::repo_root().join("tests/performance/runtime_replay_test.py"))
        .output()
        .expect("run replay preparation contract tests");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

impl Bundle {
    fn new() -> Self {
        let root = super::support::temp::temp_dir("ckc-replay-loader-test");
        fs::create_dir(&root).expect("create owned fixture");
        let compiler = b"fixture compiler bytes (never executed)";
        fs::write(root.join("ckc-v010"), compiler).unwrap();
        let mut manifest = format!(
            "ckc-v010-runtime-replay\t1\ncommit\t{BASELINE_COMMIT}\ncompilerIdentity\t{BASELINE_COMPILER}\ncompilerSha256\t{:x}\ncompilerBytes\t{}\nllvmVersion\t22.1.8\ntarget\tlinux-x86_64\ncpuPolicy\tbaseline\nrecipeSha256\t{}\nadapterSetSha256\t{}\nsourceDiffSha256\t{}\nbaselineManifestSha256\t{BASELINE_MANIFEST_SHA256}\n",
            Sha256::digest(compiler),
            compiler.len(),
            "a".repeat(64),
            "b".repeat(64),
            "c".repeat(64),
        );
        manifest.push_str(&format!("llvmComponentSha256\t{}\n", "e".repeat(64)));
        for mode in ["unchecked", "checked"] {
            for case in RUNTIME_CASES {
                let name = format!("{case}-{mode}.so");
                let bytes = format!("fixture library {case}/{mode} (not opened)");
                fs::write(root.join(&name), &bytes).unwrap();
                manifest.push_str(&format!(
                    "artifact\t{mode}\t{case}\t{name}\t{}\t{:x}\n",
                    bytes.len(),
                    Sha256::digest(bytes.as_bytes())
                ));
            }
        }
        fs::write(root.join("replay.tsv"), &manifest).unwrap();
        Self { root, manifest }
    }

    fn load(&self) -> Result<replay_api::RuntimeReplay, String> {
        load_replay(
            &self.root,
            &ExpectedReplay {
                target: "linux-x86_64",
                cpu: "baseline",
                recipe_sha256: &"a".repeat(64),
                adapter_set_sha256: &"b".repeat(64),
                llvm_component_sha256: &"e".repeat(64),
            },
        )
    }

    fn change(&self, from: &str, to: &str) {
        assert!(
            self.manifest.contains(from),
            "fixture must contain {from:?}"
        );
        fs::write(
            self.root.join("replay.tsv"),
            self.manifest.replacen(from, to, 1),
        )
        .unwrap();
    }
}

impl Drop for Bundle {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).expect("remove owned test fixture");
    }
}

#[test]
fn replay_loader_should_accept_the_exact_complete_hashed_bundle() {
    let bundle = Bundle::new();
    let replay = bundle.load().expect("valid bundle before dynamic loading");
    assert_eq!(replay.artifacts.len(), 8);
    assert_eq!(replay.metadata["commit"], BASELINE_COMMIT);
    assert_eq!(
        replay.manifest_sha256,
        format!("{:x}", Sha256::digest(bundle.manifest.as_bytes()))
    );
    for artifact in &replay.artifacts {
        assert!(RUNTIME_CASES.contains(&artifact.case.as_str()));
        assert!(matches!(artifact.mode.as_str(), "checked" | "unchecked"));
        let bytes = fs::read(&artifact.path).unwrap();
        assert_eq!(artifact.bytes, bytes.len() as u64);
        assert_eq!(artifact.sha256, format!("{:x}", Sha256::digest(bytes)));
    }
}

#[test]
fn replay_recipe_and_adapter_identity_should_match_the_independent_preparer() {
    let root = super::support::oracle::repo_root();
    let output = std::process::Command::new("python3")
        .args(["-B", "-c", r#"
import importlib.util, pathlib, sys
repo = pathlib.Path(sys.argv[1])
spec = importlib.util.spec_from_file_location("prepare_replay", repo / "scripts/prepare-performance-replay.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
print(module.recipe_digest(repo))
print(module.named_digest((f"benches/baselines/{name}", digest) for name, digest in module.ADAPTERS))
"#])
        .arg(root).output().expect("run independent recipe calculation");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    let values = text.lines().collect::<Vec<_>>();
    assert_eq!(values.len(), 2);
    assert_eq!(replay_api::recipe_digest(root).unwrap(), values[0]);
    assert_eq!(replay_api::adapter_set_digest(root).unwrap(), values[1]);
}

#[test]
fn replay_loader_should_reject_identity_schema_and_record_mutations() {
    let mutations = [
        ("ckc-v010-runtime-replay\t1", "ckc-v010-runtime-replay\t2"),
        (BASELINE_COMMIT, "0000000000000000000000000000000000000000"),
        ("calckernel 0.10.0", "calckernel 0.11.0"),
        ("llvmVersion\t22.1.8", "llvmVersion\t22.1.7"),
        ("target\tlinux-x86_64", "target\tlinux-aarch64"),
        ("cpuPolicy\tbaseline", "cpuPolicy\tnative"),
        ("recipeSha256\t", "unrecognizedField\t"),
        (
            BASELINE_MANIFEST_SHA256,
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        ),
        ("sourceDiffSha256\t", "sourceDiffSha256\tINVALID"),
        ("compilerBytes\t", "compilerBytes\t-"),
        (
            "artifact\tunchecked\tbranch_mix\t",
            "artifact\tunchecked\tunknown\t",
        ),
        ("branch_mix-unchecked.so", "../branch_mix-unchecked.so"),
        ("branch_mix-unchecked.so", "/tmp/branch_mix-unchecked.so"),
    ];
    for (from, to) in mutations {
        let bundle = Bundle::new();
        bundle.change(from, to);
        assert!(bundle.load().is_err(), "must reject {from:?} -> {to:?}");
    }
    for field in ["recipeSha256", "adapterSetSha256", "llvmComponentSha256"] {
        let bundle = Bundle::new();
        let old = match field {
            "recipeSha256" => 'a',
            "adapterSetSha256" => 'b',
            _ => 'e',
        };
        bundle.change(
            &format!("{field}\t{}", old.to_string().repeat(64)),
            &format!("{field}\t{}", "d".repeat(64)),
        );
        assert!(bundle.load().is_err(), "must reject changed {field}");
    }
    for prefix in ["commit\t", "artifact\tunchecked\tbranch_mix\t"] {
        for duplicate in [false, true] {
            let bundle = Bundle::new();
            let line = bundle
                .manifest
                .lines()
                .find(|line| line.starts_with(prefix))
                .unwrap();
            let replacement = if duplicate {
                format!("{line}\n{line}\n")
            } else {
                String::new()
            };
            bundle.change(&format!("{line}\n"), &replacement);
            assert!(
                bundle.load().is_err(),
                "duplicate={duplicate}, record={prefix}"
            );
        }
    }
}

#[test]
fn replay_loader_should_reject_missing_changed_and_redirected_files() {
    for file in ["replay.tsv", "ckc-v010", "proof_loop-checked.so"] {
        let bundle = Bundle::new();
        fs::remove_file(bundle.root.join(file)).unwrap();
        assert!(bundle.load().is_err(), "missing {file}");
    }
    for file in ["ckc-v010", "proof_loop-checked.so"] {
        for same_size in [false, true] {
            let bundle = Bundle::new();
            let mut bytes = fs::read(bundle.root.join(file)).unwrap();
            if same_size {
                bytes[0] ^= 1;
            } else {
                bytes.push(0);
            }
            fs::write(bundle.root.join(file), bytes).unwrap();
            assert!(
                bundle.load().is_err(),
                "changed {file}, same_size={same_size}"
            );
        }
    }
    #[cfg(unix)]
    {
        let bundle = Bundle::new();
        let library = bundle.root.join("proof_loop-checked.so");
        let moved = bundle.root.join("redirected.so");
        fs::rename(&library, &moved).unwrap();
        std::os::unix::fs::symlink(&moved, &library).unwrap();
        assert!(
            bundle.load().is_err(),
            "symlink must not be trusted even when bytes match"
        );
    }
}
