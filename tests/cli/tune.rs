use std::{ffi::OsString, process::Command};

#[cfg(feature = "native-toolchain")]
use std::fs;

#[cfg(feature = "native-toolchain")]
use super::support::temp::{temp_dir, unique_id};

#[cfg(all(feature = "native-toolchain", unix))]
#[path = "../support/tune_diagnostics.rs"]
mod diagnostics;

#[cfg(all(feature = "native-toolchain", unix))]
#[path = "tune_diagnostics.rs"]
mod diagnostic_tests;

fn run(args: impl IntoIterator<Item = OsString>) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ckc"))
        .args(args)
        .output()
        .expect("run ckc")
}

fn os(value: impl AsRef<std::ffi::OsStr>) -> OsString {
    value.as_ref().to_os_string()
}

#[test]
fn predicated_attestation_formatter_is_canonical_and_ordinary_cli_is_silent() {
    let attestation = calckernel::PredicatedUpdateAttestation {
        function: "floyd".to_string(),
        header: calckernel::BlockId::from_index(2),
        compare: calckernel::InstructionId::from_index(3),
        load: calckernel::InstructionId::from_index(4),
        store: calckernel::InstructionId::from_index(5),
        unit_id: [0x0a; 32],
        variant_id: [0xb1; 32],
        alternative_id: [0xc2; 32],
        vector_bits: 256,
        interleave: 4,
        minimum: 128,
        pre_state_digest: [0xde; 32],
        post_state_digest: [0xf0; 32],
    };
    let line = calckernel::format_predicated_update_attestation(&attestation);
    assert_eq!(
        line,
        format!(
            "CKTUNE-ATTEST/1 shape=predicated-same-place-update function=floyd header=2 compare=3 load=4 store=5 unit={} variant={} alternative={} vectorBits=256 uf=4 minimum=128 pre={} post={}",
            "0a".repeat(32),
            "b1".repeat(32),
            "c2".repeat(32),
            "de".repeat(32),
            "f0".repeat(32),
        )
    );
    assert!(!line.contains('\n'));

    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tune/decision-schema1-baseline.cktune");
    let ordinary = run([os("tune"), os("inspect"), os(&fixture)]);
    assert!(ordinary.status.success());
    assert!(!String::from_utf8_lossy(&ordinary.stdout).contains("CKTUNE-ATTEST/"));
    assert!(!String::from_utf8_lossy(&ordinary.stderr).contains("CKTUNE-ATTEST/"));
}

#[test]
fn tune_inspect_is_read_only_and_supports_exact_json_switch() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tune/decision-schema1-baseline.cktune");
    let text = run([os("tune"), os("inspect"), os(&fixture)]);
    assert!(
        text.status.success(),
        "{}",
        String::from_utf8_lossy(&text.stderr)
    );
    assert!(String::from_utf8_lossy(&text.stdout).starts_with("CKTUNE-INSPECT\t1\t"));
    let json = run([os("tune"), os("inspect"), os(&fixture), os("--json")]);
    assert!(
        json.status.success(),
        "{}",
        String::from_utf8_lossy(&json.stderr)
    );
    assert!(String::from_utf8_lossy(&json.stdout).starts_with("{\"fileMagic\":\"CKTUNE01\""));

    for args in [
        vec![os("tune"), os("inspect")],
        vec![os("tune"), os("inspect"), os(&fixture), os("--unknown")],
        vec![
            os("tune"),
            os("inspect"),
            os(&fixture),
            os("--json"),
            os("--json"),
        ],
    ] {
        let rejected = run(args);
        assert!(!rejected.status.success());
    }
}

#[cfg(feature = "native-toolchain")]
#[test]
fn tune_use_rejects_legacy_policy_before_source_or_output_access() {
    let root = temp_dir(&format!("ckc-tune-legacy-policy-{}", unique_id()));
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tune/decision-schema1-tuned.cktune");
    let before = fs::read(&fixture).expect("legacy decision");
    let source = root.join("missing.ck");
    let output = root.join("must-not-exist");
    let rejected = run([
        os("build"),
        os(&source),
        os("--kind"),
        os("executable"),
        os("--cpu"),
        os("native"),
        os("-O3"),
        os("--tune-use"),
        os(&fixture),
        os("--out"),
        os(&output),
    ]);
    assert_eq!(rejected.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&rejected.stderr)
            .contains("legacy tuning contract is inspection-only"),
        "{}",
        String::from_utf8_lossy(&rejected.stderr)
    );
    assert_eq!(fs::read(&fixture).expect("retained decision"), before);
    assert!(!root.exists(), "legacy replay accessed its destination");
}

#[cfg(feature = "native-toolchain")]
#[test]
fn tune_build_option_matrix_fails_before_creating_outputs() {
    let root = temp_dir(&format!("ckc-tune-cli-matrix-{}", unique_id()));
    fs::create_dir_all(&root).expect("root");
    let source = root.join("main.ck");
    fs::write(
        &source,
        "export fn kernel() -> u32 { let i: u32 = 0; let total: u32 = 0; while i < 12 { total = total + i; i = i + 1; } return total; } fn main() -> i32 { return 0; }",
    )
    .expect("source");
    let config = root.join("workload.cktune.toml");
    fs::write(&config, "not parsed because CLI must fail first").expect("config");

    let invalid = [
        vec!["--kind", "static", "--cpu", "native", "-O3"],
        vec!["--kind", "object", "--cpu", "native", "-O3"],
        vec!["--kind", "executable", "--cpu", "baseline", "-O3"],
        vec!["--kind", "executable", "--cpu", "multiversion", "-O3"],
        vec!["--kind", "executable", "--cpu", "native", "-O2"],
        vec![
            "--kind",
            "executable",
            "--cpu",
            "native",
            "-O3",
            "--sanitize-contracts",
        ],
        vec![
            "--kind",
            "executable",
            "--cpu",
            "native",
            "-O3",
            "--pgo-generate",
            "x",
        ],
    ];
    for (ordinal, tail) in invalid.into_iter().enumerate() {
        let out = root.join(format!("invalid-{ordinal}"));
        let decision = root.join(format!("invalid-{ordinal}.cktune"));
        let mut args = vec![
            os("tune"),
            os("build"),
            os(&source),
            os("--config"),
            os(&config),
            os("--out"),
            os(&out),
            os("--tune-out"),
            os(&decision),
        ];
        args.extend(tail.into_iter().map(os));
        let rejected = run(args);
        assert_eq!(
            rejected.status.code(),
            Some(1),
            "{}",
            String::from_utf8_lossy(&rejected.stderr)
        );
        assert!(!out.exists());
        assert!(!decision.exists());
    }
}

#[cfg(feature = "native-toolchain")]
#[test]
fn tune_options_are_rejected_on_ordinary_commands_and_duplicates_fail_closed() {
    let root = temp_dir("ckc-tune-cli-isolation");
    fs::create_dir_all(&root).expect("root");
    let source = root.join("main.ck");
    fs::write(&source, "fn main() -> i32 { return 0; }").expect("source");
    for args in [
        vec![os("run"), os(&source), os("--tune-use"), os("x.cktune")],
        vec![
            os("emit-kir"),
            os(&source),
            os("--tune-use"),
            os("x.cktune"),
        ],
        vec![
            os("build-llvm"),
            os(&source),
            os("--out"),
            os("x"),
            os("--tune-use"),
            os("x.cktune"),
        ],
        vec![
            os("tune"),
            os("build"),
            os(&source),
            os("--config"),
            os("a"),
            os("--config"),
            os("b"),
        ],
    ] {
        let rejected = run(args);
        assert!(!rejected.status.success());
    }
}

#[cfg(all(feature = "native-toolchain", unix))]
const TUNE_FIXTURE_DIGEST: &str =
    "8e37bed9dff3949ffd23ae638260dff869f5cc26e551f2a9e5e289a8888949fa";

#[cfg(all(feature = "native-toolchain", unix))]
fn compile_tune_c_fixture(
    root: &std::path::Path,
    name: &str,
    source: &str,
    mut compiler: Command,
) -> std::path::PathBuf {
    fs::create_dir_all(root).expect("fixture root");
    let source_path = root.join(format!("{name}.c"));
    let output = root.join(name);
    fs::write(&source_path, source).expect("C fixture source");
    let compiled = compiler
        .arg(&source_path)
        .arg("-o")
        .arg(&output)
        .output()
        .expect("compile C fixture");
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    output
}

#[cfg(all(feature = "native-toolchain", unix))]
fn compile_tune_fixture_runner(root: &std::path::Path, compiler: Command) -> std::path::PathBuf {
    compile_tune_c_fixture(
        root,
        "runner",
        include_str!("../fixtures/tune/cli-executable-runner.c"),
        compiler,
    )
}

#[cfg(all(feature = "native-toolchain", unix))]
fn tune_fixture_command(runner: &std::path::Path, artifact: &std::path::Path) -> Command {
    let mut command = Command::new(runner);
    command
        .env_clear()
        .env("CK_TUNE_PROTOCOL", "1")
        .env("CK_TUNE_ARTIFACT_KIND", "executable")
        .env("CK_TUNE_ARTIFACT", artifact)
        .env("CK_TUNE_CASE", "search")
        .env("CK_TUNE_SEED", "7")
        .env("CK_TUNE_ITERATIONS", "1");
    command
}

#[cfg(all(feature = "native-toolchain", unix))]
#[test]
fn tune_fixture_runner_executes_each_claimed_iteration() {
    use sha2::{Digest, Sha256};

    assert_eq!(
        TUNE_FIXTURE_DIGEST,
        format!("{:x}", Sha256::digest(b"66\n"))
    );
    let root = temp_dir(&format!("ckc-tune-runner-iterations-{}", unique_id()));
    let runner = compile_tune_fixture_runner(&root, Command::new("cc"));
    let artifact = compile_tune_c_fixture(
        &root,
        "artifact",
        include_str!("../fixtures/tune/cli-traced-artifact.c"),
        Command::new("cc"),
    );
    for iterations in [1, 2, 5] {
        let trace = root.join(format!("trace-{iterations}"));
        let output = tune_fixture_command(&runner, &artifact)
            .env("CK_TUNE_ITERATIONS", iterations.to_string())
            .env("CK_FIXTURE_TRACE", &trace)
            .output()
            .expect("run fixture supervisor");
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            fs::read(&trace).unwrap_or_default(),
            vec![b'x'; iterations],
            "runner claimed {iterations} iterations without executing the artifact that many times"
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("protocol UTF-8"),
            format!("CKTUNE/1 search 7 {iterations} {iterations} {TUNE_FIXTURE_DIGEST}\n")
        );
        assert!(output.stderr.is_empty());
    }
}

#[cfg(all(feature = "native-toolchain", unix))]
#[test]
fn tune_fixture_runner_rejects_unexecuted_or_incorrect_artifacts() {
    let root = temp_dir(&format!("ckc-tune-runner-reject-{}", unique_id()));
    let runner = compile_tune_fixture_runner(&root, Command::new("cc"));
    let artifact = compile_tune_c_fixture(
        &root,
        "artifact",
        include_str!("../fixtures/tune/cli-traced-artifact.c"),
        Command::new("cc"),
    );
    for mode in [
        "missing",
        "exit",
        "signal",
        "wrong-output",
        "empty-output",
        "extra-output",
    ] {
        let input = if mode == "missing" {
            root.join("missing-artifact")
        } else {
            artifact.clone()
        };
        let output = tune_fixture_command(&runner, &input)
            .env("CK_FIXTURE_MODE", mode)
            .output()
            .expect("run failing fixture artifact");
        assert!(
            !output.status.success(),
            "runner accepted {mode} without verifying the artifact: {output:?}"
        );
        assert!(output.stdout.is_empty(), "failure claimed completed work");
    }
}

#[cfg(all(feature = "native-toolchain", unix))]
#[test]
fn tune_fixture_runner_rejects_invalid_iteration_counts() {
    let root = temp_dir(&format!("ckc-tune-runner-counts-{}", unique_id()));
    let runner = compile_tune_fixture_runner(&root, Command::new("cc"));
    let artifact = compile_tune_c_fixture(
        &root,
        "artifact",
        include_str!("../fixtures/tune/cli-traced-artifact.c"),
        Command::new("cc"),
    );
    for iterations in [
        "",
        "0",
        "01",
        "-1",
        "+1",
        " 1",
        "1x",
        "18446744073709551616",
    ] {
        let trace = root.join(format!("trace-{}", unique_id()));
        let output = tune_fixture_command(&runner, &artifact)
            .env("CK_TUNE_ITERATIONS", iterations)
            .env("CK_FIXTURE_TRACE", &trace)
            .output()
            .expect("run invalid fixture iteration count");
        assert!(
            !output.status.success(),
            "runner accepted invalid iteration count {iterations:?}: {output:?}"
        );
        assert!(output.stdout.is_empty());
        assert!(!trace.exists(), "invalid count executed the artifact");
    }
}

#[cfg(all(feature = "native-toolchain", unix))]
#[test]
fn tune_build_cold_then_warm_publishes_exact_decision_and_artifact() {
    // macOS exposes the system temporary directory through `/var`, which is a
    // symlink to `/private/var`.  The tuning snapshot contract deliberately
    // rejects every symlink component, so keep this end-to-end fixture below
    // the canonical repository checkout instead.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/tune-cli-tests")
        .join(format!("cold-warm-{}", unique_id()));
    fs::create_dir_all(&root).expect("root");
    eprintln!("cold/warm fixture evidence: {}", root.display());
    let runner = compile_tune_fixture_runner(&root, Command::new("cc"));
    let digest = TUNE_FIXTURE_DIGEST;
    let source = root.join("main.ck");
    fs::write(
        &source,
        "export fn kernel() -> u32 { let i: u32 = 0; let total: u32 = 0; while i < 12 { total = total + i; i = i + 1; } return total; } fn main() -> i32 { print_u32(kernel()); print_newline(); return 0; }",
    )
    .expect("source");
    let config = root.join("workload.cktune.toml");
    fs::write(
        &config,
        format!(
            "schema=1\n[runner]\npath=\"{}\"\ninput_root=\".\"\ntimeout_ms=30000\n[[case]]\nid=\"search\"\nrole=\"search\"\nseed=7\nweight=1\nexpected_digest=\"{digest}\"\n[[case]]\nid=\"validation\"\nrole=\"validation\"\nseed=8\nweight=1\nexpected_digest=\"{digest}\"\n",
            runner.display()
        ),
    )
    .expect("manifest");
    let out = root.join("program");
    let decision = root.join("program.cktune");
    let home = root.join("home");
    fs::create_dir(&home).expect("home");
    let args = [
        os("tune"),
        os("build"),
        os(&source),
        os("--config"),
        os(&config),
        os("--out"),
        os(&out),
        os("--kind"),
        os("executable"),
        os("--cpu"),
        os("native"),
        os("-O3"),
        os("--budget"),
        os("quick"),
        os("--tune-out"),
        os(&decision),
    ];
    let cold = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .args(&args)
        .env("HOME", &home)
        .output()
        .expect("cold tune build");
    #[cfg(target_os = "macos")]
    diagnostics::after_failure(cold.status.success(), || {
        diagnostics::collect_startup(
            &root,
            &source,
            std::path::Path::new(env!("CARGO_BIN_EXE_ckc")),
            &runner,
        )
        .map_err(|error| error.to_string())
    });
    assert!(
        cold.status.success(),
        "{}",
        String::from_utf8_lossy(&cold.stderr)
    );
    assert!(
        !root.join("post-failure-startup").exists(),
        "successful cold build ran failure-only diagnostics"
    );
    assert!(String::from_utf8_lossy(&cold.stdout).contains("fresh session"));
    let artifact_path = calckernel::NativeArtifactPaths::new(
        calckernel::NativePlatform::host(),
        calckernel::NativeArtifactKind::Executable,
        &out,
    )
    .primary;
    let cold_artifact = fs::read(&artifact_path).expect("cold artifact");
    let cold_decision = fs::read(&decision).expect("cold decision");
    calckernel::decode_tune_decision(&cold_decision).expect("valid decision");

    let warm = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .args(&args)
        .env("HOME", &home)
        .output()
        .expect("warm tune build");
    assert!(
        warm.status.success(),
        "{}",
        String::from_utf8_lossy(&warm.stderr)
    );
    assert!(String::from_utf8_lossy(&warm.stdout).contains("warm exact reuse"));
    assert_eq!(
        fs::read(&artifact_path).expect("warm artifact"),
        cold_artifact
    );
    assert_eq!(fs::read(&decision).expect("warm decision"), cold_decision);

    let replay_out = root.join("replayed-program");
    let replay = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .args([
            os("build"),
            os(&source),
            os("--out"),
            os(&replay_out),
            os("--kind"),
            os("executable"),
            os("--cpu"),
            os("native"),
            os("-O3"),
            os("--tune-use"),
            os(&decision),
        ])
        .env("HOME", &home)
        .output()
        .expect("tune replay");
    assert!(
        replay.status.success(),
        "{}",
        String::from_utf8_lossy(&replay.stderr)
    );
    let replay_artifact = calckernel::NativeArtifactPaths::new(
        calckernel::NativePlatform::host(),
        calckernel::NativeArtifactKind::Executable,
        &replay_out,
    )
    .primary;
    assert_eq!(
        fs::read(&replay_artifact).expect("replay artifact"),
        cold_artifact
    );

    fs::write(&source, "fn main() -> i32 { return 1; }").expect("mutate source");
    let stale_out = root.join("stale-program");
    let stale = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .args([
            os("build"),
            os(&source),
            os("--out"),
            os(&stale_out),
            os("--kind"),
            os("executable"),
            os("--cpu"),
            os("native"),
            os("-O3"),
            os("--tune-use"),
            os(&decision),
        ])
        .env("HOME", &home)
        .output()
        .expect("stale tune replay");
    assert!(!stale.status.success());
    assert!(!stale_out.exists());
}
