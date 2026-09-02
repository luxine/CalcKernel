use std::{ffi::OsString, process::Command};

#[cfg(feature = "native-toolchain")]
use std::fs;

#[cfg(feature = "native-toolchain")]
use super::support::temp::{temp_dir, unique_id};

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
    let runner_source = root.join("runner.c");
    let runner = root.join("runner");
    let digest = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    fs::write(
        &runner_source,
        format!(
            "#include <stdio.h>\n#include <stdlib.h>\n#include <unistd.h>\nint main(void) {{ usleep(55000); const char *c=getenv(\"CK_TUNE_CASE\"),*s=getenv(\"CK_TUNE_SEED\"),*i=getenv(\"CK_TUNE_ITERATIONS\"); if(!c||!s||!i) return 2; printf(\"CKTUNE/1 %s %s %s %s {digest}\\n\",c,s,i,i); return 0; }}\n"
        ),
    )
    .expect("runner source");
    let cc = std::env::var_os("CKC_CLANG_ORACLE").unwrap_or_else(|| OsString::from("cc"));
    assert!(
        Command::new(cc)
            .args([runner_source.as_os_str(), "-o".as_ref(), runner.as_os_str()])
            .status()
            .expect("compile runner")
            .success()
    );
    let source = root.join("main.ck");
    fs::write(
        &source,
        "export fn kernel() -> u32 { let i: u32 = 0; let total: u32 = 0; while i < 12 { total = total + i; i = i + 1; } return total; } fn main() -> i32 { return 0; }",
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
    assert!(
        cold.status.success(),
        "{}",
        String::from_utf8_lossy(&cold.stderr)
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
