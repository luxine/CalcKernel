use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use sha2::{Digest, Sha256};

use super::support::oracle::repo_root;

#[path = "../../benches/tune/predicated.rs"]
mod predicated;

#[test]
fn predicated_update_generator_should_match_frozen_cells_and_training_digest() {
    let matrix = predicated::PredicatedMatrix::generate(128, 113).expect("training matrix");
    let bits = matrix.values[..16]
        .iter()
        .map(|value| format!("{:016x}", value.to_bits()))
        .collect::<Vec<_>>();
    assert_eq!(
        bits,
        [
            "0000000000000000",
            "4026000000000000",
            "408b680000000000",
            "7ff0000000000000",
            "7ff0000000000000",
            "408b900000000000",
            "407b800000000000",
            "4081900000000000",
            "408a980000000000",
            "4073700000000000",
            "4072c00000000000",
            "7ff0000000000000",
            "408c180000000000",
            "408fd00000000000",
            "4084c80000000000",
            "408ba00000000000",
        ]
    );
    let mut solved = matrix;
    solved.scalar_floyd().expect("scalar Floyd");
    assert_eq!(
        predicated::hex(&solved.result_digest().expect("training digest")),
        predicated::TRAINING.expected_digest
    );
    assert_eq!(
        tune_result_digest("predicated-update.search", &solved),
        "42c6b833bf2207f5d0716d249099daf28dcf0250e63dbd2a9a4f438a10a215af"
    );
    let mut validation =
        predicated::PredicatedMatrix::generate(256, 127).expect("validation matrix");
    validation.scalar_floyd().expect("validation scalar Floyd");
    assert_eq!(
        predicated::hex(&validation.result_digest().expect("validation digest")),
        predicated::VALIDATION.expected_digest
    );
    assert_eq!(
        tune_result_digest("predicated-update.validation", &validation),
        "8b9f2194f5fe7afdfd1d856689ac288d04b70bf984f2310e7011d2ced391aa10"
    );
    assert_eq!(predicated::RELEASE.n, 1_024);
    assert!(predicated::checked_invocation_bytes(1_024, 129).is_err());
}

fn tune_result_digest(case_id: &str, matrix: &predicated::PredicatedMatrix) -> String {
    let result = matrix
        .canonical_result_bytes()
        .expect("canonical matrix bytes");
    let mut digest = Sha256::new();
    digest.update(b"CK-TUNE-RESULT\0");
    digest.update(1u32.to_be_bytes());
    digest.update(
        u32::try_from(case_id.len())
            .expect("case length")
            .to_be_bytes(),
    );
    digest.update(case_id.as_bytes());
    digest.update(
        u64::try_from(result.len())
            .expect("result length")
            .to_be_bytes(),
    );
    digest.update(result);
    format!("{:x}", digest.finalize())
}

#[test]
fn predicated_update_runner_should_close_direct_protocols() {
    let root = repo_root();
    let runner = env!("CARGO_BIN_EXE_ckc-tune-runner");
    let oracle = Command::new(runner)
        .env_clear()
        .current_dir(root)
        .args(["--ck-predicated-oracle", "training", "128", "113"])
        .output()
        .expect("training oracle");
    assert!(
        oracle.status.success(),
        "{}",
        String::from_utf8_lossy(&oracle.stderr)
    );
    assert_eq!(
        String::from_utf8(oracle.stdout).expect("oracle UTF-8"),
        format!(
            "CKPREDORACLE/1 training 128 113 {}\n",
            predicated::TRAINING.expected_digest
        )
    );

    for arguments in [
        vec!["--ck-predicated-oracle"],
        vec!["--ck-predicated-oracle", "training", "0128", "113"],
        vec!["--ck-predicated-oracle", "training", "128", "114"],
        vec!["--ck-predicated-oracle", "unknown", "128", "113"],
        vec![
            "--ck-predicated-profile",
            "missing.so",
            "wrong_flush",
            "128",
            "113",
        ],
        vec![
            "--ck-predicated-profile",
            "missing.so",
            "ck_profile_flush_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "128",
            "113",
        ],
        vec![
            "--ck-predicated-perf",
            "missing.so",
            "training",
            "128",
            "113",
            "1",
        ],
        vec![
            "--ck-predicated-perf",
            "missing.so",
            "release-held-out",
            "1024",
            "131",
            "0",
        ],
        vec![
            "--ck-predicated-perf",
            "missing.so",
            "release-held-out",
            "1024",
            "131",
            "129",
        ],
    ] {
        let rejected = Command::new(runner)
            .env_clear()
            .current_dir(root)
            .args(arguments)
            .output()
            .expect("negative protocol case");
        assert!(!rejected.status.success());
        assert!(!String::from_utf8_lossy(&rejected.stdout).contains("CKPRED"));
    }
}

#[cfg(all(feature = "native-toolchain", unix))]
#[test]
fn predicated_update_native_runner_should_call_ck_abi_and_flush_once() {
    let root = repo_root();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let temporary = root
        .join("target/ckc-perf/tests")
        .join(format!("predicated-native-{}-{nonce}", std::process::id()));
    let shards = temporary.join("shards");
    fs::create_dir_all(&shards).expect("profile shard directory");
    let output = temporary.join("generation");
    let build = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .current_dir(root)
        .args([
            "build",
            "benches/fixtures/tune/predicated_update.ck",
            "--out",
        ])
        .arg(&output)
        .args([
            "--kind",
            "dynamic",
            "--cpu",
            "native",
            "-O3",
            "--overflow",
            "unchecked",
            "--bounds",
            "unchecked",
            "--pgo-generate",
        ])
        .arg(&shards)
        .output()
        .expect("build instrumented Floyd");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let paths = calckernel::NativeArtifactPaths::new(
        calckernel::NativePlatform::host(),
        calckernel::NativeArtifactKind::Dynamic,
        &output,
    );
    let header = fs::read_to_string(paths.header.as_ref().expect("dynamic header"))
        .expect("read generation header");
    let flush = header
        .split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
        .find(|token| token.starts_with("ck_profile_flush_") && token.len() == 81)
        .expect("one profile flush symbol");
    assert_eq!(header.matches("ck_profile_flush_").count(), 1);

    let runner = env!("CARGO_BIN_EXE_ckc-tune-runner");
    let profile = Command::new(runner)
        .env_clear()
        .current_dir(root)
        .arg("--ck-predicated-profile")
        .arg(&paths.primary)
        .arg(flush)
        .args(["128", "113"])
        .output()
        .expect("run profile protocol");
    assert!(
        profile.status.success(),
        "{}",
        String::from_utf8_lossy(&profile.stderr)
    );
    assert_eq!(
        String::from_utf8(profile.stdout).expect("profile UTF-8"),
        format!(
            "CKPREDPROFILE/1 128 113 {} 0\n",
            predicated::TRAINING.expected_digest
        )
    );
    let shard_files = fs::read_dir(&shards)
        .expect("read shard directory")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .collect::<Vec<_>>();
    assert_eq!(shard_files.len(), 1, "exactly one completed profile shard");

    let snapshot = temporary.join("snapshot");
    let staged = snapshot.join("inputs");
    fs::create_dir_all(&staged).expect("snapshot inputs");
    let mut map_entries = Vec::new();
    for (ordinal, relative) in [
        "benches/fixtures/tune/predicated-update-training.tsv",
        "benches/fixtures/tune/predicated-update-validation.tsv",
    ]
    .into_iter()
    .enumerate()
    {
        let bytes = fs::read(root.join(relative)).expect("predicated input");
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        let staged_basename = format!("{ordinal:08x}-{}.bin", predicated::hex(&digest));
        fs::write(staged.join(&staged_basename), &bytes).expect("stage predicated input");
        map_entries.push(calckernel::TuneInputMapEntry {
            logical_path: relative
                .strip_prefix("benches/")
                .expect("benches prefix")
                .to_string(),
            staged_basename,
            bytes: u64::try_from(bytes.len()).expect("input byte count"),
            digest,
        });
    }
    let input_map = snapshot.join("input-map.bin");
    fs::write(
        &input_map,
        calckernel::encode_input_map(&map_entries).expect("encode predicated input map"),
    )
    .expect("write predicated input map");
    let tuned = Command::new(runner)
        .env_clear()
        .current_dir(root)
        .arg("--ck-predicated-tune")
        .env("CK_TUNE_PROTOCOL", "1")
        .env("CK_TUNE_ARTIFACT", &paths.primary)
        .env("CK_TUNE_ARTIFACT_KIND", "dynamic")
        .env("CK_TUNE_CASE", "predicated-update.search")
        .env("CK_TUNE_SEED", "113")
        .env("CK_TUNE_ITERATIONS", "1")
        .env("CK_TUNE_TEMP", &snapshot)
        .env("CK_TUNE_INPUT_MAP", &input_map)
        .output()
        .expect("run predicated tune protocol");
    assert!(
        tuned.status.success(),
        "{}",
        String::from_utf8_lossy(&tuned.stderr)
    );
    assert_eq!(
        String::from_utf8(tuned.stdout).expect("tune UTF-8"),
        "CKTUNE/1 predicated-update.search 113 1 1 42c6b833bf2207f5d0716d249099daf28dcf0250e63dbd2a9a4f438a10a215af\n"
    );

    let performance = Command::new(runner)
        .env_clear()
        .current_dir(root)
        .arg("--ck-predicated-perf")
        .arg(&paths.primary)
        .args(["validation", "256", "127", "2"])
        .output()
        .expect("run native predicated performance smoke");
    assert!(
        performance.status.success(),
        "{}",
        String::from_utf8_lossy(&performance.stderr)
    );
    let fields = String::from_utf8(performance.stdout)
        .expect("performance UTF-8")
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert_eq!(fields.len(), 8);
    assert_eq!(fields[0], "CKPREDPERF/1");
    assert_eq!(&fields[1..6], &["validation", "256", "127", "2", "2"]);
    assert!(fields[6].parse::<u64>().is_ok_and(|elapsed| elapsed > 0));
    assert_eq!(fields[7], predicated::VALIDATION.expected_digest);
    fs::remove_dir_all(temporary).expect("remove native fixture");
}

#[test]
fn tune_oracle_manifest_pins_exact_source_bytes_and_modes() {
    let root = repo_root();
    let manifest = fs::read_to_string(root.join("benches/oracles/tune/manifest.toml"))
        .expect("tune oracle manifest");
    for relative in [
        "benches/oracles/tune/c/tune_oracle.c",
        "benches/oracles/tune/rust/tune_oracle.rs",
    ] {
        let bytes = fs::read(root.join(relative)).expect("oracle source");
        let digest = format!("{:x}", Sha256::digest(bytes));
        assert!(manifest.contains(&format!("sha256 = \"{digest}\"")));
    }
    for required in [
        "clang_version = \"22.1.8\"",
        "rust_version = \"1.90.0\"",
        "fast_math = false",
        "fp_contraction = false",
        "overflow = \"unchecked-defined-inputs\"",
        "bounds = \"unchecked-defined-inputs\"",
        "-DCK_TUNE_GENERIC=1",
    ] {
        assert!(
            manifest.contains(required),
            "missing oracle contract {required}"
        );
    }
    assert_eq!(manifest.matches("[[case]]").count(), 7);
}

#[test]
fn tune_archive_producer_is_directly_executable_and_deterministic() {
    let root = repo_root();
    let producer = root.join("scripts/package-v014-performance-archive.py");
    #[cfg(unix)]
    {
        let mode = fs::metadata(&producer)
            .expect("archive producer")
            .permissions()
            .mode();
        assert_ne!(
            mode & 0o111,
            0,
            "archive producer must be directly executable"
        );
    }
    let source = fs::read_to_string(&producer).expect("archive producer source");
    assert!(source.starts_with("#!/usr/bin/python3\n"));
    for required in ["PAX_FORMAT", "compresslevel=9", "mtime=0", "filename=\"\""] {
        assert!(
            source.contains(required),
            "missing deterministic archive rule {required}"
        );
    }

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let temporary = root
        .join("target/ckc-perf/tests")
        .join(format!("archive-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&temporary).expect("temporary archive directory");
    let compiler = std::env::current_exe().expect("current test executable");
    let first = temporary.join("first.tar.gz");
    let second = temporary.join("second.tar.gz");
    for output in [&first, &second] {
        let status = Command::new(&producer)
            .current_dir(root)
            .args(["--compiler"])
            .arg(&compiler)
            .args([
                "--license",
                "LICENSE",
                "--notices",
                "THIRD_PARTY_NOTICES.md",
                "--out",
            ])
            .arg(output)
            .status()
            .expect("run archive producer");
        assert!(status.success());
    }
    assert_eq!(fs::read(first).unwrap(), fs::read(second).unwrap());
    fs::remove_dir_all(temporary).expect("remove temporary archive directory");
}

#[cfg(unix)]
#[test]
fn tune_performance_runner_times_the_native_batch_and_returns_exact_correctness() {
    let root = repo_root();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let temporary = root
        .join("target/ckc-perf/tests")
        .join(format!("native-runner-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&temporary).expect("temporary runner directory");
    let library = temporary.join(if cfg!(target_os = "macos") {
        "oracle.dylib"
    } else {
        "oracle.so"
    });
    let compiled = Command::new("cc")
        .current_dir(root)
        .args([
            "-std=c11",
            "-O3",
            "-fno-fast-math",
            "-ffp-contract=off",
            "-DCK_TUNE_GENERIC=1",
            "-DCK_TUNE_ORACLE_CASE=7",
            "-shared",
            "-fPIC",
            "benches/oracles/tune/c/tune_oracle.c",
            "-o",
        ])
        .arg(&library)
        .status()
        .expect("compile performance runner probe");
    assert!(compiled.success());
    let output = Command::new(env!("CARGO_BIN_EXE_ckc-tune-runner"))
        .env_clear()
        .current_dir(root)
        .args(["--ck-perf"])
        .arg(&library)
        .args([
            "contract-fixed-length",
            "contract-fixed-length.release",
            "4000",
            "83",
            "13",
            "1000",
        ])
        .output()
        .expect("run native performance batch");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).expect("UTF-8 runner receipt");
    let fields = text.split_whitespace().collect::<Vec<_>>();
    assert_eq!(fields.len(), 7);
    assert_eq!(fields[0], "CKPERF/1");
    assert_eq!(fields[1], "contract-fixed-length.release");
    assert_eq!(&fields[2..5], &["83", "1000", "1000"]);
    assert!(fields[5].parse::<u64>().is_ok_and(|elapsed| elapsed > 0));
    assert_eq!(
        fields[6],
        "843434df40c89d2af135810472bd157e63e4351d07c8f5bb5b918af54104a2c3"
    );
    fs::remove_dir_all(temporary).expect("remove temporary runner directory");
}
