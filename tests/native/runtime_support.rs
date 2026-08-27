use std::{fs, path::PathBuf, process::Command};

use calckernel::{
    BoundsMode, EmitLlvmOptions, NativeContext, NativeLoweringOptions, NativeOptimizationLevel,
    NativeTarget, OverflowMode, SourceFile, check, link_native_executable,
    lower_native_executable_module_with_options, lower_to_mir,
};

pub(super) fn executable_bytes(
    source: &str,
    overflow_mode: OverflowMode,
    bounds_mode: BoundsMode,
) -> Vec<u8> {
    let checked = check(&SourceFile::new("runtime.ck", source));
    assert_eq!(checked.diagnostics, [], "{:#?}", checked.diagnostics);
    let mir = lower_to_mir(&checked.checked_program).expect("lower runtime MIR");
    let context = NativeContext::new().expect("native context");
    let target = NativeTarget::host().expect("native target");
    let module = lower_native_executable_module_with_options(
        &context,
        &target,
        &mir,
        &NativeLoweringOptions {
            emit: EmitLlvmOptions::default(),
            overflow_mode,
            bounds_mode,
        },
    )
    .expect("lower executable module")
    .verify()
    .expect("verify executable module")
    .optimize(&target, NativeOptimizationLevel::O3)
    .expect("optimize executable module");
    let object = target.emit_object(module).expect("emit executable object");
    link_native_executable(&object)
        .expect("link executable")
        .as_bytes()
        .to_vec()
}

pub(super) fn write_executable(bytes: &[u8], label: &str) -> PathBuf {
    let suffix = if cfg!(target_os = "windows") {
        ".exe"
    } else {
        ""
    };
    let path = std::env::temp_dir().join(format!(
        "ckc-native-{label}-{}-{suffix}",
        std::process::id()
    ));
    fs::write(&path, bytes).expect("write executable");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("make executable");
    }
    path
}

pub(super) fn run_executable(bytes: &[u8], label: &str) -> std::process::Output {
    let path = write_executable(bytes, label);
    let output = Command::new(&path)
        .env("PATH", "")
        .output()
        .expect("run native executable");
    fs::remove_file(path).expect("remove native executable");
    output
}
