use std::{io, io::Write};

use calckernel::{
    BoundsMode, CheckedProgram, EffectTarget, EmitWasmOptions, KirBoundsMode, KirBuildConfig,
    KirConsumer, KirOptimizationLevel, KirOverflowMode, KirPassManagerResult, KirSanitizerMode,
    MemoryEffect, OverflowMode, SourceFile, annotate_unsafe_contracts, build_kir_module, check,
    emit_c_kir_header, emit_c_kir_module_with_contracts, emit_wasm_kir_module, emit_wat_kir_module,
    format_diagnostics, import_contract_facts, lower_to_mir, print_fact_arena, print_kir_module,
    print_mir_module, print_proof_arena, run_kir_pass_pipeline,
};

#[cfg(feature = "native-toolchain")]
use super::cache::{self, CacheKeyInput, CacheManifest};
use super::{args::*, output::*};
#[cfg(feature = "native-toolchain")]
use calckernel::{
    EmitLlvmOptions, NativeArtifactKind, NativeArtifactPaths, NativeContext, NativeCpu,
    NativeHeaderMode, NativeObject, NativeOptimizationLevel, NativePlatform, NativeTarget,
    create_native_static_archive, emit_native_header, link_native_dynamic_library,
    link_native_executable, lower_native_kir_module,
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
    let codegen_contract = format!(
        "kir-v1;strict-fp;entry-wrapper-v1;native-cpu;host-only;sanitizer-contracts={}",
        u8::from(args.sanitize_contracts)
    );
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
        kir_contract_version: 1,
        sanitizer_mode: u8::from(args.sanitize_contracts),
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
        kir_contract_version: key_input.kir_contract_version,
        sanitizer_mode: key_input.sanitizer_mode,
    };
    if let Some(object) = cache::load_object(&target, &key, args.no_cache) {
        return Ok(object);
    }

    let source = SourceFile::new(input, String::from_utf8_lossy(&source_bytes).into_owned());
    let checked = check(&source);
    if !checked.diagnostics.is_empty() {
        return Err(format_diagnostics(&source, &checked.diagnostics));
    }
    if checked.checked_program.entry.is_none() {
        return Err("ckc run requires fn main() -> void or i32".to_string());
    }
    let compiled = compile_kir(
        &checked.checked_program,
        KirConsumer::NativeExecutable,
        overflow_mode,
        bounds_mode,
        opt_level,
        args.sanitize_contracts,
        args,
    )?;
    let context = NativeContext::new().map_err(|error| error.to_string())?;
    let level = NativeOptimizationLevel::try_from(opt_level).map_err(|error| error.to_string())?;
    let module = lower_native_kir_module(
        &context,
        &target,
        &compiled.result,
        &EmitLlvmOptions {
            source_file_name: None,
            target_triple: None,
        },
    )
    .map_err(|error| error.to_string())?
    .verify()
    .map_err(|error| error.to_string())?
    .audit()
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
        "emit-kir" => run_emit_kir,
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

struct CompiledKir {
    result: KirPassManagerResult,
}

fn compile_kir(
    program: &CheckedProgram,
    consumer: KirConsumer,
    overflow_mode: OverflowMode,
    bounds_mode: BoundsMode,
    opt_level: u8,
    sanitize_contracts: bool,
    args: &ParsedArgs,
) -> Result<CompiledKir, String> {
    let semantic_mir = lower_to_mir(program).map_err(|error| error.to_string())?;
    let kir = build_kir_module(
        &semantic_mir,
        KirBuildConfig {
            consumer,
            overflow_mode: match overflow_mode {
                OverflowMode::Unchecked => KirOverflowMode::Unchecked,
                OverflowMode::Checked => KirOverflowMode::Checked,
            },
            bounds_mode: match bounds_mode {
                BoundsMode::Unchecked => KirBoundsMode::Unchecked,
                BoundsMode::Checked => KirBoundsMode::Checked,
            },
            sanitizer_mode: if sanitize_contracts {
                KirSanitizerMode::Contracts
            } else {
                KirSanitizerMode::Disabled
            },
        },
    )
    .map_err(|error| error.to_string())?;
    let contracts = import_contract_facts(&kir, program, 0).map_err(|error| error.to_string())?;
    let result = run_kir_pass_pipeline(
        kir,
        match opt_level {
            0 => KirOptimizationLevel::O0,
            1 => KirOptimizationLevel::O1,
            2 => KirOptimizationLevel::O2,
            3 => KirOptimizationLevel::O3,
            _ => return Err("optimization level is outside 0..=3".to_string()),
        },
        Some(&contracts),
    );
    if !result.errors.is_empty() {
        return Err(format!(
            "KIR verification failed: {}",
            result.errors.join("; ")
        ));
    }
    emit_kir_inspection(program, &result, args)?;
    Ok(CompiledKir { result })
}

fn emit_kir_inspection(
    program: &CheckedProgram,
    result: &KirPassManagerResult,
    args: &ParsedArgs,
) -> Result<(), String> {
    let mut sections = Vec::new();
    if args.print_facts {
        let facts = result.contract_facts.as_ref().map_or_else(
            || "facts generation=0\n".to_string(),
            |facts| print_fact_arena(facts.facts()),
        );
        sections.push(format!("===== KIR FACTS =====\n{facts}"));
        sections.push(format!(
            "===== KIR PROOFS =====\n{}",
            print_proof_arena(&result.proofs)
        ));
    }
    if args.print_effect_summaries {
        let mut text = String::from("===== EFFECT SUMMARIES =====\n");
        for (name, summary) in &program.effect_summaries {
            let accesses = summary
                .accesses()
                .map(|access| {
                    let target = match access.target {
                        EffectTarget::Parameter(index) => format!("param{index}"),
                        EffectTarget::All => "all".to_string(),
                    };
                    let effect = match access.effect {
                        MemoryEffect::None => "none",
                        MemoryEffect::Read => "read",
                        MemoryEffect::Write => "write",
                        MemoryEffect::ReadWrite => "readwrite",
                    };
                    format!("{target}:{effect}")
                })
                .collect::<Vec<_>>()
                .join(",");
            text.push_str(&format!(
                "{name} memory=[{accesses}] runtime={} may-fail={} unsafe-calls={} conservative={}\n",
                summary.runtime_effect,
                summary.may_fail,
                summary.unsafe_calls,
                summary.conservative
            ));
        }
        sections.push(text);
    }
    if args.explain_optimization {
        let mut text = String::from("===== OPTIMIZATION EXPLANATIONS =====\n");
        for explanation in &result.explanations {
            let trusted = result.eliminated_guards.iter().any(|elimination| {
                elimination.guard_instruction == explanation.guard_instruction
                    && elimination.used_trusted_contract
            });
            let proof = explanation.proof.map_or_else(
                || "none".to_string(),
                |proof| format!("proof{}", proof.index()),
            );
            text.push_str(&format!(
                "f{} b{} i{} removed={} proof={proof} trusted-contract={trusted} reason={}\n",
                explanation.function.index(),
                explanation.block.index(),
                explanation.guard_instruction.index(),
                explanation.removed,
                explanation.reason
            ));
        }
        sections.push(text);
    }
    if sections.is_empty() {
        return Ok(());
    }
    let mut stderr = io::stderr().lock();
    for section in sections {
        stderr
            .write_all(section.as_bytes())
            .and_then(|_| {
                if section.ends_with('\n') {
                    Ok(())
                } else {
                    stderr.write_all(b"\n")
                }
            })
            .map_err(|error| format!("failed to write KIR inspection output: {error}"))?;
    }
    Ok(())
}

pub(super) fn run_emit_kir(args: &ParsedArgs) -> Result<(), String> {
    let input = require_input(args, "emit-kir")?;
    let overflow_mode = parse_overflow_mode(args)?;
    let bounds_mode = parse_bounds_mode(args)?;
    let opt_level = parse_opt_level(args)?;
    let (source, checked) = check_file(input)?;
    if !checked.diagnostics.is_empty() {
        return Err(format_diagnostics(&source, &checked.diagnostics));
    }
    let compiled = compile_kir(
        &checked.checked_program,
        KirConsumer::Inspection,
        overflow_mode,
        bounds_mode,
        opt_level,
        false,
        args,
    )?;
    write_or_print(
        args.out.as_deref(),
        &print_kir_module(
            compiled
                .result
                .artifact
                .as_ref()
                .expect("verified KIR artifact"),
        ),
        "KIR",
    )
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
    let _opt_level = parse_opt_level(args)?;
    let (source, checked) = check_file(input)?;
    if !checked.diagnostics.is_empty() {
        return Err(format_diagnostics(&source, &checked.diagnostics));
    }
    let mir = lower_to_mir(&checked.checked_program).map_err(|error| error.to_string())?;
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
    let compiled = compile_kir(
        &checked.checked_program,
        KirConsumer::C,
        overflow_mode,
        bounds_mode,
        opt_level,
        false,
        args,
    )?;
    let kir = compiled
        .result
        .artifact
        .as_ref()
        .expect("verified C KIR artifact");
    let text = emit_c_kir_module_with_contracts(kir, compiled.result.contract_facts.as_ref())?;
    let header_text = annotate_unsafe_contracts(&emit_c_kir_header(kir), &checked.checked_program);
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
    let compiled = compile_kir(
        &checked.checked_program,
        KirConsumer::WebAssembly,
        overflow_mode,
        bounds_mode,
        opt_level,
        false,
        args,
    )?;
    let kir = compiled
        .result
        .artifact
        .as_ref()
        .expect("verified WebAssembly KIR artifact");
    write_or_print_single_line(
        args.out.as_deref(),
        &emit_wat_kir_module(kir, EmitWasmOptions { opt_level })?,
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
    let compiled = compile_kir(
        &checked.checked_program,
        KirConsumer::WebAssembly,
        overflow_mode,
        bounds_mode,
        opt_level,
        false,
        args,
    )?;
    let bytes = emit_wasm_kir_module(
        compiled
            .result
            .artifact
            .as_ref()
            .expect("verified WebAssembly KIR artifact"),
        EmitWasmOptions { opt_level },
    )?;
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
    let compiled = compile_kir(
        &checked.checked_program,
        KirConsumer::NativeLibrary,
        overflow_mode,
        bounds_mode,
        opt_level,
        false,
        args,
    )?;
    let context = NativeContext::new().map_err(|error| error.to_string())?;
    let target =
        NativeTarget::host_with_cpu(NativeCpu::Baseline).map_err(|error| error.to_string())?;
    let level = NativeOptimizationLevel::try_from(opt_level).map_err(|error| error.to_string())?;
    let text = lower_native_kir_module(
        &context,
        &target,
        &compiled.result,
        &EmitLlvmOptions {
            source_file_name: Some(input.to_string()),
            target_triple: args.target.clone(),
        },
    )
    .map_err(|error| error.to_string())?
    .verify()
    .map_err(|error| error.to_string())?
    .audit()
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
    if kind == ArtifactKind::Executable && checked.checked_program.entry.is_none() {
        return Err(
            "standalone native executable requires fn main() -> void or i32; no output was created"
                .to_string(),
        );
    }
    let consumer = if kind == ArtifactKind::Executable {
        KirConsumer::NativeExecutable
    } else {
        KirConsumer::NativeLibrary
    };
    let compiled = compile_kir(
        &checked.checked_program,
        consumer,
        overflow_mode,
        bounds_mode,
        opt_level,
        args.sanitize_contracts,
        args,
    )?;
    let cpu = match args.cpu.unwrap_or(CpuPolicy::Baseline) {
        CpuPolicy::Baseline => NativeCpu::Baseline,
        CpuPolicy::Native => NativeCpu::Native,
    };
    let context = NativeContext::new().map_err(|error| error.to_string())?;
    let target = NativeTarget::host_with_cpu(cpu).map_err(|error| error.to_string())?;
    let level = NativeOptimizationLevel::try_from(opt_level).map_err(|error| error.to_string())?;
    let lowered = lower_native_kir_module(
        &context,
        &target,
        &compiled.result,
        &EmitLlvmOptions {
            source_file_name: Some(input.to_string()),
            target_triple: args.target.clone(),
        },
    );
    let optimized = lowered
        .map_err(|error| error.to_string())?
        .verify()
        .map_err(|error| error.to_string())?
        .audit()
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
        annotate_unsafe_contracts(
            &emit_native_header(
                compiled
                    .result
                    .artifact
                    .as_ref()
                    .expect("verified Native KIR artifact"),
                if kind == ArtifactKind::Dynamic {
                    NativeHeaderMode::Dynamic
                } else {
                    NativeHeaderMode::StaticOrObject
                },
            ),
            &checked.checked_program,
        )
    });
    let exports = compiled
        .result
        .artifact
        .as_ref()
        .expect("verified Native KIR artifact")
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
