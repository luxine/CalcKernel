use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};

pub(super) fn run_clang(args: &[String]) -> Result<(), String> {
    run_clang_with_missing_hint(args, None)
}

pub(super) fn run_llvm_clang(args: &[String]) -> Result<(), String> {
    run_clang_with_missing_hint(
        args,
        Some("You can still run emit-llvm to generate LLVM IR without clang."),
    )
}

pub(super) fn run_clang_with_missing_hint(
    args: &[String],
    missing_hint: Option<&str>,
) -> Result<(), String> {
    let version = Command::new("clang")
        .arg("--version")
        .output()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                missing_clang_message(missing_hint)
            } else {
                error.to_string()
            }
        })?;
    if !version.status.success() {
        return Err(String::from_utf8_lossy(&version.stderr).into_owned());
    }

    let output = Command::new("clang").args(args).output().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            missing_clang_message(missing_hint)
        } else {
            error.to_string()
        }
    })?;
    if !output.status.success() {
        return Err(if output.stderr.is_empty() {
            format!(
                "clang failed with exit code {}.",
                output.status.code().unwrap_or(-1)
            )
        } else {
            String::from_utf8_lossy(&output.stderr).into_owned()
        });
    }
    Ok(())
}

pub(super) fn missing_clang_message(hint: Option<&str>) -> String {
    let message = "clang was not found. Install clang and make sure it is available on PATH.";
    match hint {
        Some(hint) => format!("{message}\n{hint}"),
        None => message.to_string(),
    }
}

pub(super) fn clang_shared_args(input: &std::path::Path, output: &std::path::Path) -> Vec<String> {
    let mut args = vec![
        "-std=c11".to_string(),
        "-O3".to_string(),
        "-Wall".to_string(),
        "-Wextra".to_string(),
        "-Werror".to_string(),
        "-DCK_BUILD_DLL".to_string(),
        "-shared".to_string(),
    ];
    if !cfg!(target_os = "windows") {
        args.push("-fPIC".to_string());
    }
    args.push(input.to_string_lossy().into_owned());
    args.push("-o".to_string());
    args.push(output.to_string_lossy().into_owned());
    args
}

pub(super) fn shared_library_output_path(path: &std::path::Path) -> PathBuf {
    let extension = path.extension().and_then(|extension| extension.to_str());
    if matches!(extension, Some("so" | "dylib" | "dll")) {
        return path.to_path_buf();
    }
    if cfg!(target_os = "macos") {
        path.with_extension("dylib")
    } else if cfg!(target_os = "windows") {
        path.with_extension("dll")
    } else {
        path.with_extension("so")
    }
}

pub(super) fn object_output_path(path: &std::path::Path) -> PathBuf {
    let extension = path.extension().and_then(|extension| extension.to_str());
    if matches!(extension, Some("o" | "obj")) {
        return path.to_path_buf();
    }
    if cfg!(target_os = "windows") {
        path.with_extension("obj")
    } else {
        path.with_extension("o")
    }
}

pub(super) fn llvm_intermediate_path(output_path: &std::path::Path, kind: &str) -> PathBuf {
    if kind == "object" {
        return output_path.with_extension("ll");
    }
    PathBuf::from(format!("{}.ll", output_path.display()))
}

pub(super) fn detect_native_llvm_target_triple() -> Option<String> {
    let mut child = Command::new("clang")
        .args(["-###", "-x", "c", "-c", "-", "-o", "/dev/null"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;

    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(b"int ik_target_probe;\n").ok()?;
    }

    let output = child.wait_with_output().ok()?;
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    extract_llvm_target_triple(&combined)
}

pub(super) fn extract_llvm_target_triple(output: &str) -> Option<String> {
    let marker = "\"-triple\"";
    let after_marker = output.get(output.find(marker)? + marker.len()..)?;
    let first_quote = after_marker.find('"')?;
    let triple_start = first_quote + 1;
    let after_start = after_marker.get(triple_start..)?;
    let triple_end = after_start.find('"')?;
    Some(after_start[..triple_end].to_string())
}
