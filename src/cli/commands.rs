use std::{io, io::Write};

use calckernel::{
    BoundsMode, EmitCOptions, EmitWasmOptions, MirArtifactConsumer, MirModule, MirPassBoundsMode,
    MirPassContext, MirPassDebugFlags, MirPassOverflowMode, MirPassTargetBackend, OverflowMode,
    SourceFile, build_mir_optimization_pipeline, check, emit_c_header, emit_c_module_with_header,
    emit_wasm_module_with_options, emit_wat_module_with_options, format_diagnostics, lower_to_mir,
    prepare_non_executable_artifact, print_mir_module, print_mir_pass_pipeline,
    run_mir_pass_pipeline,
};

#[cfg(feature = "native-toolchain")]
use super::cache::{self, CacheKeyInput, CacheManifest};
use super::{args::*, output::*};
#[cfg(feature = "native-toolchain")]
use calckernel::{
    EmitLlvmOptions, NativeArtifactKind, NativeArtifactPaths, NativeContext, NativeCpu,
    NativeHeaderMode, NativeLoweringOptions, NativeObject, NativeOptimizationLevel, NativePlatform,
    NativeTarget, create_native_static_archive, emit_native_header, link_native_dynamic_library,
    link_native_executable, lower_native_executable_module_with_options,
    lower_native_llvm_module_with_options,
};

#[cfg(feature = "native-toolchain")]
pub(super) fn compile_run_object(args: &ParsedArgs) -> Result<NativeObject, String> {
    let input = require_input(args, "run")?;
    validate_source_file_extension(input)?;
    let overflow_mode = parse_overflow_mode(args)?;
    let bounds_mode = parse_bounds_mode(args)?;
    let opt_level = parse_opt_level(args)?;
    let source_bytes = read_file_bytes(input)?;
    let target =
        NativeTarget::host_with_cpu(NativeCpu::Native).map_err(|error| error.to_string())?;
    let bridge = calckernel::bridge_info().map_err(|error| error.to_string())?;
    let target_triple = target.triple().map_err(|error| error.to_string())?;
    let cpu = target.cpu().map_err(|error| error.to_string())?;
    let features = target.features().map_err(|error| error.to_string())?;
    let codegen_contract = "strict-fp;entry-wrapper-v1;native-cpu;host-only".to_string();
    let key_input = CacheKeyInput {
        source: source_bytes.clone(),
        compiler_version: env!("CARGO_PKG_VERSION").to_string(),
        native_abi: calckernel::NATIVE_ABI_VERSION,
        runtime_abi: calckernel::RUNTIME_ABI_VERSION,
        bridge_abi: calckernel::LLVM_BRIDGE_ABI_VERSION,
        llvm_version: bridge.llvm_version.clone(),
        llvm_manifest_sha256: env!("CKC_LLVM_MANIFEST_SHA256").to_string(),
        target_triple: target_triple.clone(),
        optimization_level: opt_level,
        overflow_mode: mode_value(overflow_mode),
        bounds_mode: bounds_mode_value(bounds_mode),
        cpu: cpu.clone(),
        features: features.clone(),
        codegen_contract: codegen_contract.clone(),
        runtime_sha256: [
            env!("CKC_RUNTIME_SHA256_0").to_string(),
            env!("CKC_RUNTIME_SHA256_1").to_string(),
            env!("CKC_RUNTIME_SHA256_2").to_string(),
            env!("CKC_RUNTIME_SHA256_3").to_string(),
            env!("CKC_RUNTIME_SHA256_4").to_string(),
        ],
    };
    let key = cache::cache_key_hex(&key_input);
    let manifest = CacheManifest {
        key: key.clone(),
        compiler_version: key_input.compiler_version,
        llvm_version: key_input.llvm_version,
        target_triple,
        cpu,
        features,
        codegen_contract,
        native_abi: key_input.native_abi,
        runtime_abi: key_input.runtime_abi,
        bridge_abi: key_input.bridge_abi,
        optimization_level: opt_level,
        overflow_mode: key_input.overflow_mode,
        bounds_mode: key_input.bounds_mode,
    };
    if let Some(object) = cache::load_object(&target, &key, args.no_cache) {
        return Ok(object);
    }

    let source = SourceFile::new(input, String::from_utf8_lossy(&source_bytes).into_owned());
    let checked = check(&source);
    if !checked.diagnostics.is_empty() {
        return Err(format_diagnostics(&source, &checked.diagnostics));
    }
    let mir = lower_and_optimize(
        &checked.checked_program,
        opt_level,
        mir_overflow_mode(overflow_mode),
        mir_bounds_mode(bounds_mode),
        MirPassTargetBackend::Llvm,
        &args.debug,
    )?;
    if mir.entry.is_none() {
        return Err("ckc run requires fn main() -> void or i32".to_string());
    }
    let context = NativeContext::new().map_err(|error| error.to_string())?;
    let level = NativeOptimizationLevel::try_from(opt_level).map_err(|error| error.to_string())?;
    let module = lower_native_executable_module_with_options(
        &context,
        &target,
        &mir,
        &NativeLoweringOptions {
            emit: EmitLlvmOptions {
                source_file_name: None,
                target_triple: None,
            },
            overflow_mode,
            bounds_mode,
        },
    )
    .map_err(|error| error.to_string())?
    .verify()
    .map_err(|error| error.to_string())?
    .optimize(&target, level)
    .map_err(|error| error.to_string())?;
    let object = target
        .emit_object(module)
        .map_err(|error| error.to_string())?;
    cache::store_object(&manifest, &object, args.no_cache);
    Ok(object)
}

#[cfg(feature = "native-toolchain")]
const fn mode_value(mode: OverflowMode) -> u8 {
    match mode {
        OverflowMode::Unchecked => 0,
        OverflowMode::Checked => 1,
    }
}

#[cfg(feature = "native-toolchain")]
const fn bounds_mode_value(mode: BoundsMode) -> u8 {
    match mode {
        BoundsMode::Unchecked => 0,
        BoundsMode::Checked => 1,
    }
}

pub(super) fn dispatch(command: &str, args: &[String]) -> Option<Result<(), String>> {
    #[cfg(not(feature = "native-toolchain"))]
    if matches!(
        command,
        "run" | "cache" | "emit-llvm" | "build" | "build-llvm"
    ) {
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
        "cache" => run_cache,
        "licenses" => run_licenses,
        _ => return None,
    };
    Some(ParsedArgs::parse(command, args).and_then(|parsed| run_command(&parsed)))
}

#[cfg(feature = "native-toolchain")]
pub(super) fn run_cache(args: &ParsedArgs) -> Result<(), String> {
    if args.positionals != ["clean"] {
        return Err("Usage: ckc cache clean".to_string());
    }
    cache::clean_default()?;
    println!("OK: native cache cleaned");
    Ok(())
}

#[cfg(not(feature = "native-toolchain"))]
pub(super) fn run_cache(_args: &ParsedArgs) -> Result<(), String> {
    Err(native_unavailable_error())
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

#[cfg(feature = "native-toolchain")]
pub(super) fn run_emit_llvm(args: &ParsedArgs) -> Result<(), String> {
    let input = require_input(args, "emit-llvm")?;
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
        mir_overflow_mode(overflow_mode),
        mir_bounds_mode(bounds_mode),
        MirPassTargetBackend::Llvm,
        &args.debug,
    )?;
    let context = NativeContext::new().map_err(|error| error.to_string())?;
    let target =
        NativeTarget::host_with_cpu(NativeCpu::Baseline).map_err(|error| error.to_string())?;
    let level = NativeOptimizationLevel::try_from(opt_level).map_err(|error| error.to_string())?;
    let text = lower_native_llvm_module_with_options(
        &context,
        &target,
        &mir,
        &NativeLoweringOptions {
            emit: EmitLlvmOptions {
                source_file_name: Some(input.to_string()),
                target_triple: args.target.clone(),
            },
            overflow_mode,
            bounds_mode,
        },
    )
    .map_err(|error| error.to_string())?
    .verify()
    .map_err(|error| error.to_string())?
    .optimize(&target, level)
    .map_err(|error| error.to_string())?
    .to_ir_string()
    .map_err(|error| error.to_string())?;
    write_or_print_single_line(args.out.as_deref(), &text, "LLVM IR")
}

#[cfg(feature = "native-toolchain")]
pub(super) fn run_build(args: &ParsedArgs) -> Result<(), String> {
    let input = require_input(args, "build")?;
    let out = require_out(args, "build")?;
    let kind = args.kind.unwrap_or(ArtifactKind::Dynamic);
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
        mir_overflow_mode(overflow_mode),
        mir_bounds_mode(bounds_mode),
        MirPassTargetBackend::Llvm,
        &args.debug,
    )?;
    let mir = if kind == ArtifactKind::Executable {
        if mir.entry.is_none() {
            return Err(
                "standalone native executable requires fn main() -> void or i32; no output was created"
                    .to_string(),
            );
        }
        mir
    } else {
        prepare_non_executable_artifact(&mir, MirArtifactConsumer::NativeLibrary)
            .map_err(|error| error.to_string())?
    };
    let cpu = match args.cpu.unwrap_or(CpuPolicy::Baseline) {
        CpuPolicy::Baseline => NativeCpu::Baseline,
        CpuPolicy::Native => NativeCpu::Native,
    };
    let context = NativeContext::new().map_err(|error| error.to_string())?;
    let target = NativeTarget::host_with_cpu(cpu).map_err(|error| error.to_string())?;
    let level = NativeOptimizationLevel::try_from(opt_level).map_err(|error| error.to_string())?;
    let lowering_options = NativeLoweringOptions {
        emit: EmitLlvmOptions {
            source_file_name: Some(input.to_string()),
            target_triple: args.target.clone(),
        },
        overflow_mode,
        bounds_mode,
    };
    let lowered = if kind == ArtifactKind::Executable {
        lower_native_executable_module_with_options(&context, &target, &mir, &lowering_options)
    } else {
        lower_native_llvm_module_with_options(&context, &target, &mir, &lowering_options)
    };
    let optimized = lowered
        .map_err(|error| error.to_string())?
        .verify()
        .map_err(|error| error.to_string())?
        .optimize(&target, level)
        .map_err(|error| error.to_string())?;
    let object = target
        .emit_object(optimized)
        .map_err(|error| error.to_string())?;
    let artifact_kind = match kind {
        ArtifactKind::Executable => NativeArtifactKind::Executable,
        ArtifactKind::Dynamic => NativeArtifactKind::Dynamic,
        ArtifactKind::Static => NativeArtifactKind::Static,
        ArtifactKind::Object => NativeArtifactKind::Object,
    };
    let paths = NativeArtifactPaths::new(NativePlatform::host(), artifact_kind, &absolutize(out));
    let header = (!matches!(kind, ArtifactKind::Executable)).then(|| {
        emit_native_header(
            &mir,
            EmitCOptions {
                overflow_mode,
                bounds_mode,
                opt_level,
            },
            if kind == ArtifactKind::Dynamic {
                NativeHeaderMode::Dynamic
            } else {
                NativeHeaderMode::StaticOrObject
            },
        )
    });
    let exports = mir
        .functions
        .iter()
        .filter(|function| function.exported)
        .map(|function| function.name.clone())
        .collect::<Vec<_>>();
    let (primary, import_library) = match kind {
        ArtifactKind::Executable => (
            link_native_executable(&object)
                .map_err(|error| error.to_string())?
                .as_bytes()
                .to_vec(),
            None,
        ),
        ArtifactKind::Object => (object.as_bytes().to_vec(), None),
        ArtifactKind::Static => (
            create_native_static_archive(&object)
                .map_err(|error| error.to_string())?
                .as_bytes()
                .to_vec(),
            None,
        ),
        ArtifactKind::Dynamic => {
            let library = link_native_dynamic_library(&object, &exports)
                .map_err(|error| error.to_string())?;
            (
                library.as_bytes().to_vec(),
                library.import_library().map(<[u8]>::to_vec),
            )
        }
    };
    let mut transaction = OutputTransaction::new();
    if kind == ArtifactKind::Executable {
        transaction.stage_executable(paths.primary.clone(), &primary)?;
    } else {
        transaction.stage(paths.primary.clone(), &primary)?;
    }
    if let (Some(path), Some(header)) = (&paths.header, header.as_deref()) {
        transaction.stage(path.clone(), header.as_bytes())?;
    }
    if let (Some(path), Some(bytes)) = (&paths.import_library, import_library.as_deref()) {
        transaction.stage(path.clone(), bytes)?;
    }
    transaction.commit()?;
    println!("OK: built native {}", artifact_kind_name(artifact_kind));
    println!("{}", paths.primary.display());
    if let Some(path) = paths.header {
        println!("{}", path.display());
    }
    if let Some(path) = paths.import_library {
        println!("{}", path.display());
    }
    Ok(())
}

#[cfg(feature = "native-toolchain")]
const fn artifact_kind_name(kind: NativeArtifactKind) -> &'static str {
    match kind {
        NativeArtifactKind::Executable => "executable",
        NativeArtifactKind::Dynamic => "dynamic library",
        NativeArtifactKind::Static => "static library",
        NativeArtifactKind::Object => "object",
    }
}

#[cfg(feature = "native-toolchain")]
pub(super) fn run_build_llvm(args: &ParsedArgs) -> Result<(), String> {
    let _ = require_input(args, "build-llvm")?;
    let _ = require_out(args, "build-llvm")?;
    let kind = args.kind.unwrap_or(ArtifactKind::Dynamic);
    if !matches!(kind, ArtifactKind::Dynamic | ArtifactKind::Object) {
        return Err("build-llvm supports only --kind dynamic or object.".to_string());
    }
    eprintln!("warning: 'build-llvm' is deprecated; use 'build' instead");
    run_build(args)
}

#[cfg(not(feature = "native-toolchain"))]
pub(super) fn run_emit_llvm(_args: &ParsedArgs) -> Result<(), String> {
    Err(native_unavailable_error())
}

#[cfg(not(feature = "native-toolchain"))]
pub(super) fn run_build(_args: &ParsedArgs) -> Result<(), String> {
    Err(native_unavailable_error())
}

#[cfg(not(feature = "native-toolchain"))]
pub(super) fn run_build_llvm(_args: &ParsedArgs) -> Result<(), String> {
    Err(native_unavailable_error())
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
