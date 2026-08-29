use std::{ffi::OsString, fs, process::Command};

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

    let executable = dir.join("program");
    let accepted = run([
        os("build"),
        os(&source),
        os("--out"),
        os(&executable),
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
