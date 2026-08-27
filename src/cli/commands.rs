use std::path::PathBuf;
use std::{io, io::Write};

use calckernel::{
    BoundsMode, EmitCOptions, EmitLlvmOptions, EmitWasmOptions, MirArtifactConsumer, MirModule,
    MirPassBoundsMode, MirPassContext, MirPassDebugFlags, MirPassOverflowMode,
    MirPassTargetBackend, OverflowMode, SourceFile, build_mir_optimization_pipeline, check,
    emit_c_header, emit_c_module_with_header, emit_llvm_module, emit_wasm_module_with_options,
    emit_wat_module_with_options, format_diagnostics, lower_to_mir,
    prepare_non_executable_artifact, print_mir_module, print_mir_pass_pipeline,
    run_mir_pass_pipeline,
};

use super::{args::*, output::*, toolchain::*};

pub(super) fn dispatch(command: &str, args: &[String]) -> Option<Result<(), String>> {
    #[cfg(not(feature = "native-toolchain"))]
    if matches!(command, "run" | "emit-llvm" | "build" | "build-llvm") {
        return Some(Err(native_unavailable_error()));
    }

    let run_command = match command {
        "check" => run_check,
        "emit-mir" => run_emit_mir,
        "emit-c" => run_emit_c,
        "emit-wat" => run_emit_wat,
        "emit-wasm" => run_emit_wasm,
        "emit-llvm" => run_emit_llvm,
        "build" => run_build,
        "build-llvm" => run_build_llvm,
        "licenses" => run_licenses,
        _ => return None,
    };
    Some(ParsedArgs::parse(args).and_then(|parsed| run_command(&parsed)))
}

pub(super) fn run_version(args: &[String]) -> Result<(), String> {
    if !args.is_empty() && args != ["--verbose"] {
        return Err("Usage: ckc --version [--verbose]".to_string());
    }

    println!("ckc {}", env!("CARGO_PKG_VERSION"));
    if args.is_empty() {
        return Ok(());
    }

    println!("Native ABI: {}", calckernel::NATIVE_ABI_VERSION);
    println!("Runtime ABI: {}", calckernel::RUNTIME_ABI_VERSION);
    #[cfg(feature = "native-toolchain")]
    {
        let info = calckernel::bridge_info().map_err(|error| error.to_string())?;
        let jit = calckernel::NativeJit::new().map_err(|error| error.to_string())?;
        println!("LLVM: {}", info.llvm_version);
        println!(
            "LLVM manifest SHA-256: {}",
            env!("CKC_LLVM_MANIFEST_SHA256")
        );
        println!("Target: {}", env!("CKC_BUILD_TARGET"));
        println!("Code generator: {}", code_generator_name());
        println!(
            "ORC object layer: {}",
            match jit.object_layer() {
                calckernel::OrcObjectLayer::JitLink => "JITLink",
                calckernel::OrcObjectLayer::RuntimeDyldCoffAarch64 => {
                    "RuntimeDyld (COFF AArch64)"
                }
            }
        );
    }
    #[cfg(not(feature = "native-toolchain"))]
    {
        println!("LLVM: unavailable (native-toolchain feature disabled)");
        println!("Target: {}", env!("CKC_BUILD_TARGET"));
        println!("Code generator: unavailable");
        println!("ORC object layer: unavailable");
    }
    Ok(())
}

pub(super) fn run_licenses(args: &ParsedArgs) -> Result<(), String> {
    if !args.positionals.is_empty() {
        return Err("Usage: ckc licenses".to_string());
    }

    let stdout = io::stdout();
    let mut output = stdout.lock();
    for notice in calckernel::embedded_notices() {
        writeln!(output, "===== {} =====", notice.name)
            .and_then(|_| output.write_all(notice.bytes))
            .and_then(|_| {
                if notice.bytes.ends_with(b"\n") {
                    writeln!(output)
                } else {
                    writeln!(output, "\n")
                }
            })
            .map_err(|error| format!("failed to write embedded notices: {error}"))?;
    }
    Ok(())
}

#[cfg(feature = "native-toolchain")]
fn code_generator_name() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "AArch64",
        "x86_64" => "X86",
        architecture => architecture,
    }
}

#[cfg(not(feature = "native-toolchain"))]
fn native_unavailable_error() -> String {
    "error: native toolchain unavailable: this developer build was compiled without the \
     'native-toolchain' feature"
        .to_string()
}

pub(super) fn run_check(args: &ParsedArgs) -> Result<(), String> {
    let input = require_input(args, "check")?;
    let (source, checked) = check_file(input)?;
    if !checked.diagnostics.is_empty() {
        return Err(format_diagnostics(&source, &checked.diagnostics));
    }
    println!("OK: {input}");
    Ok(())
}

pub(super) fn run_emit_mir(args: &ParsedArgs) -> Result<(), String> {
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

pub(super) fn run_emit_c(args: &ParsedArgs) -> Result<(), String> {
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
    let mir = prepare_non_executable_artifact(&mir, MirArtifactConsumer::C)
        .map_err(|error| error.to_string())?;
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

pub(super) fn run_emit_wat(args: &ParsedArgs) -> Result<(), String> {
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
    let mir = prepare_non_executable_artifact(&mir, MirArtifactConsumer::WebAssembly)
        .map_err(|error| error.to_string())?;
    write_or_print_single_line(
        args.out.as_deref(),
        &emit_wat_module_with_options(&mir, EmitWasmOptions { opt_level }),
        "WAT",
    )
}

pub(super) fn run_emit_wasm(args: &ParsedArgs) -> Result<(), String> {
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

pub(super) fn run_emit_llvm(args: &ParsedArgs) -> Result<(), String> {
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

pub(super) fn run_build(args: &ParsedArgs) -> Result<(), String> {
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
    let mir = prepare_non_executable_artifact(&mir, MirArtifactConsumer::C)
        .map_err(|error| error.to_string())?;
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

pub(super) fn run_build_llvm(args: &ParsedArgs) -> Result<(), String> {
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

pub(super) fn check_file(input: &str) -> Result<(SourceFile, calckernel::CheckResult), String> {
    validate_source_file_extension(input)?;
    let text = read_text_lossy(input)?;
    let source = SourceFile::new(input, text);
    let checked = check(&source);
    Ok((source, checked))
}

pub(super) fn validate_source_file_extension(input: &str) -> Result<(), String> {
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

pub(super) fn lower_and_optimize(
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
