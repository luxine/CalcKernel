use std::{fs, path::PathBuf, process::Command};

use super::support::compiler::optimized_module;
use calckernel::{
    BoundsMode, EmitLlvmOptions, KirConsumer, NativeContext, NativeOptimizationLevel, NativeTarget,
    OverflowMode, link_native_executable, lower_native_kir_module,
};

pub(super) fn executable_object(
    source: &str,
    overflow_mode: OverflowMode,
    bounds_mode: BoundsMode,
) -> calckernel::NativeObject {
    let kir = optimized_module(
        source,
        3,
        KirConsumer::NativeExecutable,
        overflow_mode,
        bounds_mode,
    );
    let context = NativeContext::new().expect("native context");
    let target = NativeTarget::host().expect("native target");
    let module = lower_native_kir_module(&context, &target, &kir, &EmitLlvmOptions::default())
        .expect("lower executable module")
        .verify()
        .expect("verify executable module")
        .audit()
        .expect("audit executable module facts")
        .optimize(&target, NativeOptimizationLevel::O3)
        .expect("optimize executable module");
    target.emit_object(module).expect("emit executable object")
}

pub(super) fn executable_bytes(
    source: &str,
    overflow_mode: OverflowMode,
    bounds_mode: BoundsMode,
) -> Vec<u8> {
    link_native_executable(&executable_object(source, overflow_mode, bounds_mode))
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
