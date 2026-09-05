use std::{fs, path::PathBuf};

use sha2::{Digest, Sha256};

#[path = "../../benches/runtime_replay.rs"]
mod replay_api;

use replay_api::{
    ExpectedReplay, RUNTIME_CASES, ReplayGeneration, V010_BASELINE_COMMIT, V010_BASELINE_COMPILER,
    V010_BASELINE_MANIFEST_SHA256, V011_BASELINE_COMMIT, V011_BASELINE_COMPILER,
    V011_BASELINE_MANIFEST_SHA256, load_replay,
};

struct Bundle {
    root: PathBuf,
    manifest: String,
    generation: ReplayGeneration,
}

#[test]
fn replay_sampling_should_rotate_every_channel_once_and_balance_positions() {
    let mut positions = [[0; 12]; 12];
    for round in 0..20 {
        let order = replay_api::sampling_round(round);
        assert_eq!(order, std::array::from_fn(|offset| (round + offset) % 12));
        let mut sorted = order;
        sorted.sort();
        assert_eq!(sorted, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
        for (position, channel) in order.into_iter().enumerate() {
            positions[channel][position] += 1;
        }
        assert_eq!(replay_api::sampling_round(round + 12), order);
    }
    assert!(
        positions
            .into_iter()
            .flatten()
            .all(|count| matches!(count, 1 | 2))
    );
    // An arbitrary investigation round must not overflow the schedule arithmetic.
    assert_eq!(replay_api::sampling_round(usize::MAX)[0], usize::MAX % 12);
}

#[test]
fn oracle_sampling_should_rotate_three_channels_with_the_same_fail_fast_contract() {
    let mut calls = Vec::new();
    let samples = replay_api::sample_three_channels(3, 20, |channel, warmup| {
        calls.push((channel, warmup));
        Ok::<_, ()>(calls.len() as u128)
    })
    .unwrap();
    assert_eq!(samples.warmup_order.len(), 3);
    assert_eq!(samples.sample_order.len(), 20);
    for (round, order) in samples.warmup_order.iter().enumerate() {
        assert_eq!(*order, std::array::from_fn(|offset| (round + offset) % 3));
    }
    for (round, order) in samples.sample_order.iter().enumerate() {
        assert_eq!(*order, std::array::from_fn(|offset| (round + offset) % 3));
    }
    assert!(samples.channels.iter().all(|channel| channel.len() == 20));
}

#[test]
fn oracle_sample_repetitions_should_use_the_upper_median_instead_of_a_fast_outlier() {
    let mut durations = [4_u128, 8, 8, 8, 8, 8, 8].into_iter();
    let sample = replay_api::sample_upper_median::<(), 7>(|| {
        Ok(durations.next().expect("seven frozen durations"))
    })
    .unwrap();

    assert_eq!(sample, 8);
}

#[test]
fn oracle_sample_repetitions_should_stop_at_the_first_measurement_error() {
    let mut calls = 0;
    let sample = replay_api::sample_upper_median::<&str, 7>(|| {
        calls += 1;
        if calls == 3 {
            Err("measurement failed")
        } else {
            Ok(8)
        }
    });

    assert_eq!((sample, calls), (Err("measurement failed"), 3));
}

#[test]
fn oracle_upper_median_rows_should_interleave_every_raw_channel_call() {
    let mut calls = Vec::new();
    let samples =
        replay_api::sample_three_channels_upper_median::<(), 7>(0, 1, |channel, warmup| {
            assert!(!warmup);
            calls.push(channel);
            Ok((calls.len() * 10 + channel) as u128)
        })
        .unwrap();

    assert_eq!(
        calls,
        vec![
            0, 1, 2, 1, 2, 0, 2, 0, 1, 0, 1, 2, 1, 2, 0, 2, 0, 1, 0, 1, 2,
        ]
    );
    assert_eq!(samples.channels, [vec![100], vec![111], vec![122]]);
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

#[test]
fn replay_sampling_should_execute_and_record_the_exact_warmup_and_sample_order() {
    let mut calls = Vec::new();
    let samples = replay_api::sample_channels(3, 20, |channel, warmup| {
        calls.push((channel, warmup));
        Ok::<_, ()>(calls.len() as u128)
    })
    .unwrap();
    let expected_warmup = (0..3).map(replay_api::sampling_round).collect::<Vec<_>>();
    let expected_samples = (0..20).map(replay_api::sampling_round).collect::<Vec<_>>();
    assert_eq!(samples.warmup_order, expected_warmup);
    assert_eq!(samples.sample_order, expected_samples);
    let expected_calls = expected_warmup
        .iter()
        .flatten()
        .map(|&channel| (channel, true))
        .chain(
            expected_samples
                .iter()
                .flatten()
                .map(|&channel| (channel, false)),
        )
        .collect::<Vec<_>>();
    assert_eq!(calls, expected_calls);
    for channel in 0..12 {
        let expected_values = calls
            .iter()
            .enumerate()
            .filter_map(|(index, &call)| (call == (channel, false)).then_some((index + 1) as u128))
            .collect::<Vec<_>>();
        assert_eq!(samples.channels[channel], expected_values);
        assert_eq!(samples.channels[channel].len(), 20);
    }
}

#[test]
fn replay_sampling_should_stop_immediately_on_any_warmup_or_timed_error() {
    for failing_call in [1, 36, 37, 276] {
        let mut calls = 0;
        let result = replay_api::sample_channels(3, 20, |_, _| {
            calls += 1;
            if calls == failing_call {
                Err("result/status mismatch")
            } else {
                Ok(1)
            }
        });
        assert_eq!(result.unwrap_err(), "result/status mismatch");
        assert_eq!(calls, failing_call);
    }
}

impl Bundle {
    fn new(generation: ReplayGeneration) -> Self {
        let root = super::support::temp::temp_dir("ckc-replay-loader-test");
        fs::create_dir(&root).expect("create owned fixture");
        let compiler = b"fixture compiler bytes (never executed)";
        fs::write(root.join(generation.compiler_file()), compiler).unwrap();
        let (header, commit, identity, manifest_digest) = match generation {
            ReplayGeneration::V010 => (
                "ckc-v010-runtime-replay",
                V010_BASELINE_COMMIT,
                V010_BASELINE_COMPILER,
                V010_BASELINE_MANIFEST_SHA256,
            ),
            ReplayGeneration::V011 => (
                "ckc-v011-runtime-replay",
                V011_BASELINE_COMMIT,
                V011_BASELINE_COMPILER,
                V011_BASELINE_MANIFEST_SHA256,
            ),
        };
        let mut manifest = format!(
            "{header}\t1\ncommit\t{commit}\ncompilerIdentity\t{identity}\ncompilerSha256\t{:x}\ncompilerBytes\t{}\nllvmVersion\t22.1.8\ntarget\tlinux-x86_64\ncpuPolicy\tbaseline\nrecipeSha256\t{}\nadapterSetSha256\t{}\nsourceDiffSha256\t{}\nbaselineManifestSha256\t{manifest_digest}\n",
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
        Self {
            root,
            manifest,
            generation,
        }
    }

    fn load(&self) -> Result<replay_api::RuntimeReplay, String> {
        load_replay(
            &self.root,
            &ExpectedReplay {
                generation: self.generation,
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
    for generation in [ReplayGeneration::V010, ReplayGeneration::V011] {
        let bundle = Bundle::new(generation);
        let replay = bundle.load().expect("valid bundle before dynamic loading");
        assert_eq!(replay.generation, generation);
        assert_eq!(replay.artifacts.len(), 8);
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
    assert_eq!(
        replay_api::v010_adapter_set_digest(root).unwrap(),
        values[1]
    );
    assert_eq!(
        replay_api::v011_adapter_set_digest(root).unwrap(),
        format!("{:x}", Sha256::digest([]))
    );
}

#[test]
fn replay_loader_should_reject_identity_schema_and_record_mutations() {
    let mutations = [
        ("ckc-v010-runtime-replay\t1", "ckc-v010-runtime-replay\t2"),
        (
            V010_BASELINE_COMMIT,
            "0000000000000000000000000000000000000000",
        ),
        ("calckernel 0.10.0", "calckernel 0.11.0"),
        ("llvmVersion\t22.1.8", "llvmVersion\t22.1.7"),
        ("target\tlinux-x86_64", "target\tlinux-aarch64"),
        ("cpuPolicy\tbaseline", "cpuPolicy\tnative"),
        ("recipeSha256\t", "unrecognizedField\t"),
        (
            V010_BASELINE_MANIFEST_SHA256,
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
        let bundle = Bundle::new(ReplayGeneration::V010);
        bundle.change(from, to);
        assert!(bundle.load().is_err(), "must reject {from:?} -> {to:?}");
    }
    for field in ["recipeSha256", "adapterSetSha256", "llvmComponentSha256"] {
        let bundle = Bundle::new(ReplayGeneration::V010);
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
            let bundle = Bundle::new(ReplayGeneration::V010);
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
        let bundle = Bundle::new(ReplayGeneration::V010);
        fs::remove_file(bundle.root.join(file)).unwrap();
        assert!(bundle.load().is_err(), "missing {file}");
    }
    for file in ["ckc-v010", "proof_loop-checked.so"] {
        for same_size in [false, true] {
            let bundle = Bundle::new(ReplayGeneration::V010);
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
        let bundle = Bundle::new(ReplayGeneration::V010);
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
