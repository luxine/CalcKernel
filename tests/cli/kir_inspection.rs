use std::{ffi::OsString, fs, process::Command};

#[cfg(feature = "native-toolchain")]
use calckernel::{NativeArtifactKind, NativeArtifactPaths, NativePlatform};

use super::support::temp::unique_id;

fn os(value: impl AsRef<std::ffi::OsStr>) -> OsString {
    value.as_ref().to_os_string()
}

fn fixture(source: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("ckc_kir_inspection_{}", unique_id()));
    fs::create_dir_all(&dir).expect("fixture dir");
    let path = dir.join("input.ck");
    fs::write(&path, source).expect("fixture source");
    (dir, path)
}

fn run(args: impl IntoIterator<Item = OsString>) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ckc"))
        .args(args)
        .output()
        .expect("run ckc")
}

#[test]
fn argument_emit_kir_and_inspection_flags_should_obey_the_allowed_matrix() {
    let (dir, source) = fixture("export fn answer() -> i32 { return 42; }");
    let kir = run([
        os("emit-kir"),
        os(&source),
        os("--overflow"),
        os("checked"),
        os("--bounds"),
        os("checked"),
    ]);
    assert!(
        kir.status.success(),
        "{}",
        String::from_utf8_lossy(&kir.stderr)
    );
    let kir_text = String::from_utf8(kir.stdout).expect("KIR UTF-8");
    assert!(kir_text.contains("overflow=checked"), "{kir_text}");
    assert!(kir_text.contains("bounds=checked"), "{kir_text}");

    let rejected = run([os("check"), os(&source), os("--print-facts")]);
    assert_eq!(rejected.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&rejected.stderr)
            .contains("Option --inspection is not valid for 'check'.")
    );

    let c_out = dir.join("rejected.c");
    let sanitizer = run([
        os("emit-c"),
        os(&source),
        os("--out"),
        os(&c_out),
        os("--sanitize-contracts"),
    ]);
    assert_eq!(sanitizer.status.code(), Some(1));
    assert!(!c_out.exists(), "rejected command must not create output");
}

#[test]
fn emit_kir_consumer_should_select_every_portable_identity_and_validate_cpu_pairing() {
    let (_, source) = fixture("export fn answer() -> i32 { return 42; }");
    for (consumer, printed) in [
        ("inspection", "consumer=inspection"),
        ("c", "consumer=c"),
        ("wasm", "consumer=wasm"),
    ] {
        let output = run([os("emit-kir"), os(&source), os("--consumer"), os(consumer)]);
        assert!(
            output.status.success(),
            "{consumer}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).expect("KIR UTF-8");
        assert!(stdout.starts_with("kir-v3 "), "{consumer}: {stdout}");
        assert!(stdout.contains(printed), "{consumer}: {stdout}");
    }

    for args in [
        vec![
            os("emit-kir"),
            os(&source),
            os("--consumer"),
            os("c"),
            os("--cpu"),
            os("baseline"),
        ],
        vec![os("emit-kir"), os(&source), os("--cpu"), os("native")],
    ] {
        let output = run(args);
        assert_eq!(output.status.code(), Some(1));
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("--cpu is valid only with a Native emit-kir consumer"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let wrong_command = run([os("emit-c"), os(&source), os("--consumer"), os("c")]);
    assert_eq!(wrong_command.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&wrong_command.stderr)
            .contains("Option --consumer is not valid for 'emit-c'.")
    );
}

#[cfg(not(feature = "native-toolchain"))]
#[test]
fn emit_kir_consumer_should_fail_closed_for_native_without_the_toolchain() {
    let (_, source) = fixture("fn main() -> i32 { return 0; }");
    for consumer in ["native-library", "native-executable"] {
        let output = run([os("emit-kir"), os(&source), os("--consumer"), os(consumer)]);
        assert_eq!(output.status.code(), Some(1));
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("native toolchain unavailable"),
            "{consumer}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn optimization_level_emit_kir_consumer_should_keep_portable_default_at_o0() {
    let (_, source) = fixture("export fn answer() -> i32 { return 40 + 2; }");
    for consumer in ["inspection", "c", "wasm"] {
        let implicit = run([os("emit-kir"), os(&source), os("--consumer"), os(consumer)]);
        let explicit = run([
            os("emit-kir"),
            os(&source),
            os("--consumer"),
            os(consumer),
            os("-O0"),
        ]);
        assert!(implicit.status.success(), "{consumer}");
        assert_eq!(implicit.stdout, explicit.stdout, "{consumer}");
        assert_eq!(implicit.stderr, explicit.stderr, "{consumer}");
    }
}

#[cfg(feature = "native-toolchain")]
#[test]
fn argument_contract_sanitizer_should_accept_only_run_and_executable_build() {
    let (dir, source) = fixture("fn main() -> i32 { return 0; }");
    let rejected = dir.join("library");
    let output = run([
        os("build"),
        os(&source),
        os("--out"),
        os(&rejected),
        os("--kind"),
        os("dynamic"),
        os("--sanitize-contracts"),
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(!rejected.exists());

    let executable_base = dir.join("program");
    let accepted = run([
        os("build"),
        os(&source),
        os("--out"),
        os(&executable_base),
        os("--kind"),
        os("executable"),
        os("--sanitize-contracts"),
        os("-O0"),
    ]);
    assert!(
        accepted.status.success(),
        "{}",
        String::from_utf8_lossy(&accepted.stderr)
    );
    let executable = NativeArtifactPaths::new(
        NativePlatform::host(),
        NativeArtifactKind::Executable,
        &executable_base,
    )
    .primary;
    assert!(executable.exists());
}

#[test]
fn kir_inspection_should_be_byte_deterministic_and_identify_trusted_evidence() {
    let (_, source) = fixture(
        r#"
        export unsafe fn get(items: slice<i32>, n: u32) -> i32
        contract { requires n < items.len; effects read(items); }
        { return items[n]; }
        "#,
    );
    let args = [
        os("emit-kir"),
        os(&source),
        os("--bounds"),
        os("checked"),
        os("-O1"),
        os("--print-facts"),
        os("--print-effect-summaries"),
        os("--explain-optimization"),
    ];
    let first = run(args.clone());
    let second = run(args);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(first.stdout, second.stdout);
    assert_eq!(first.stderr, second.stderr);

    let stderr = String::from_utf8(first.stderr).expect("inspection UTF-8");
    for marker in [
        "===== KIR FACTS =====",
        "===== KIR PROOFS =====",
        "===== EFFECT SUMMARIES =====",
        "===== OPTIMIZATION EXPLANATIONS =====",
        "trusted-contract=true",
        "reason=",
    ] {
        assert!(stderr.contains(marker), "missing {marker}:\n{stderr}");
    }
    assert!(!stderr.contains(&source.to_string_lossy().to_string()));
    assert!(
        !stderr.contains("0x"),
        "inspection must not contain addresses"
    );
}

#[test]
fn kir_inspection_should_report_fixed_loop_budget_fallback_without_losing_the_artifact() {
    let mut text =
        String::from("export fn diamonds(flag: bool, start: u32) -> u32 { let x: u32 = start;");
    for _ in 0..40 {
        text.push_str("if flag { x = x + 1; } else { x = x + 2; }");
    }
    text.push_str("return x; }");
    let (_, source) = fixture(&text);
    let args = [
        os("emit-kir"),
        os(&source),
        os("-O3"),
        os("--overflow"),
        os("unchecked"),
        os("--bounds"),
        os("unchecked"),
        os("--explain-optimization"),
    ];
    let first = run(args.clone());
    let second = run(args);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(first.stdout, second.stdout);
    assert_eq!(first.stderr, second.stderr);
    assert!(String::from_utf8_lossy(&first.stdout).contains("export fn"));
    assert!(
        String::from_utf8_lossy(&first.stderr)
            .contains("f0 pass=natural-loop-analysis reason=fixed-kir-budget-exhausted"),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
}

#[test]
fn vector_explanation_portable_profile_should_report_a_stable_empty_frontier() {
    let (_, source) = fixture(
        r#"
        export fn map(a: slice<u32>, b: slice<u32>, n: u32) -> void {
          let i: u32 = 0;
          while i < n { b[i] = a[i] + 1; i = i + 1; }
        }
        "#,
    );
    let args = [
        os("emit-kir"),
        os(&source),
        os("--consumer"),
        os("inspection"),
        os("-O3"),
        os("--overflow"),
        os("unchecked"),
        os("--bounds"),
        os("unchecked"),
        os("--explain-optimization"),
    ];
    let first = run(args.clone());
    let second = run(args);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(first.stderr, second.stderr);
    let stderr = String::from_utf8(first.stderr).expect("vector explanation UTF-8");
    assert!(
        stderr.contains("optimizer-audit accepted=0 rejected=0 attempts=0"),
        "{stderr}"
    );
}

#[cfg(feature = "native-toolchain")]
#[test]
fn vector_explanation_native_winner_should_report_plan_cost_growth_proofs_and_reason() {
    let (_, source) = fixture(
        r#"
        export unsafe fn map(a: slice<u32>, b: slice<u32>, n: u32) -> void
        contract { requires noalias(a, b); effects read(a), write(b); }
        {
          let i: u32 = 0;
          while i < n { b[i] = a[i] + 7; i = i + 1; }
        }
        "#,
    );
    let args = [
        os("emit-kir"),
        os(&source),
        os("--consumer"),
        os("native-library"),
        os("--cpu"),
        os("baseline"),
        os("-O3"),
        os("--overflow"),
        os("unchecked"),
        os("--bounds"),
        os("unchecked"),
        os("--explain-optimization"),
    ];
    let first = run(args.clone());
    let second = run(args);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(first.stderr, second.stderr);
    let stderr = String::from_utf8(first.stderr).expect("native vector explanation UTF-8");
    for marker in [
        "optimizer-audit accepted=1",
        "loop:f0:loop0:loop-simd:scalar:vf",
        "vector-plan candidate=loop:f0:loop0:loop-simd:scalar:vf",
        "disposition=accepted",
        "predicates=trip-threshold",
        "cost=scalar:",
        "growth=function:",
        "proofs=canonical:",
        "reason=accepted",
    ] {
        assert!(stderr.contains(marker), "missing {marker}:\n{stderr}");
    }
    assert!(!stderr.contains("0x"), "{stderr}");
}
