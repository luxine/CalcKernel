use std::{ffi::OsString, fs, process::Command};

use super::support::temp::unique_id;

#[derive(Debug)]
struct CapturedOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run(args: impl IntoIterator<Item = OsString>) -> CapturedOutput {
    let output = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .args(args)
        .output()
        .expect("run ckc");
    CapturedOutput {
        code: output.status.code(),
        stdout: String::from_utf8(output.stdout).expect("stdout UTF-8"),
        stderr: String::from_utf8(output.stderr).expect("stderr UTF-8"),
    }
}

#[cfg(feature = "native-toolchain")]
fn run_empty_path(args: impl IntoIterator<Item = OsString>) -> CapturedOutput {
    let output = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .args(args)
        .env("PATH", "")
        .output()
        .expect("run ckc without external tools");
    CapturedOutput {
        code: output.status.code(),
        stdout: String::from_utf8(output.stdout).expect("stdout UTF-8"),
        stderr: String::from_utf8(output.stderr).expect("stderr UTF-8"),
    }
}

fn os(value: impl AsRef<std::ffi::OsStr>) -> OsString {
    value.as_ref().to_os_string()
}

fn fixture(source: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("ckc_cli_{}", unique_id()));
    fs::create_dir_all(&dir).expect("fixture dir");
    let path = dir.join("input.ck");
    fs::write(&path, source).expect("fixture source");
    (dir, path)
}

#[test]
fn cli_should_report_version_and_embedded_licenses() {
    let version = run([os("--version")]);
    assert_eq!(version.code, Some(0));
    assert_eq!(
        version.stdout,
        format!("ckc {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(version.stderr, "");

    let licenses = run([os("licenses")]);
    assert_eq!(licenses.code, Some(0), "{}", licenses.stderr);
    assert!(licenses.stdout.contains("The LLVM Project"));
    assert!(
        licenses
            .stdout
            .contains("Apache License v2.0 with LLVM Exceptions")
    );
    assert!(licenses.stdout.contains("Ryu floating-point conversion"));
    assert!(licenses.stdout.contains("Boost Software License"));
}

#[cfg(feature = "native-toolchain")]
#[test]
fn cli_should_report_pinned_native_toolchain_metadata() {
    let output = run([os("--version"), os("--verbose")]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    for needle in [
        "LLVM: 22.1.8",
        "Native ABI: 1",
        "Runtime ABI: 2",
        "LLVM manifest SHA-256:",
        "ORC object layer:",
    ] {
        assert!(output.stdout.contains(needle), "{}", output.stdout);
    }
}

#[cfg(not(feature = "native-toolchain"))]
#[test]
fn cli_should_use_one_native_unavailable_error_without_feature() {
    for args in [
        vec![os("run"), os("missing.ck")],
        vec![os("cache"), os("clean")],
        vec![os("emit-llvm"), os("missing.ck")],
        vec![os("build"), os("missing.ck"), os("--out"), os("x")],
    ] {
        let output = run(args);
        assert_eq!(output.code, Some(1));
        assert!(output.stderr.contains("native toolchain unavailable"));
    }
}

#[test]
fn cli_should_check_and_emit_portable_outputs() {
    let (dir, source) = fixture("export fn add(a: i32, b: i32) -> i32 { return a + b; }");
    let check = run([os("check"), os(&source)]);
    assert_eq!(check.code, Some(0), "{}", check.stderr);

    let mir = run([os("emit-mir"), os(&source)]);
    assert_eq!(mir.code, Some(0), "{}", mir.stderr);
    assert!(mir.stdout.contains("export fn add"));

    let c_path = dir.join("out.c");
    let c = run([os("emit-c"), os(&source), os("--out"), os(&c_path)]);
    assert_eq!(c.code, Some(0), "{}", c.stderr);
    assert!(
        fs::read_to_string(c_path)
            .expect("C output")
            .contains("add")
    );
    assert!(dir.join("out.h").is_file());

    let wasm_path = dir.join("out.wasm");
    let wasm = run([os("emit-wasm"), os(&source), os("--out"), os(&wasm_path)]);
    assert_eq!(wasm.code, Some(0), "{}", wasm.stderr);
    assert_eq!(&fs::read(wasm_path).expect("WASM")[..4], b"\0asm");
}

#[test]
fn cli_should_reject_unknown_and_command_irrelevant_options() {
    let (_, source) = fixture("export fn answer() -> i32 { return 42; }");
    for (args, expected) in [
        (
            vec![os("check"), os(&source), os("--unknown")],
            "Unknown option: --unknown.",
        ),
        (
            vec![os("check"), os(&source), os("--out"), os("x")],
            "Option --out is not valid for 'check'.",
        ),
        (
            vec![os("emit-c"), os(&source), os("--cpu"), os("native")],
            "Option --cpu is not valid for 'emit-c'.",
        ),
    ] {
        let output = run(args);
        assert_eq!(output.code, Some(1));
        assert!(output.stderr.contains(expected), "{}", output.stderr);
    }
}

#[cfg(feature = "native-toolchain")]
#[test]
fn emit_llvm_should_render_verified_structural_module_and_accept_checked_modes() {
    let (_, source) = fixture(
        "export fn read(items: slice<i32>, index: u32, delta: i32) -> i32 { return items[index] + delta; }",
    );
    let output = run([
        os("emit-llvm"),
        os(&source),
        os("--overflow"),
        os("checked"),
        os("--bounds"),
        os("checked"),
        os("-O0"),
    ]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    for needle in [
        "target datalayout =",
        "target triple =",
        "llvm.sadd.with.overflow.i32",
        "icmp uge i32",
        "ptr %ck_return",
    ] {
        assert!(
            output.stdout.contains(needle),
            "missing {needle}:\n{}",
            output.stdout
        );
    }
}

#[cfg(feature = "native-toolchain")]
#[test]
fn emit_llvm_should_default_to_o0_and_honor_o3() {
    let (_, source) = fixture("export fn fold(a: i64) -> i64 { return (a + 1) * 2; }");
    let o0 = run([os("emit-llvm"), os(&source)]);
    let o3 = run([os("emit-llvm"), os(&source), os("-O3")]);
    assert_eq!(o0.code, Some(0), "{}", o0.stderr);
    assert_eq!(o3.code, Some(0), "{}", o3.stderr);
    assert!(o0.stdout.contains("alloca"), "{}", o0.stdout);
    assert!(!o3.stdout.contains("alloca"), "{}", o3.stdout);
}

#[cfg(feature = "native-toolchain")]
#[test]
fn emit_llvm_should_reject_nonhost_target_before_writing_output() {
    let (dir, source) = fixture("export fn answer() -> i32 { return 42; }");
    let out = dir.join("out.ll");
    let output = run([
        os("emit-llvm"),
        os(&source),
        os("--target"),
        os("wasm32-unknown-unknown"),
        os("--out"),
        os(&out),
    ]);
    assert_eq!(output.code, Some(1));
    assert!(output.stderr.contains("does not match native target"));
    assert!(!out.exists());
}

#[cfg(feature = "native-toolchain")]
#[test]
fn build_object_should_use_embedded_llvm_with_o3_and_cpu_policies() {
    let (dir, source) = fixture("export fn answer() -> i32 { return 42; }");
    for cpu in ["baseline", "native"] {
        let out = dir.join(format!("answer-{cpu}"));
        let output = run([
            os("build"),
            os(&source),
            os("--kind"),
            os("object"),
            os("--cpu"),
            os(cpu),
            os("--out"),
            os(&out),
        ]);
        assert_eq!(output.code, Some(0), "{}", output.stderr);
        let path = object_path(&out);
        let bytes = fs::read(&path).expect("native object");
        assert!(bytes.len() > 64);
        assert!(!out.with_extension("ll").exists());
        assert!(!out.with_extension("c").exists());
    }
}

#[cfg(feature = "native-toolchain")]
#[test]
fn build_llvm_object_should_be_one_deprecated_alias_without_clang() {
    let (dir, source) = fixture("export fn answer() -> i32 { return 42; }");
    let out = dir.join("alias");
    let output = run([
        os("build-llvm"),
        os(&source),
        os("--kind"),
        os("object"),
        os("--out"),
        os(&out),
    ]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    assert_eq!(
        output.stderr.matches("deprecated").count(),
        1,
        "{}",
        output.stderr
    );
    assert!(object_path(&out).is_file());
}

#[cfg(feature = "native-toolchain")]
#[test]
fn build_should_default_to_dynamic_and_create_exact_transactional_output_sets() {
    use calckernel::{NativeArtifactKind, NativeArtifactPaths, NativePlatform};

    let (dir, source) = fixture("export fn answer() -> i32 { return 42; }");
    for (kind, artifact_kind) in [
        (None, NativeArtifactKind::Dynamic),
        (Some("static"), NativeArtifactKind::Static),
        (Some("object"), NativeArtifactKind::Object),
    ] {
        let base = dir.join(kind.unwrap_or("dynamic-default"));
        let mut arguments = vec![os("build"), os(&source), os("--out"), os(&base)];
        if let Some(kind) = kind {
            arguments.extend([os("--kind"), os(kind)]);
        }
        let output = run_empty_path(arguments);
        assert_eq!(output.code, Some(0), "{}", output.stderr);
        let paths = NativeArtifactPaths::new(NativePlatform::host(), artifact_kind, &base);
        assert!(paths.primary.is_file(), "{}", paths.primary.display());
        let header =
            fs::read_to_string(paths.header.expect("library header")).expect("read native header");
        assert!(header.contains("CK_API int32_t answer(void);"), "{header}");
        if artifact_kind == NativeArtifactKind::Dynamic {
            assert!(header.contains("dllimport"), "{header}");
        } else {
            assert!(!header.contains("dllimport"), "{header}");
            assert!(!header.contains("dllexport"), "{header}");
        }
    }
}

#[cfg(feature = "native-toolchain")]
#[test]
fn build_llvm_dynamic_should_be_one_deprecated_alias_without_external_tools() {
    use calckernel::{NativeArtifactKind, NativeArtifactPaths, NativePlatform};

    let (dir, source) = fixture("export fn answer() -> i32 { return 42; }");
    let base = dir.join("alias-dynamic");
    let output = run_empty_path([
        os("build-llvm"),
        os(&source),
        os("--kind"),
        os("dynamic"),
        os("--out"),
        os(&base),
    ]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    assert_eq!(output.stderr.matches("deprecated").count(), 1);
    let paths =
        NativeArtifactPaths::new(NativePlatform::host(), NativeArtifactKind::Dynamic, &base);
    assert!(paths.primary.is_file());
    assert!(paths.header.expect("header").is_file());
}

#[cfg(feature = "native-toolchain")]
#[test]
fn executable_kind_should_build_run_without_path_and_emit_no_header() {
    use calckernel::{NativeArtifactKind, NativeArtifactPaths, NativePlatform};

    let (dir, source) = fixture("fn main() -> i32 { print_i32(7); return 7; }");
    let base = dir.join("program");
    let output = run_empty_path([
        os("build"),
        os(&source),
        os("--kind"),
        os("executable"),
        os("--out"),
        os(&base),
    ]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let paths = NativeArtifactPaths::new(
        NativePlatform::host(),
        NativeArtifactKind::Executable,
        &base,
    );
    assert!(paths.primary.is_file());
    assert!(paths.header.is_none());
    let run = Command::new(&paths.primary)
        .env("PATH", "")
        .output()
        .expect("run standalone executable");
    assert_eq!(run.status.code(), Some(7));
    assert_eq!(run.stdout, b"7");
    assert_eq!(run.stderr, b"");
}

#[cfg(feature = "native-toolchain")]
#[test]
fn executable_without_entry_should_fail_before_creating_output() {
    let (dir, source) = fixture("export fn answer() -> i32 { return 42; }");
    let base = dir.join("program");
    let output = run_empty_path([
        os("build"),
        os(&source),
        os("--kind"),
        os("executable"),
        os("--out"),
        os(&base),
    ]);
    assert_eq!(output.code, Some(1));
    assert!(output.stderr.contains("requires fn main()"));
    assert!(!base.exists());
}

#[cfg(feature = "native-toolchain")]
#[test]
fn native_product_should_not_create_partial_output_on_semantic_failure() {
    let (dir, source) = fixture("export fn broken() -> i32 { return missing; }");
    for (command, out) in [
        ("emit-llvm", dir.join("broken.ll")),
        ("build", dir.join("broken-object")),
    ] {
        let mut args = vec![os(command), os(&source), os("--out"), os(&out)];
        if command == "build" {
            args.extend([os("--kind"), os("object")]);
        }
        let output = run(args);
        assert_eq!(output.code, Some(1));
        assert!(!out.exists());
        assert!(!object_path(&out).exists());
    }
}

#[cfg(feature = "native-toolchain")]
fn object_path(base: &std::path::Path) -> std::path::PathBuf {
    if base
        .extension()
        .is_some_and(|extension| extension == "o" || extension == "obj")
    {
        base.to_path_buf()
    } else if cfg!(target_os = "windows") {
        base.with_extension("obj")
    } else {
        base.with_extension("o")
    }
}
