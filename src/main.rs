use std::{
    env, fs,
    io::Write,
    path::PathBuf,
    process,
    process::{Command, Stdio},
};

use calckernel::{
    BoundsMode, EmitCOptions, EmitLlvmOptions, EmitWasmOptions, MirModule, MirPassBoundsMode,
    MirPassContext, MirPassDebugFlags, MirPassOverflowMode, MirPassTargetBackend, OverflowMode,
    SourceFile, build_mir_optimization_pipeline, check, emit_c_header, emit_c_module_with_header,
    emit_llvm_module, emit_wasm_module_with_options, emit_wat_module_with_options,
    format_diagnostics, lower_to_mir, print_mir_module, print_mir_pass_pipeline,
    run_mir_pass_pipeline,
};

fn main() {
    let code = run(env::args().skip(1).collect());
    process::exit(code);
}

fn run(args: Vec<String>) -> i32 {
    let Some(command) = args.first().map(String::as_str) else {
        eprint!("{}", usage());
        return 2;
    };

    if command == "--help" || command == "-h" {
        print!("{}", usage());
        return 0;
    }

    let result = match command {
        "check" => parse_and_run(&args[1..], run_check),
        "emit-mir" => parse_and_run(&args[1..], run_emit_mir),
        "emit-c" => parse_and_run(&args[1..], run_emit_c),
        "emit-wat" => parse_and_run(&args[1..], run_emit_wat),
        "emit-wasm" => parse_and_run(&args[1..], run_emit_wasm),
        "emit-llvm" => parse_and_run(&args[1..], run_emit_llvm),
        "build" => parse_and_run(&args[1..], run_build),
        "build-llvm" => parse_and_run(&args[1..], run_build_llvm),
        _ => {
            eprint!("{}", usage());
            return 2;
        }
    };

    match result {
        Ok(()) => 0,
        Err(message) => {
            print_error(&message);
            1
        }
    }
}

fn parse_and_run(
    args: &[String],
    run_command: fn(&ParsedArgs) -> Result<(), String>,
) -> Result<(), String> {
    let parsed = ParsedArgs::parse(args)?;
    run_command(&parsed)
}

fn print_error(message: &str) {
    if message.ends_with('\n') {
        eprint!("{message}");
    } else {
        eprintln!("{message}");
    }
}

fn run_check(args: &ParsedArgs) -> Result<(), String> {
    let input = require_input(args, "check")?;
    let (source, checked) = check_file(input)?;
    if !checked.diagnostics.is_empty() {
        return Err(format_diagnostics(&source, &checked.diagnostics));
    }
    println!("OK: {input}");
    Ok(())
}

fn run_emit_mir(args: &ParsedArgs) -> Result<(), String> {
    let input = require_input(args, "emit-mir")?;
    let opt_level = parse_opt_level(args)?;
    let (source, checked) = check_file(input)?;
    if !checked.diagnostics.is_empty() {
        return Err(format_diagnostics(&source, &checked.diagnostics));
    }
    let mir = lower_and_optimize(
        &checked.checked_program,
        opt_level,
        MirPassOverflowMode::Unchecked,
        MirPassBoundsMode::Unchecked,
        MirPassTargetBackend::Mir,
        &args.debug,
    )?;
    write_or_print(args.out.as_deref(), &print_mir_module(&mir), "MIR")
}

fn run_emit_c(args: &ParsedArgs) -> Result<(), String> {
    let input = require_input(args, "emit-c")?;
    let out = require_out(args, "emit-c")?;
    let header = args
        .header
        .clone()
        .unwrap_or_else(|| default_header_file_for_c_output(out));
    let overflow_mode = parse_overflow_mode(args)?;
    let bounds_mode = parse_bounds_mode(args)?;
    let opt_level = parse_opt_level(args)?;
    let (source, checked) = check_file(input)?;
    if !checked.diagnostics.is_empty() {
        return Err(format_diagnostics(&source, &checked.diagnostics));
    }
    let mir = lower_and_optimize(
        &checked.checked_program,
        opt_level,
        match overflow_mode {
            OverflowMode::Unchecked => MirPassOverflowMode::Unchecked,
            OverflowMode::Checked => MirPassOverflowMode::Checked,
        },
        mir_bounds_mode(bounds_mode),
        MirPassTargetBackend::C,
        &args.debug,
    )?;
    let header_name = header_include_name(&header)?;
    let options = EmitCOptions {
        overflow_mode,
        bounds_mode,
        opt_level,
    };
    let text = emit_c_module_with_header(&mir, options, &header_name);
    let header_text = emit_c_header(&mir, options);
    write_text_atomic(&header, &header_text)?;
    write_text_atomic(out, &text)?;
    println!(
        "OK: emitted C with overflow={}, bounds={}",
        match overflow_mode {
            OverflowMode::Unchecked => "unchecked",
            OverflowMode::Checked => "checked",
        },
        bounds_mode_name(bounds_mode),
    );
    println!("Wrote {}", absolutize(out).display());
    println!("Wrote {}", absolutize(&header).display());
    Ok(())
}

fn run_emit_wat(args: &ParsedArgs) -> Result<(), String> {
    let input = require_input(args, "emit-wat")?;
    let overflow_mode = parse_overflow_mode(args)?;
    if overflow_mode == OverflowMode::Checked {
        return Err(unsupported_checked_wasm_error());
    }
    let bounds_mode = parse_bounds_mode(args)?;
    if bounds_mode == BoundsMode::Checked {
        return Err(unsupported_checked_wasm_bounds_error());
    }
    let opt_level = parse_opt_level(args)?;
    let (source, checked) = check_file(input)?;
    if !checked.diagnostics.is_empty() {
        return Err(format_diagnostics(&source, &checked.diagnostics));
    }
    let mir = lower_and_optimize(
        &checked.checked_program,
        opt_level,
        MirPassOverflowMode::Unchecked,
        MirPassBoundsMode::Unchecked,
        MirPassTargetBackend::Wasm,
        &args.debug,
    )?;
    write_or_print_single_line(
        args.out.as_deref(),
        &emit_wat_module_with_options(&mir, EmitWasmOptions { opt_level }),
        "WAT",
    )
}

fn run_emit_wasm(args: &ParsedArgs) -> Result<(), String> {
    let input = require_input(args, "emit-wasm")?;
    let out = require_out(args, "emit-wasm")?;
    let overflow_mode = parse_overflow_mode(args)?;
    if overflow_mode == OverflowMode::Checked {
        return Err(unsupported_checked_wasm_error());
    }
    let bounds_mode = parse_bounds_mode(args)?;
    if bounds_mode == BoundsMode::Checked {
        return Err(unsupported_checked_wasm_bounds_error());
    }
    let opt_level = parse_opt_level(args)?;
    let (source, checked) = check_file(input)?;
    if !checked.diagnostics.is_empty() {
        return Err(format_diagnostics(&source, &checked.diagnostics));
    }
    let mir = lower_and_optimize(
        &checked.checked_program,
        opt_level,
        MirPassOverflowMode::Unchecked,
        MirPassBoundsMode::Unchecked,
        MirPassTargetBackend::Wasm,
        &args.debug,
    )?;
    let bytes = emit_wasm_module_with_options(&mir, EmitWasmOptions { opt_level })?;
    write_bytes_atomic(out, &bytes)?;
    println!("OK: emitted WASM {out}");
    Ok(())
}

fn run_emit_llvm(args: &ParsedArgs) -> Result<(), String> {
    let input = require_input(args, "emit-llvm")?;
    let overflow_mode = parse_overflow_mode(args)?;
    if overflow_mode == OverflowMode::Checked {
        return Err(unsupported_checked_llvm_error());
    }
    let bounds_mode = parse_bounds_mode(args)?;
    if bounds_mode == BoundsMode::Checked {
        return Err(unsupported_checked_llvm_bounds_error());
    }
    let opt_level = parse_opt_level(args)?;
    let (source, checked) = check_file(input)?;
    if !checked.diagnostics.is_empty() {
        return Err(format_diagnostics(&source, &checked.diagnostics));
    }
    let mir = lower_and_optimize(
        &checked.checked_program,
        opt_level,
        MirPassOverflowMode::Unchecked,
        MirPassBoundsMode::Unchecked,
        MirPassTargetBackend::Llvm,
        &args.debug,
    )?;
    let text = emit_llvm_module(
        &mir,
        &EmitLlvmOptions {
            source_file_name: Some(input.to_string()),
            target_triple: args
                .target
                .clone()
                .or_else(detect_native_llvm_target_triple),
        },
    );
    write_or_print_single_line(args.out.as_deref(), &text, "LLVM IR")
}

fn run_build(args: &ParsedArgs) -> Result<(), String> {
    let input = require_input(args, "build")?;
    let out = require_out(args, "build")?;
    let overflow_mode = parse_overflow_mode(args)?;
    let bounds_mode = parse_bounds_mode(args)?;
    let opt_level = parse_opt_level(args)?;
    let (source, checked) = check_file(input)?;
    if !checked.diagnostics.is_empty() {
        return Err(format_diagnostics(&source, &checked.diagnostics));
    }

    let mir = lower_and_optimize(
        &checked.checked_program,
        opt_level,
        match overflow_mode {
            OverflowMode::Unchecked => MirPassOverflowMode::Unchecked,
            OverflowMode::Checked => MirPassOverflowMode::Checked,
        },
        mir_bounds_mode(bounds_mode),
        MirPassTargetBackend::C,
        &args.debug,
    )?;
    let requested_output = absolutize(out);
    let c_path = format!("{}.c", requested_output.display());
    let header_path = format!("{}.h", requested_output.display());
    let header_name = header_include_name(&header_path)?;
    let output_path = shared_library_output_path(&requested_output);
    let options = EmitCOptions {
        overflow_mode,
        bounds_mode,
        opt_level,
    };
    write_text_atomic(
        &c_path,
        &emit_c_module_with_header(&mir, options, &header_name),
    )?;
    write_text_atomic(&header_path, &emit_c_header(&mir, options))?;
    run_clang(&clang_shared_args(&PathBuf::from(&c_path), &output_path))?;
    println!(
        "OK: built library with overflow={}, bounds={}",
        match overflow_mode {
            OverflowMode::Unchecked => "unchecked",
            OverflowMode::Checked => "checked",
        },
        bounds_mode_name(bounds_mode),
    );
    println!("{}", output_path.display());
    Ok(())
}

fn run_build_llvm(args: &ParsedArgs) -> Result<(), String> {
    let input = require_input(args, "build-llvm")?;
    let out = require_out(args, "build-llvm")?;
    let overflow_mode = parse_overflow_mode(args)?;
    if overflow_mode == OverflowMode::Checked {
        return Err(unsupported_checked_llvm_error());
    }
    let bounds_mode = parse_bounds_mode(args)?;
    if bounds_mode == BoundsMode::Checked {
        return Err(unsupported_checked_llvm_bounds_error());
    }
    let opt_level = parse_opt_level(args)?;
    let kind = args.kind.as_deref().unwrap_or("dynamic");
    if kind != "dynamic" && kind != "object" {
        return Err(format!(
            "Invalid value for --kind: {kind}. Expected 'dynamic' or 'object'."
        ));
    }

    let (source, checked) = check_file(input)?;
    if !checked.diagnostics.is_empty() {
        return Err(format_diagnostics(&source, &checked.diagnostics));
    }
    let mir = lower_and_optimize(
        &checked.checked_program,
        opt_level,
        MirPassOverflowMode::Unchecked,
        MirPassBoundsMode::Unchecked,
        MirPassTargetBackend::Llvm,
        &args.debug,
    )?;
    let requested_output = absolutize(out);
    let output_path = if kind == "object" {
        object_output_path(&requested_output)
    } else {
        shared_library_output_path(&requested_output)
    };
    let ll_path = llvm_intermediate_path(&requested_output, kind);
    let text = emit_llvm_module(
        &mir,
        &EmitLlvmOptions {
            source_file_name: Some(input.to_string()),
            target_triple: args.target.clone(),
        },
    );
    write_text_atomic(&ll_path.to_string_lossy(), &text)?;
    let clang_args = if kind == "object" {
        vec![
            format!("-O{opt_level}"),
            "-c".to_string(),
            ll_path.to_string_lossy().into_owned(),
            "-o".to_string(),
            output_path.to_string_lossy().into_owned(),
        ]
    } else {
        let mut args = vec![format!("-O{opt_level}"), "-shared".to_string()];
        if !cfg!(target_os = "windows") {
            args.push("-fPIC".to_string());
        }
        args.push(ll_path.to_string_lossy().into_owned());
        args.push("-o".to_string());
        args.push(output_path.to_string_lossy().into_owned());
        args
    };
    run_llvm_clang(&clang_args)?;
    println!(
        "OK: built LLVM {}",
        if kind == "object" {
            "object"
        } else {
            "library"
        }
    );
    println!("{}", output_path.display());
    Ok(())
}

fn check_file(input: &str) -> Result<(SourceFile, calckernel::CheckResult), String> {
    validate_source_file_extension(input)?;
    let path = absolutize(input);
    let bytes = fs::read(&path).map_err(|error| format_read_file_error(&path, error))?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let source = SourceFile::new(input, text);
    let checked = check(&source);
    Ok((source, checked))
}

fn format_read_file_error(path: &std::path::Path, error: std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::NotFound => {
            format_node_open_error(path, "ENOENT", "no such file or directory")
        }
        std::io::ErrorKind::PermissionDenied => {
            format_node_open_error(path, "EACCES", "permission denied")
        }
        std::io::ErrorKind::IsADirectory => {
            "EISDIR: illegal operation on a directory, read".to_string()
        }
        _ => format!("{}: {error}", path.display()),
    }
}

fn format_open_file_error(path: &std::path::Path, error: std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::NotFound => {
            format_node_open_error(path, "ENOENT", "no such file or directory")
        }
        std::io::ErrorKind::PermissionDenied => {
            format_node_open_error(path, "EACCES", "permission denied")
        }
        std::io::ErrorKind::IsADirectory => {
            format_node_open_error(path, "EISDIR", "illegal operation on a directory")
        }
        _ => error.to_string(),
    }
}

fn format_node_open_error(path: &std::path::Path, code: &str, message: &str) -> String {
    format!("{code}: {message}, open '{}'", path.display())
}

fn format_rename_file_error(
    from: &std::path::Path,
    to: &std::path::Path,
    error: std::io::Error,
) -> String {
    match error.kind() {
        std::io::ErrorKind::NotFound => format!(
            "ENOENT: no such file or directory, rename '{}' -> '{}'",
            from.display(),
            to.display()
        ),
        std::io::ErrorKind::PermissionDenied => format!(
            "EACCES: permission denied, rename '{}' -> '{}'",
            from.display(),
            to.display()
        ),
        std::io::ErrorKind::IsADirectory => format!(
            "EISDIR: illegal operation on a directory, rename '{}' -> '{}'",
            from.display(),
            to.display()
        ),
        _ => error.to_string(),
    }
}

fn validate_source_file_extension(input: &str) -> Result<(), String> {
    if input.ends_with(".ik") {
        return Err(
            "CalcKernel source files use .ck. Legacy .ik files are no longer accepted.".to_string(),
        );
    }

    if !input.ends_with(".ck") {
        return Err("CalcKernel source files use .ck.".to_string());
    }

    Ok(())
}

fn lower_and_optimize(
    checked_program: &calckernel::CheckedProgram,
    opt_level: u8,
    overflow_mode: MirPassOverflowMode,
    bounds_mode: MirPassBoundsMode,
    target_backend: MirPassTargetBackend,
    debug: &MirPassDebugFlags,
) -> Result<MirModule, String> {
    let mir = lower_to_mir(checked_program).map_err(|error| error.to_string())?;
    let pipeline = build_mir_optimization_pipeline(opt_level);
    if debug.print_pass_pipeline {
        eprintln!("MIR pass pipeline: {}", print_mir_pass_pipeline(&pipeline));
    }
    if debug.print_mir_before_opt {
        eprint!("MIR before optimization:\n{}", print_mir_module(&mir));
    }
    let result = run_mir_pass_pipeline(
        mir,
        &pipeline,
        &MirPassContext {
            opt_level,
            overflow_mode,
            bounds_mode,
            target_backend,
            debug: debug.clone(),
        },
    );
    if debug.print_mir_after_opt {
        eprint!(
            "MIR after optimization:\n{}",
            print_mir_module(&result.module)
        );
    }
    if !result.validation_errors.is_empty() {
        return Err(result
            .validation_errors
            .into_iter()
            .map(|error| error.message)
            .collect::<Vec<_>>()
            .join("\n"));
    }
    Ok(result.module)
}

fn write_or_print(out: Option<&str>, text: &str, label: &str) -> Result<(), String> {
    if let Some(out) = out {
        write_text(out, text)?;
        println!("OK: emitted {label}");
        println!("Wrote {}", absolutize(out).display());
    } else {
        print!("{text}");
    }
    Ok(())
}

fn write_or_print_single_line(out: Option<&str>, text: &str, label: &str) -> Result<(), String> {
    if let Some(out) = out {
        write_text_atomic(out, text)?;
        println!("OK: emitted {label} {out}");
    } else {
        print!("{text}");
    }
    Ok(())
}

fn write_text(path: &str, text: &str) -> Result<(), String> {
    let path = PathBuf::from(path);
    create_parent_dirs(&path)?;
    fs::write(&path, text).map_err(|error| format_open_file_error(&path, error))
}

fn write_text_atomic(path: &str, text: &str) -> Result<(), String> {
    write_bytes_atomic(path, text.as_bytes())
}

fn write_bytes_atomic(path: &str, bytes: &[u8]) -> Result<(), String> {
    let path = PathBuf::from(path);
    create_parent_dirs(&path)?;
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    let temp_path = PathBuf::from(format!("{}.tmp-{}-{millis}", path.display(), process::id()));
    if let Err(error) = fs::write(&temp_path, bytes) {
        let _ = fs::remove_file(&temp_path);
        return Err(format_open_file_error(&temp_path, error));
    }
    if let Err(error) = fs::rename(&temp_path, &path) {
        let _ = fs::remove_file(&temp_path);
        return Err(format_rename_file_error(&temp_path, &path, error));
    }
    Ok(())
}

fn create_parent_dirs(path: &std::path::Path) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| format_make_directory_error(parent, error))?;
    }
    Ok(())
}

fn format_make_directory_error(path: &std::path::Path, error: std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::AlreadyExists => {
            format!("EEXIST: file already exists, mkdir '{}'", path.display())
        }
        std::io::ErrorKind::PermissionDenied => {
            format!("EACCES: permission denied, mkdir '{}'", path.display())
        }
        std::io::ErrorKind::NotADirectory => {
            format!("ENOTDIR: not a directory, mkdir '{}'", path.display())
        }
        _ => error.to_string(),
    }
}

fn absolutize(path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn default_header_file_for_c_output(c_file: &str) -> String {
    let path = PathBuf::from(c_file);
    let file_name = path
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .unwrap_or(c_file)
        .to_string();
    let base_name = file_name
        .rfind('.')
        .filter(|index| *index > 0)
        .map_or(file_name.as_str(), |index| &file_name[..index]);
    let mut header = path;
    header.set_file_name(format!("{base_name}.h"));
    header.to_string_lossy().into_owned()
}

fn header_include_name(header: &str) -> Result<String, String> {
    PathBuf::from(header)
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("Invalid header path '{header}'."))
}

fn require_input<'args>(args: &'args ParsedArgs, command: &str) -> Result<&'args str, String> {
    if args.positionals.len() != 1 {
        return Err(format!("Usage error for '{command}'.\n{}\n", usage()));
    }
    Ok(&args.positionals[0])
}

fn require_out<'args>(args: &'args ParsedArgs, command: &str) -> Result<&'args str, String> {
    args.out
        .as_deref()
        .ok_or_else(|| format!("Usage error for '{command}': missing --out.\n{}\n", usage()))
}

#[derive(Debug, Clone)]
struct ParsedArgs {
    positionals: Vec<String>,
    out: Option<String>,
    overflow: Option<String>,
    bounds: Option<String>,
    opt_level: Option<String>,
    target: Option<String>,
    kind: Option<String>,
    header: Option<String>,
    debug: MirPassDebugFlags,
}

impl ParsedArgs {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut parsed = Self {
            positionals: Vec::new(),
            out: None,
            overflow: None,
            bounds: None,
            opt_level: None,
            target: None,
            kind: None,
            header: None,
            debug: MirPassDebugFlags::default(),
        };
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--out" => {
                    index += 1;
                    parsed.out = Some(require_long_flag_value(args, index, "--out")?.to_string());
                }
                "-o" => {
                    index += 1;
                    parsed.out = Some(require_short_flag_value(args, index, "-o")?.to_string());
                }
                "--overflow" => {
                    index += 1;
                    parsed.overflow =
                        Some(require_long_flag_value(args, index, "--overflow")?.to_string());
                }
                "--bounds" => {
                    index += 1;
                    parsed.bounds =
                        Some(require_long_flag_value(args, index, "--bounds")?.to_string());
                }
                "--opt-level" => {
                    index += 1;
                    parsed.opt_level =
                        Some(require_long_flag_value(args, index, "--opt-level")?.to_string());
                }
                flag if flag.starts_with("-O") => {
                    parsed.opt_level = Some(flag[2..].to_string());
                }
                "--target" => {
                    index += 1;
                    parsed.target =
                        Some(require_long_flag_value(args, index, "--target")?.to_string());
                }
                "--kind" => {
                    index += 1;
                    parsed.kind = Some(require_long_flag_value(args, index, "--kind")?.to_string());
                }
                "--header" => {
                    index += 1;
                    parsed.header =
                        Some(require_long_flag_value(args, index, "--header")?.to_string());
                }
                "--print-pass-pipeline" => parsed.debug.print_pass_pipeline = true,
                "--print-mir-before-opt" => parsed.debug.print_mir_before_opt = true,
                "--print-mir-after-opt" => parsed.debug.print_mir_after_opt = true,
                flag if flag.starts_with("--") => {
                    index += 1;
                    let _ = require_long_flag_value(args, index, flag)?;
                }
                positional => parsed.positionals.push(positional.to_string()),
            }
            index += 1;
        }
        Ok(parsed)
    }
}

fn parse_opt_level_value(value: &str) -> Result<u8, String> {
    match value {
        "0" => Ok(0),
        "1" => Ok(1),
        "2" => Ok(2),
        "3" => Ok(3),
        other => Err(format!(
            "Invalid optimization level: {other}. Expected 0, 1, 2, or 3."
        )),
    }
}

fn parse_opt_level(args: &ParsedArgs) -> Result<u8, String> {
    args.opt_level
        .as_deref()
        .map_or(Ok(0), parse_opt_level_value)
}

fn parse_overflow_mode(args: &ParsedArgs) -> Result<OverflowMode, String> {
    match args.overflow.as_deref().unwrap_or("unchecked") {
        "unchecked" => Ok(OverflowMode::Unchecked),
        "checked" => Ok(OverflowMode::Checked),
        other => Err(format!(
            "Invalid value for --overflow: {other}. Expected 'unchecked' or 'checked'."
        )),
    }
}

fn parse_bounds_mode(args: &ParsedArgs) -> Result<BoundsMode, String> {
    match args.bounds.as_deref().unwrap_or("unchecked") {
        "unchecked" => Ok(BoundsMode::Unchecked),
        "checked" => Ok(BoundsMode::Checked),
        other => Err(format!(
            "Invalid value for --bounds: {other}. Expected 'unchecked' or 'checked'."
        )),
    }
}

fn mir_bounds_mode(bounds_mode: BoundsMode) -> MirPassBoundsMode {
    match bounds_mode {
        BoundsMode::Unchecked => MirPassBoundsMode::Unchecked,
        BoundsMode::Checked => MirPassBoundsMode::Checked,
    }
}

fn bounds_mode_name(bounds_mode: BoundsMode) -> &'static str {
    match bounds_mode {
        BoundsMode::Unchecked => "unchecked",
        BoundsMode::Checked => "checked",
    }
}

fn unsupported_checked_wasm_error() -> String {
    "error: WASM backend does not support --overflow checked yet.\n\
     help: use --overflow unchecked, or use emit-c/build for checked C output."
        .to_string()
}

fn unsupported_checked_llvm_error() -> String {
    "error: LLVM backend does not support --overflow checked yet.\n\
     Use --overflow unchecked, or use the C backend for checked arithmetic."
        .to_string()
}

fn unsupported_checked_wasm_bounds_error() -> String {
    "error: WASM backend does not support --bounds checked yet.\n\
     help: use --bounds unchecked, or use emit-c/build for checked C bounds."
        .to_string()
}

fn unsupported_checked_llvm_bounds_error() -> String {
    "error: LLVM backend does not support --bounds checked yet.\n\
     help: use --bounds unchecked, or use emit-c/build for checked C bounds."
        .to_string()
}

fn require_long_flag_value<'args>(
    args: &'args [String],
    index: usize,
    flag: &str,
) -> Result<&'args str, String> {
    let Some(value) = args.get(index).map(String::as_str) else {
        return Err(format!("Missing value for {flag}."));
    };
    if value.starts_with("--") {
        return Err(format!("Missing value for {flag}."));
    }
    Ok(value)
}

fn require_short_flag_value<'args>(
    args: &'args [String],
    index: usize,
    flag: &str,
) -> Result<&'args str, String> {
    let Some(value) = args.get(index).map(String::as_str) else {
        return Err(format!("Missing value for {flag}."));
    };
    if value.starts_with('-') {
        return Err(format!("Missing value for {flag}."));
    }
    Ok(value)
}

fn usage() -> &'static str {
    concat!(
        "Usage:\n",
        "  ckc check <file>\n",
        "  ckc emit-c <file> --out <c-file> [--header <h-file>] [--overflow <unchecked|checked>] [--bounds <unchecked|checked>] [--opt-level <0|1|2|3>]\n",
        "  ckc emit-mir <file> [--out <mir-file>] [--opt-level <0|1|2|3>]\n",
        "  ckc emit-llvm <file> [--out <ll-file>] [--target <triple>] [--overflow unchecked] [--bounds unchecked] [--opt-level <0|1|2|3>]\n",
        "  ckc emit-wat <file> [--out <wat-file>] [--overflow unchecked] [--bounds unchecked] [--opt-level <0|1|2|3>]\n",
        "  ckc emit-wasm <file> --out <wasm-file> [--overflow unchecked] [--bounds unchecked] [--opt-level <0|1|2|3>]\n",
        "  ckc build <file> --out <output-path> [--overflow <unchecked|checked>] [--bounds <unchecked|checked>] [--opt-level <0|1|2|3>]\n",
        "  ckc build-llvm <file> --out <output-path> [--kind <dynamic|object>] [--target <triple>] [--overflow unchecked] [--bounds unchecked] [--opt-level <0|1|2|3>]\n",
        "\n",
        "Options:\n",
        "  --overflow <unchecked|checked>    Arithmetic overflow handling mode. Default: unchecked.\n",
        "  --bounds <unchecked|checked>      Slice bounds mode. Default: unchecked; checked is C-only.\n",
        "  -o <file>                         Alias for --out <file>.\n",
        "  --opt-level <0|1|2|3>            MIR optimization level. Default: 0.\n",
        "  -O0, -O1, -O2, -O3              Alias for --opt-level.\n",
        "  --print-pass-pipeline           Print the selected MIR pass pipeline to stderr.\n",
        "  --print-mir-before-opt          Print MIR before optimization to stderr.\n",
        "  --print-mir-after-opt           Print MIR after optimization to stderr.\n",
    )
}

fn run_clang(args: &[String]) -> Result<(), String> {
    run_clang_with_missing_hint(args, None)
}

fn run_llvm_clang(args: &[String]) -> Result<(), String> {
    run_clang_with_missing_hint(
        args,
        Some("You can still run emit-llvm to generate LLVM IR without clang."),
    )
}

fn run_clang_with_missing_hint(args: &[String], missing_hint: Option<&str>) -> Result<(), String> {
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

fn missing_clang_message(hint: Option<&str>) -> String {
    let message = "clang was not found. Install clang and make sure it is available on PATH.";
    match hint {
        Some(hint) => format!("{message}\n{hint}"),
        None => message.to_string(),
    }
}

fn clang_shared_args(input: &std::path::Path, output: &std::path::Path) -> Vec<String> {
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

fn shared_library_output_path(path: &std::path::Path) -> PathBuf {
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

fn object_output_path(path: &std::path::Path) -> PathBuf {
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

fn llvm_intermediate_path(output_path: &std::path::Path, kind: &str) -> PathBuf {
    if kind == "object" {
        return output_path.with_extension("ll");
    }
    PathBuf::from(format!("{}.ll", output_path.display()))
}

fn detect_native_llvm_target_triple() -> Option<String> {
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

fn extract_llvm_target_triple(output: &str) -> Option<String> {
    let marker = "\"-triple\"";
    let after_marker = output.get(output.find(marker)? + marker.len()..)?;
    let first_quote = after_marker.find('"')?;
    let triple_start = first_quote + 1;
    let after_start = after_marker.get(triple_start..)?;
    let triple_end = after_start.find('"')?;
    Some(after_start[..triple_end].to_string())
}
