use std::{io, io::Write};

#[cfg(feature = "native-toolchain")]
use std::collections::BTreeSet;

use calckernel::{
    BoundsMode, CheckedProgram, EffectTarget, EmitWasmOptions, KirBoundsMode, KirBuildConfig,
    KirConsumer, KirOptimizationLevel, KirOverflowMode, KirPassManagerResult, KirSanitizerMode,
    KirTargetProfile, KirVerifiedProgramState, MemoryEffect, OverflowMode, SourceFile,
    annotate_unsafe_contracts, build_kir_module, build_kir_module_with_profile, check,
    emit_c_kir_header, emit_c_kir_module_with_contracts, emit_wasm_kir_module, emit_wat_kir_module,
    format_diagnostics, import_contract_facts, lower_to_mir, prepare_kir_pre_tune_state,
    print_fact_arena, print_kir_module, print_mir_module, print_optimization_audit,
    print_proof_arena, run_kir_pass_pipeline,
};

#[cfg(feature = "native-toolchain")]
use super::cache::{self, CacheKeyInput, CacheManifest};
use super::{args::*, output::*};
#[cfg(feature = "native-toolchain")]
use calckernel::{
    CkCompilerProfileIdentity, CkImmutableProfileAnalysis, CkLateProfileLayoutPlan,
    CkLateProfileLayoutReport, CkModuleProfileIdentity, CkProfileAnalysis, CkProfileContract,
    CkProfileCpuPolicy, CkProfileEndianness, CkProfileEvent, CkProfileIdentity, CkProfileKirMode,
    CkProfileModes, CkProfileObjectFormat, CkProfileObservation, CkProfileOptimizationFamily,
    CkProfileSchemaIdentity, CkProfileTargetIdentity, CkProfileTopology, CkProfileWorkTerm,
    EmitLlvmOptions, KirMultiversionPlanningRequest, NATIVE_PROFILE_RUNTIME_SHA256,
    NativeArtifactKind, NativeArtifactPaths, NativeContext, NativeCpu, NativeHeaderMode,
    NativeMultiversionTargetSet, NativeObject, NativeOptimizationLevel, NativePlatform,
    NativeProfileGeneration, NativeTarget, VerifiedNativeBuild, anchor_profile_directory,
    apply_profile, build_late_profile_layout_plan, build_verified_native_artifact,
    check_kir_multiversion_bundle, create_native_multiversion_static_archive,
    create_native_profile_generation_static_archive, emit_native_header,
    emit_native_multiversion_objects, emit_native_profile_generation_header,
    link_native_multiversion_dynamic_library, link_native_multiversion_executable,
    link_native_profile_generation_dynamic_library, link_native_profile_generation_executable,
    lower_native_kir_module, lower_native_profile_generation_module,
    pass_result_from_verified_tuning_state, prepare_ck_profile_kir, print_kir_multiversion_bundle,
    propose_kir_multiversion_bundle, read_profile_input, run_profile_guided_kir_pass_pipeline,
    validate_profile_analysis_for_optimizer,
};
#[cfg(feature = "native-toolchain")]
use sha2::{Digest, Sha256};

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
    let profile = target
        .kir_profile(KirConsumer::NativeExecutable)
        .map_err(|error| error.to_string())?;
    let bridge = calckernel::bridge_info().map_err(|error| error.to_string())?;
    let target_triple = target.triple().map_err(|error| error.to_string())?;
    let cpu = target.cpu().map_err(|error| error.to_string())?;
    let features = target.features().map_err(|error| error.to_string())?;
    let codegen_contract = format!(
        "kir-v3;strict-fp;entry-wrapper-v1;native-cpu;host-only;sanitizer-contracts={}",
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
        kir_contract_version: 2,
        sanitizer_mode: u8::from(args.sanitize_contracts),
        target_profile_digest: profile.digest_hex(),
        vector_cost_model_schema: calckernel::KIR_VECTOR_COST_MODEL_SCHEMA,
        vector_proof_schema: calckernel::KIR_VECTOR_PROOF_SCHEMA,
        vector_budget_identity: calckernel::kir_vector_budget_identity().to_string(),
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
        profile_identity: "mode=off;format=1;contract=1;digest=none".to_string(),
        artifact_identity: "kind=jit-object;topology=native-executable".to_string(),
        pgo_identity: "mode=off;confidence=1;site=3;cost=3".to_string(),
        multiversion_identity: "policy=native;target-set=single;variants=none".to_string(),
        dispatch_identity: format!(
            "table=none;detector=1;thunk=1;runtime={}",
            calckernel::NATIVE_DISPATCH_RUNTIME_SHA256
        ),
        budget_identity: format!(
            "vector={};multiversion=1;growth=none",
            calckernel::kir_vector_budget_identity()
        ),
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
        target_profile_digest: key_input.target_profile_digest,
        vector_cost_model_schema: key_input.vector_cost_model_schema,
        vector_proof_schema: key_input.vector_proof_schema,
        vector_budget_identity: key_input.vector_budget_identity,
        profile_identity: key_input.profile_identity,
        artifact_identity: key_input.artifact_identity,
        pgo_identity: key_input.pgo_identity,
        multiversion_identity: key_input.multiversion_identity,
        dispatch_identity: key_input.dispatch_identity,
        budget_identity: key_input.budget_identity,
    };
    if let Some(object) = cache::load_object(&target, &manifest, args.no_cache) {
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
        KirCompilationTarget {
            consumer: KirConsumer::NativeExecutable,
            profile: Some(profile),
        },
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
    #[cfg_attr(not(feature = "native-toolchain"), allow(dead_code))]
    pre_tune: Option<KirVerifiedProgramState>,
}

struct KirCompilationTarget {
    consumer: KirConsumer,
    profile: Option<KirTargetProfile>,
}

fn compile_kir(
    program: &CheckedProgram,
    target: KirCompilationTarget,
    overflow_mode: OverflowMode,
    bounds_mode: BoundsMode,
    opt_level: u8,
    sanitize_contracts: bool,
    args: &ParsedArgs,
) -> Result<CompiledKir, String> {
    let semantic_mir = lower_to_mir(program).map_err(|error| error.to_string())?;
    let config = KirBuildConfig {
        consumer: target.consumer,
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
    };
    let kir = match target.profile {
        Some(profile) => build_kir_module_with_profile(&semantic_mir, config, profile),
        None => build_kir_module(&semantic_mir, config),
    }
    .map_err(|error| error.to_string())?;
    let contracts = import_contract_facts(&kir, program, 0).map_err(|error| error.to_string())?;
    let level = match opt_level {
        0 => KirOptimizationLevel::O0,
        1 => KirOptimizationLevel::O1,
        2 => KirOptimizationLevel::O2,
        3 => KirOptimizationLevel::O3,
        _ => return Err("optimization level is outside 0..=3".to_string()),
    };
    let pre_tune = (level == KirOptimizationLevel::O3
        && matches!(
            target.consumer,
            KirConsumer::NativeLibrary | KirConsumer::NativeExecutable
        ))
    .then(|| prepare_kir_pre_tune_state(kir.clone(), Some(&contracts)))
    .transpose()?;
    let result = run_kir_pass_pipeline(kir, level, Some(&contracts));
    if !result.errors.is_empty() {
        return Err(format!(
            "KIR verification failed: {}",
            result.errors.join("; ")
        ));
    }
    emit_kir_inspection(program, &result, args)?;
    Ok(CompiledKir { result, pre_tune })
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
        for fallback in &result.analysis_fallbacks {
            text.push_str(&format!(
                "f{} pass={} reason={}\n",
                fallback.function.index(),
                fallback.pass,
                fallback.reason
            ));
        }
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
        text.push_str(&print_optimization_audit(&result.audit));
        for explanation in &result.vector_explanations {
            text.push_str(&explanation.stable_text());
            text.push('\n');
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
    let consumer = args
        .consumer
        .unwrap_or(EmitKirConsumer::Inspection)
        .kir_consumer();
    #[cfg(not(feature = "native-toolchain"))]
    if matches!(
        consumer,
        KirConsumer::NativeLibrary | KirConsumer::NativeExecutable
    ) {
        return Err(native_unavailable_error());
    }
    let overflow_mode = parse_overflow_mode(args)?;
    let bounds_mode = parse_bounds_mode(args)?;
    let opt_level = parse_opt_level(args)?;
    let (source, checked) = check_file(input)?;
    if !checked.diagnostics.is_empty() {
        return Err(format_diagnostics(&source, &checked.diagnostics));
    }
    #[cfg(feature = "native-toolchain")]
    let multiversion_targets = if matches!(
        consumer,
        KirConsumer::NativeLibrary | KirConsumer::NativeExecutable
    ) && args.cpu == Some(CpuPolicy::Multiversion)
    {
        Some(NativeMultiversionTargetSet::host(consumer).map_err(|error| error.to_string())?)
    } else {
        None
    };
    #[cfg(feature = "native-toolchain")]
    let multiversion_application = if args.pgo_use.is_some() {
        multiversion_targets
            .as_ref()
            .map(|targets| {
                let baseline = targets
                    .target(calckernel::KirMultiversionTierId::Baseline)
                    .ok_or_else(|| {
                        "multiversion target set is missing its baseline target".to_string()
                    })?;
                prepare_profile_application(
                    &checked.checked_program,
                    args,
                    ProfileApplicationRequest {
                        target: baseline,
                        consumer,
                        overflow_mode,
                        bounds_mode,
                        opt_level,
                        cpu_policy: CpuPolicy::Multiversion,
                    },
                )
            })
            .transpose()?
    } else {
        None
    };
    #[cfg(feature = "native-toolchain")]
    let profile = if matches!(
        consumer,
        KirConsumer::NativeLibrary | KirConsumer::NativeExecutable
    ) {
        let cpu = match args.cpu.unwrap_or(CpuPolicy::Baseline) {
            CpuPolicy::Baseline => NativeCpu::Baseline,
            CpuPolicy::Native => NativeCpu::Native,
            CpuPolicy::Multiversion => NativeCpu::Multiversion,
        };
        if let Some(targets) = &multiversion_targets {
            Some(targets.target_set().tiers[0].profile.clone())
        } else {
            let target = NativeTarget::host_with_cpu(cpu).map_err(|error| error.to_string())?;
            Some(
                target
                    .kir_profile(consumer)
                    .map_err(|error| error.to_string())?,
            )
        }
    } else {
        None
    };
    #[cfg(not(feature = "native-toolchain"))]
    let profile = None;
    #[cfg(feature = "native-toolchain")]
    let compiled = if let Some(application) = &multiversion_application {
        compile_profile_guided_kir(&checked.checked_program, application, args)?
    } else {
        compile_kir(
            &checked.checked_program,
            KirCompilationTarget { consumer, profile },
            overflow_mode,
            bounds_mode,
            opt_level,
            false,
            args,
        )?
    };
    #[cfg(not(feature = "native-toolchain"))]
    let compiled = compile_kir(
        &checked.checked_program,
        KirCompilationTarget { consumer, profile },
        overflow_mode,
        bounds_mode,
        opt_level,
        false,
        args,
    )?;
    #[cfg(feature = "native-toolchain")]
    if let Some(targets) = &multiversion_targets {
        if let Some(application) = &multiversion_application {
            emit_profile_analysis(&application.analysis, args)?;
        }
        emit_pgo_optimizer_report(&compiled.result, args)?;
        let pgo_hot_roots = compiled.result.pgo.as_ref().map(|pgo| {
            pgo.functions
                .iter()
                .filter(|function| function.hot)
                .map(|function| function.function)
                .collect::<BTreeSet<_>>()
        });
        let request = KirMultiversionPlanningRequest {
            logical_pre_state: compiled
                .result
                .artifact
                .clone()
                .expect("verified KIR artifact"),
            target_set: targets.target_set().clone(),
            pgo_hot_roots,
            shared_growth_consumed: 0,
        };
        let bundle = propose_kir_multiversion_bundle(&request)?;
        check_kir_multiversion_bundle(&request, &bundle)?;
        return write_or_print(
            args.out.as_deref(),
            &print_kir_multiversion_bundle(&bundle),
            "multiversion KIR bundle",
        );
    }
    #[cfg(feature = "native-toolchain")]
    if args.pgo_use.is_some() {
        let cpu_policy = args.cpu.unwrap_or(CpuPolicy::Baseline);
        let cpu = match cpu_policy {
            CpuPolicy::Baseline | CpuPolicy::Multiversion => NativeCpu::Baseline,
            CpuPolicy::Native => NativeCpu::Native,
        };
        let target = NativeTarget::host_with_cpu(cpu).map_err(|error| error.to_string())?;
        let analysis = prepare_profile_application(
            &checked.checked_program,
            args,
            ProfileApplicationRequest {
                target: &target,
                consumer,
                overflow_mode,
                bounds_mode,
                opt_level,
                cpu_policy,
            },
        )?;
        emit_profile_analysis(&analysis.analysis, args)?;
    }
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
    println!("KIR: {}", calckernel::KIR_FORMAT_VERSION);
    println!(
        "CK profile: CKPART01/CKPROF01 schema {}",
        calckernel::CK_PROFILE_FORMAT_SCHEMA
    );
    println!(
        "Native cache: {} key schema {} manifest schema {}",
        calckernel::NATIVE_CACHE_ENTRY_MAGIC,
        calckernel::NATIVE_CACHE_KEY_SCHEMA,
        calckernel::NATIVE_CACHE_MANIFEST_SCHEMA
    );
    println!(
        "CK tuning: CKTUNE01 decision {} manifest {} measurement {} inspection {} plan {}",
        calckernel::TUNE_DECISION_SCHEMA,
        calckernel::TUNE_MANIFEST_SCHEMA,
        calckernel::TUNE_MEASUREMENT_SCHEMA,
        calckernel::TUNE_INSPECTION_SCHEMA,
        calckernel::TUNE_PLAN_SCHEMA
    );
    #[cfg(feature = "native-toolchain")]
    {
        let info = calckernel::bridge_info().map_err(|error| error.to_string())?;
        let jit = calckernel::NativeJit::new().map_err(|error| error.to_string())?;
        println!("LLVM: {}", info.llvm_version);
        println!("LLVM bridge ABI: {}", calckernel::LLVM_BRIDGE_ABI_VERSION);
        println!(
            "Multiversion target set schema: {}",
            calckernel::KIR_MULTIVERSION_TARGET_SET_SCHEMA
        );
        println!(
            "Profile runtime schema: {}",
            calckernel::NATIVE_PROFILE_RUNTIME_SCHEMA
        );
        println!(
            "Dispatch runtime schema: {}",
            calckernel::NATIVE_DISPATCH_RUNTIME_SCHEMA
        );
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
pub(super) fn native_unavailable_error() -> String {
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
        KirCompilationTarget {
            consumer: KirConsumer::C,
            profile: None,
        },
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
        KirCompilationTarget {
            consumer: KirConsumer::WebAssembly,
            profile: None,
        },
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
        KirCompilationTarget {
            consumer: KirConsumer::WebAssembly,
            profile: None,
        },
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
    let target =
        NativeTarget::host_with_cpu(NativeCpu::Baseline).map_err(|error| error.to_string())?;
    let compiled = compile_kir(
        &checked.checked_program,
        KirCompilationTarget {
            consumer: KirConsumer::NativeLibrary,
            profile: Some(
                target
                    .kir_profile(KirConsumer::NativeLibrary)
                    .map_err(|error| error.to_string())?,
            ),
        },
        overflow_mode,
        bounds_mode,
        opt_level,
        false,
        args,
    )?;
    let context = NativeContext::new().map_err(|error| error.to_string())?;
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
pub(super) struct NativeBuildProduct {
    pub(super) paths: NativeArtifactPaths,
    pub(super) artifact_kind: NativeArtifactKind,
    pub(super) build: VerifiedNativeBuild,
    pub(super) state: KirVerifiedProgramState,
    pub(super) source_digest: [u8; 32],
    pub(super) semantic_contract_digest: [u8; 32],
    pub(super) compilation_mode_digest: [u8; 32],
    pub(super) target_triple: String,
    pub(super) target_cpu: String,
    pub(super) target_features: Vec<String>,
    pub(super) target_profile: String,
    pub(super) profile_digest: Option<[u8; 32]>,
    pub(super) pgo: Option<calckernel::CkPgoOptimizerPlan>,
    pub(super) source_file_name: String,
}

#[cfg(feature = "native-toolchain")]
pub(super) fn compile_verified_native_product(
    args: &ParsedArgs,
) -> Result<NativeBuildProduct, String> {
    let input = require_input(args, "build")?;
    let out = require_out(args, "build")?;
    if args.pgo_generate.is_some() {
        return Err("profile-generation builds are not verified tuning products".to_string());
    }
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
    let cpu_policy = args.cpu.unwrap_or(CpuPolicy::Baseline);
    if cpu_policy == CpuPolicy::Multiversion {
        return Err("multiversion builds are not single verified tuning products".to_string());
    }
    let cpu = match cpu_policy {
        CpuPolicy::Baseline => NativeCpu::Baseline,
        CpuPolicy::Native => NativeCpu::Native,
        CpuPolicy::Multiversion => unreachable!("handled by bundle planner"),
    };
    let target = NativeTarget::host_with_cpu(cpu).map_err(|error| error.to_string())?;
    let profile_application = args
        .pgo_use
        .as_ref()
        .map(|_| {
            prepare_profile_application(
                &checked.checked_program,
                args,
                ProfileApplicationRequest {
                    target: &target,
                    consumer,
                    overflow_mode,
                    bounds_mode,
                    opt_level,
                    cpu_policy,
                },
            )
        })
        .transpose()?;
    let compiled = if let Some(application) = profile_application.as_ref()
        && opt_level == 3
    {
        compile_profile_guided_kir(&checked.checked_program, application, args)?
    } else {
        compile_kir(
            &checked.checked_program,
            KirCompilationTarget {
                consumer,
                profile: Some(
                    target
                        .kir_profile(consumer)
                        .map_err(|error| error.to_string())?,
                ),
            },
            overflow_mode,
            bounds_mode,
            opt_level,
            args.sanitize_contracts,
            args,
        )?
    };
    if let Some(application) = &profile_application {
        emit_profile_analysis(&application.analysis, args)?;
    }
    emit_pgo_optimizer_report(&compiled.result, args)?;
    let context = NativeContext::new().map_err(|error| error.to_string())?;
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
    let optimized = if level == NativeOptimizationLevel::O2 {
        if let Some(application) = &profile_application {
            let plan = late_layout_plan_for_optimized_kir(&compiled, application, &optimized)?;
            if plan.functions.is_empty() {
                let report = optimized
                    .late_layout_snapshot(&target)
                    .map_err(|error| error.to_string())?;
                emit_late_layout_report(&report, "mapping-unavailable", args)?;
                optimized
            } else {
                let (optimized, report) = optimized
                    .apply_late_profile_layout(&target, &plan)
                    .map_err(|error| error.to_string())?;
                emit_late_layout_report(&report, &report.reason, args)?;
                optimized
            }
        } else {
            optimized
        }
    } else {
        optimized
    };
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
    let verified_build = build_verified_native_artifact(
        artifact_kind,
        &object,
        &exports,
        header.as_deref().map(str::as_bytes).map(<[u8]>::to_vec),
    )
    .map_err(|error| error.to_string())?;
    let state = if let Some(pre_tune) = compiled.pre_tune.clone() {
        pre_tune
    } else {
        KirVerifiedProgramState::from_parts(
            compiled
                .result
                .artifact
                .as_ref()
                .expect("verified Native KIR artifact")
                .clone(),
            compiled.result.contract_facts.clone(),
            compiled.result.proofs.clone(),
            compiled.result.eliminated_guards.clone(),
            0,
        )?
    };
    let source_bytes = read_file_bytes(input)?;
    let source_digest = Sha256::digest(&source_bytes).into();
    let semantic_contract_digest = digest_tune_parts(
        b"CK-TUNE-SEMANTIC-CONTRACT\0",
        &[
            &[mode_value(overflow_mode)],
            &[bounds_mode_value(bounds_mode)],
            b"strict-f64",
        ],
    );
    let compilation_mode_digest = digest_tune_parts(
        b"CK-TUNE-COMPILATION-MODE\0",
        &[
            &[opt_level],
            &[mode_value(overflow_mode)],
            &[bounds_mode_value(bounds_mode)],
            &[match artifact_kind {
                NativeArtifactKind::Executable => 1,
                NativeArtifactKind::Dynamic => 2,
                NativeArtifactKind::Static => 3,
                NativeArtifactKind::Object => 4,
            }],
            b"cpu-native",
        ],
    );
    let target_triple = target.triple().map_err(|error| error.to_string())?;
    if args
        .target
        .as_deref()
        .is_some_and(|requested| requested != target_triple)
    {
        return Err("offline tuning target must equal the exact host triple".to_string());
    }
    let target_cpu = target.cpu().map_err(|error| error.to_string())?;
    let mut target_features = target
        .features()
        .map_err(|error| error.to_string())?
        .split(',')
        .filter(|feature| !feature.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    target_features.sort();
    target_features.dedup();
    let target_profile = state.module().profile.digest_hex();
    let profile_digest = args
        .pgo_use
        .as_deref()
        .map(read_file_bytes)
        .transpose()?
        .map(|bytes| Sha256::digest(bytes).into());
    Ok(NativeBuildProduct {
        paths,
        artifact_kind,
        build: verified_build,
        state,
        source_digest,
        semantic_contract_digest,
        compilation_mode_digest,
        target_triple,
        target_cpu,
        target_features,
        target_profile,
        profile_digest,
        pgo: compiled.result.pgo.clone(),
        source_file_name: input.to_string(),
    })
}

/// Lowers and links one independently replayed KIR state through the exact same
/// LLVM O3/object/embedded-linker path as the ordinary baseline.  It returns a
/// frozen build value and performs no publication or production-cache write.
#[cfg(feature = "native-toolchain")]
pub(super) fn compile_replayed_native_build(
    product: &NativeBuildProduct,
    state: &KirVerifiedProgramState,
) -> Result<VerifiedNativeBuild, String> {
    let target =
        NativeTarget::host_with_cpu(NativeCpu::Native).map_err(|error| error.to_string())?;
    let pgo = product
        .pgo
        .as_ref()
        .map(|plan| calckernel::project_pgo_plan_for_kir(state.module(), plan))
        .transpose()?;
    let result = pass_result_from_verified_tuning_state(state, pgo)?;
    let context = NativeContext::new().map_err(|error| error.to_string())?;
    let optimized = lower_native_kir_module(
        &context,
        &target,
        &result,
        &EmitLlvmOptions {
            source_file_name: Some(product.source_file_name.clone()),
            target_triple: Some(product.target_triple.clone()),
        },
    )
    .map_err(|error| error.to_string())?
    .verify()
    .map_err(|error| error.to_string())?
    .audit()
    .map_err(|error| error.to_string())?
    .optimize(&target, NativeOptimizationLevel::O3)
    .map_err(|error| error.to_string())?;
    let object = target
        .emit_object(optimized)
        .map_err(|error| error.to_string())?;
    let exports = state
        .module()
        .functions
        .iter()
        .filter(|function| function.exported)
        .map(|function| function.name.clone())
        .collect::<Vec<_>>();
    build_verified_native_artifact(
        product.artifact_kind,
        &object,
        &exports,
        product.build.header().map(<[u8]>::to_vec),
    )
    .map_err(|error| error.to_string())
}

#[cfg(feature = "native-toolchain")]
fn digest_tune_parts(domain: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(domain);
    for part in parts {
        digest.update(u64::try_from(part.len()).unwrap_or(u64::MAX).to_be_bytes());
        digest.update(part);
    }
    digest.finalize().into()
}

#[cfg(feature = "native-toolchain")]
pub(super) fn publish_native_product(product: &NativeBuildProduct) -> Result<(), String> {
    publish_verified_native_build(&product.paths, product.artifact_kind, &product.build)
}

#[cfg(feature = "native-toolchain")]
pub(super) fn publish_verified_native_build(
    paths: &NativeArtifactPaths,
    artifact_kind: NativeArtifactKind,
    build: &VerifiedNativeBuild,
) -> Result<(), String> {
    let mut transaction = OutputTransaction::new();
    if artifact_kind == NativeArtifactKind::Executable {
        transaction.stage_executable(paths.primary.clone(), build.primary())?;
    } else {
        transaction.stage(paths.primary.clone(), build.primary())?;
    }
    if let (Some(path), Some(header)) = (&paths.header, build.header()) {
        transaction.stage(path.clone(), header)?;
    }
    if let (Some(path), Some(bytes)) = (&paths.import_library, build.import_library()) {
        transaction.stage(path.clone(), bytes)?;
    }
    transaction.commit()
}

#[cfg(feature = "native-toolchain")]
pub(super) fn run_build(args: &ParsedArgs) -> Result<(), String> {
    if args.tune_use.is_some() {
        return super::tune::run_replay(args);
    }
    if args.pgo_generate.is_some() {
        return run_profile_generation_build(args);
    }
    if args.cpu == Some(CpuPolicy::Multiversion) {
        let input = require_input(args, "build")?;
        require_out(args, "build")?;
        let kind = args.kind.unwrap_or(ArtifactKind::Dynamic);
        let overflow_mode = parse_overflow_mode(args)?;
        let bounds_mode = parse_bounds_mode(args)?;
        let opt_level = parse_opt_level(args)?;
        let (source, checked) = check_file(input)?;
        if !checked.diagnostics.is_empty() {
            return Err(format_diagnostics(&source, &checked.diagnostics));
        }
        let consumer = if kind == ArtifactKind::Executable {
            KirConsumer::NativeExecutable
        } else {
            KirConsumer::NativeLibrary
        };
        return run_multiversion_planning_build(
            args,
            &checked.checked_program,
            consumer,
            kind,
            overflow_mode,
            bounds_mode,
            opt_level,
        );
    }
    let product = compile_verified_native_product(args)?;
    publish_native_product(&product)?;
    println!(
        "OK: built native {}",
        artifact_kind_name(product.artifact_kind)
    );
    println!("{}", product.paths.primary.display());
    if let Some(path) = &product.paths.header {
        println!("{}", path.display());
    }
    if let Some(path) = &product.paths.import_library {
        println!("{}", path.display());
    }
    Ok(())
}

#[cfg(feature = "native-toolchain")]
#[allow(clippy::too_many_arguments)]
fn run_multiversion_planning_build(
    args: &ParsedArgs,
    program: &CheckedProgram,
    consumer: KirConsumer,
    kind: ArtifactKind,
    overflow_mode: OverflowMode,
    bounds_mode: BoundsMode,
    opt_level: u8,
) -> Result<(), String> {
    if kind == ArtifactKind::Object {
        return Err("CPU multiversioning does not support --kind object.".to_string());
    }
    let targets = NativeMultiversionTargetSet::host(consumer).map_err(|error| error.to_string())?;
    let baseline_target = targets
        .target(calckernel::KirMultiversionTierId::Baseline)
        .ok_or_else(|| "multiversion target set is missing its baseline target".to_string())?;
    let application = args
        .pgo_use
        .as_ref()
        .map(|_| {
            prepare_profile_application(
                program,
                args,
                ProfileApplicationRequest {
                    target: baseline_target,
                    consumer,
                    overflow_mode,
                    bounds_mode,
                    opt_level,
                    cpu_policy: CpuPolicy::Multiversion,
                },
            )
        })
        .transpose()?;
    let compiled = if let Some(application) = &application {
        compile_profile_guided_kir(program, application, args)?
    } else {
        compile_kir(
            program,
            KirCompilationTarget {
                consumer,
                profile: Some(targets.target_set().tiers[0].profile.clone()),
            },
            overflow_mode,
            bounds_mode,
            opt_level,
            false,
            args,
        )?
    };
    let hot_roots = compiled.result.pgo.as_ref().map(|pgo| {
        pgo.functions
            .iter()
            .filter(|function| function.hot)
            .map(|function| function.function)
            .collect::<BTreeSet<_>>()
    });
    let request = KirMultiversionPlanningRequest {
        logical_pre_state: compiled
            .result
            .artifact
            .clone()
            .ok_or_else(|| "multiversion baseline KIR artifact is missing".to_string())?,
        target_set: targets.target_set().clone(),
        pgo_hot_roots: hot_roots,
        shared_growth_consumed: 0,
    };
    let bundle = propose_kir_multiversion_bundle(&request)?;
    check_kir_multiversion_bundle(&request, &bundle)?;
    if let Some(application) = &application {
        emit_profile_analysis(&application.analysis, args)?;
    }
    emit_pgo_optimizer_report(&compiled.result, args)?;
    let cache_manifest = multiversion_cache_manifest(
        args,
        baseline_target,
        consumer,
        kind,
        overflow_mode,
        bounds_mode,
        opt_level,
        &bundle,
    )?;
    let objects = if let Some(cached) =
        cache::load_multiversion_bundle(baseline_target, &cache_manifest, &bundle, args.no_cache)
    {
        cached
    } else {
        let context = NativeContext::new().map_err(|error| error.to_string())?;
        let emitted = emit_native_multiversion_objects(
            &context,
            &targets,
            &request,
            &bundle,
            compiled.result.pgo.as_ref(),
            &EmitLlvmOptions {
                source_file_name: None,
                target_triple: args.target.clone(),
            },
        )
        .map_err(|error| error.to_string())?;
        cache::store_multiversion_bundle(&cache_manifest, &emitted, args.no_cache);
        emitted
    };
    let exports = request
        .logical_pre_state
        .functions
        .iter()
        .filter(|function| function.exported)
        .map(|function| function.name.clone())
        .collect::<Vec<_>>();
    let artifact_kind = match kind {
        ArtifactKind::Executable => NativeArtifactKind::Executable,
        ArtifactKind::Dynamic => NativeArtifactKind::Dynamic,
        ArtifactKind::Static => NativeArtifactKind::Static,
        ArtifactKind::Object => unreachable!("multiversion object rejected above"),
    };
    let out = require_out(args, "build")?;
    let paths = NativeArtifactPaths::new(NativePlatform::host(), artifact_kind, &absolutize(out));
    let header = (kind != ArtifactKind::Executable).then(|| {
        annotate_unsafe_contracts(
            &emit_native_header(
                &request.logical_pre_state,
                if kind == ArtifactKind::Dynamic {
                    NativeHeaderMode::Dynamic
                } else {
                    NativeHeaderMode::StaticOrObject
                },
            ),
            program,
        )
    });
    let (primary, import_library) = match kind {
        ArtifactKind::Executable => (
            link_native_multiversion_executable(&objects)
                .map_err(|error| error.to_string())?
                .as_bytes()
                .to_vec(),
            None,
        ),
        ArtifactKind::Static => (
            create_native_multiversion_static_archive(&objects)
                .map_err(|error| error.to_string())?
                .as_bytes()
                .to_vec(),
            None,
        ),
        ArtifactKind::Dynamic => {
            let library = link_native_multiversion_dynamic_library(&objects, &exports)
                .map_err(|error| error.to_string())?;
            (
                library.as_bytes().to_vec(),
                library.import_library().map(<[u8]>::to_vec),
            )
        }
        ArtifactKind::Object => unreachable!("multiversion object rejected above"),
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
    println!(
        "OK: built native {} with multiversion bundle {}",
        artifact_kind_name(artifact_kind),
        bundle.digest_hex()
    );
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
#[allow(clippy::too_many_arguments)]
fn multiversion_cache_manifest(
    args: &ParsedArgs,
    target: &NativeTarget,
    consumer: KirConsumer,
    kind: ArtifactKind,
    overflow_mode: OverflowMode,
    bounds_mode: BoundsMode,
    opt_level: u8,
    bundle: &calckernel::KirMultiversionBundle,
) -> Result<CacheManifest, String> {
    let source = read_file_bytes(require_input(args, "build")?)?;
    let bridge = calckernel::bridge_info().map_err(|error| error.to_string())?;
    let target_triple = target.triple().map_err(|error| error.to_string())?;
    let cpu = target.cpu().map_err(|error| error.to_string())?;
    let features = target.features().map_err(|error| error.to_string())?;
    let profile_digest = args
        .pgo_use
        .as_deref()
        .map(read_file_bytes)
        .transpose()?
        .map(|bytes| digest_hex(&Sha256::digest(bytes)))
        .unwrap_or_else(|| "none".to_string());
    let profile_contract = calckernel::CkProfileContract::schema1();
    let profile_identity = format!(
        "mode={};format={};contract={};digest={profile_digest}",
        if args.pgo_use.is_some() { "use" } else { "off" },
        profile_contract.format_schema,
        profile_contract.contract_schema,
    );
    let artifact_identity = format!(
        "kind={};topology={}",
        match kind {
            ArtifactKind::Executable => "executable",
            ArtifactKind::Dynamic => "dynamic",
            ArtifactKind::Static => "static",
            ArtifactKind::Object => "object",
        },
        if consumer == KirConsumer::NativeExecutable {
            "native-executable"
        } else {
            "native-library"
        }
    );
    let pgo_identity = format!(
        "minimum-observations={};branch-bp={};histogram-bp={};cold-bp={};hot-work-bp={};root-work-bp={};site-schema=3;cost-schema=3",
        profile_contract.minimum_decision_observations,
        profile_contract.branch_dominance_basis_points,
        profile_contract.histogram_dominance_basis_points,
        profile_contract.cold_basis_points,
        profile_contract.hot_work_coverage_basis_points,
        profile_contract.minimum_root_work_basis_points,
    );
    let multiversion_identity = format!(
        "schema={};target-set={};bundle={}",
        bundle.schema_version,
        bundle.target_set.digest_hex(),
        bundle.digest_hex()
    );
    let dispatch_identity = format!(
        "dispatch-plan={};detector=closed-v1;thunk=acqrel-v1;runtime={}",
        bundle.digest_hex(),
        calckernel::NATIVE_DISPATCH_RUNTIME_SHA256
    );
    let budget_identity = format!(
        "minimum-benefit-bp={};minimum-units={};maximum-variants={};maximum-growth-bp={};shared-before={};trial={};additional={};total={}",
        profile_contract.minimum_variant_benefit_basis_points,
        profile_contract.minimum_absolute_cost_units,
        profile_contract.maximum_enhanced_variants,
        profile_contract.maximum_additional_kir_basis_points,
        bundle.shared_growth_consumed_before,
        bundle.trial_audit_units,
        bundle.additional_kir_units,
        bundle.total_kir_units,
    );
    let codegen_contract =
        "kir-v3;strict-fp;entry-wrapper-v1;multiversion;separate-modules;dispatch-v1".to_string();
    let key_input = CacheKeyInput {
        source,
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
        kir_contract_version: 3,
        sanitizer_mode: 0,
        target_profile_digest: bundle.target_set.tiers[0].profile.digest_hex(),
        vector_cost_model_schema: calckernel::KIR_VECTOR_COST_MODEL_SCHEMA,
        vector_proof_schema: calckernel::KIR_VECTOR_PROOF_SCHEMA,
        vector_budget_identity: calckernel::kir_vector_budget_identity().to_string(),
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
        profile_identity: profile_identity.clone(),
        artifact_identity: artifact_identity.clone(),
        pgo_identity: pgo_identity.clone(),
        multiversion_identity: multiversion_identity.clone(),
        dispatch_identity: dispatch_identity.clone(),
        budget_identity: budget_identity.clone(),
    };
    let key = cache::cache_key_hex(&key_input);
    Ok(CacheManifest {
        key,
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
        target_profile_digest: key_input.target_profile_digest,
        vector_cost_model_schema: key_input.vector_cost_model_schema,
        vector_proof_schema: key_input.vector_proof_schema,
        vector_budget_identity: key_input.vector_budget_identity,
        profile_identity,
        artifact_identity,
        pgo_identity,
        multiversion_identity,
        dispatch_identity,
        budget_identity,
    })
}

#[cfg(feature = "native-toolchain")]
struct CompiledProfileGeneration {
    result: KirPassManagerResult,
    plan: calckernel::CkProfileKirPlan,
    prepared_contracts: Option<calckernel::ContractFactSet>,
    semantic_graph_digest: [u8; 32],
}

#[cfg(feature = "native-toolchain")]
fn run_profile_generation_build(args: &ParsedArgs) -> Result<(), String> {
    let input = require_input(args, "build")?;
    let out = require_out(args, "build")?;
    let directory = args
        .pgo_generate
        .as_deref()
        .expect("generation build requires a directory");
    let kind = args.kind.unwrap_or(ArtifactKind::Dynamic);
    if kind == ArtifactKind::Object {
        return Err("Profile generation does not support --kind object.".to_string());
    }
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
    let cpu_policy = args.cpu.unwrap_or(CpuPolicy::Baseline);
    let target_cpu = match cpu_policy {
        CpuPolicy::Baseline => NativeCpu::Baseline,
        CpuPolicy::Native => NativeCpu::Native,
        CpuPolicy::Multiversion => NativeCpu::Multiversion,
    };
    let target = NativeTarget::host_with_cpu(target_cpu).map_err(|error| error.to_string())?;
    let target_profile = target
        .kir_profile(consumer)
        .map_err(|error| error.to_string())?;
    let compiled = compile_profile_generation_kir(
        &checked.checked_program,
        consumer,
        target_profile,
        overflow_mode,
        bounds_mode,
        args,
        true,
    )?;
    let anchor =
        anchor_profile_directory(&absolutize(directory)).map_err(|error| error.to_string())?;
    let identity = profile_generation_identity(
        &target,
        &compiled,
        overflow_mode,
        bounds_mode,
        opt_level,
        cpu_policy,
        consumer,
    )?;
    let generation = NativeProfileGeneration::new(compiled.plan.clone(), identity, anchor);
    let flush_symbol = generation
        .flush_symbol()
        .map_err(|error| error.to_string())?;
    let context = NativeContext::new().map_err(|error| error.to_string())?;
    let lowered = lower_native_profile_generation_module(
        &context,
        &target,
        &compiled.result,
        &generation,
        &EmitLlvmOptions {
            source_file_name: Some(input.to_string()),
            target_triple: args.target.clone(),
        },
    )
    .map_err(|error| error.to_string())?;
    let optimized = lowered
        .verify()
        .map_err(|error| error.to_string())?
        .audit()
        .map_err(|error| error.to_string())?
        .optimize(&target, NativeOptimizationLevel::O2)
        .map_err(|error| error.to_string())?;
    let object = target
        .emit_object(optimized)
        .map_err(|error| error.to_string())?;
    let artifact_kind = match kind {
        ArtifactKind::Executable => NativeArtifactKind::Executable,
        ArtifactKind::Dynamic => NativeArtifactKind::Dynamic,
        ArtifactKind::Static => NativeArtifactKind::Static,
        ArtifactKind::Object => unreachable!("generation object rejected above"),
    };
    let paths = NativeArtifactPaths::new(NativePlatform::host(), artifact_kind, &absolutize(out));
    let header = (!matches!(kind, ArtifactKind::Executable)).then(|| {
        annotate_unsafe_contracts(
            &emit_native_profile_generation_header(
                &compiled.plan.module,
                if kind == ArtifactKind::Dynamic {
                    NativeHeaderMode::Dynamic
                } else {
                    NativeHeaderMode::StaticOrObject
                },
                &flush_symbol,
            ),
            &checked.checked_program,
        )
    });
    let mut exports = compiled
        .plan
        .module
        .functions
        .iter()
        .filter(|function| function.exported)
        .map(|function| function.name.clone())
        .collect::<Vec<_>>();
    exports.push(flush_symbol.clone());
    let (primary, import_library) = match kind {
        ArtifactKind::Executable => (
            link_native_profile_generation_executable(&object)
                .map_err(|error| error.to_string())?
                .as_bytes()
                .to_vec(),
            None,
        ),
        ArtifactKind::Static => (
            create_native_profile_generation_static_archive(&target, &object)
                .map_err(|error| error.to_string())?
                .as_bytes()
                .to_vec(),
            None,
        ),
        ArtifactKind::Dynamic => {
            let library = link_native_profile_generation_dynamic_library(&object, &exports)
                .map_err(|error| error.to_string())?;
            (
                library.as_bytes().to_vec(),
                library.import_library().map(<[u8]>::to_vec),
            )
        }
        ArtifactKind::Object => unreachable!("generation object rejected above"),
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
    println!(
        "OK: built native profile-generation {}",
        artifact_kind_name(artifact_kind)
    );
    println!("{}", paths.primary.display());
    println!("Profile flush: {flush_symbol}");
    Ok(())
}

#[cfg(feature = "native-toolchain")]
fn compile_profile_generation_kir(
    program: &CheckedProgram,
    consumer: KirConsumer,
    profile: KirTargetProfile,
    overflow_mode: OverflowMode,
    bounds_mode: BoundsMode,
    args: &ParsedArgs,
    emit_inspection: bool,
) -> Result<CompiledProfileGeneration, String> {
    let semantic_mir = lower_to_mir(program).map_err(|error| error.to_string())?;
    let semantic_graph_digest = hash_domain(
        b"CK-SEMANTIC-MODULE-GRAPH\0",
        print_mir_module(&semantic_mir).as_bytes(),
    );
    let config = KirBuildConfig {
        consumer,
        overflow_mode: match overflow_mode {
            OverflowMode::Unchecked => KirOverflowMode::Unchecked,
            OverflowMode::Checked => KirOverflowMode::Checked,
        },
        bounds_mode: match bounds_mode {
            BoundsMode::Unchecked => KirBoundsMode::Unchecked,
            BoundsMode::Checked => KirBoundsMode::Checked,
        },
        sanitizer_mode: KirSanitizerMode::Disabled,
    };
    let initial = build_kir_module_with_profile(&semantic_mir, config, profile)
        .map_err(|error| error.to_string())?;
    let contracts =
        import_contract_facts(&initial, program, 0).map_err(|error| error.to_string())?;
    let prepared = run_kir_pass_pipeline(initial, KirOptimizationLevel::O1, Some(&contracts));
    if !prepared.errors.is_empty() {
        return Err(format!(
            "KIR generation preparation failed: {}",
            prepared.errors.join("; ")
        ));
    }
    let canonical = prepared
        .artifact
        .as_ref()
        .ok_or_else(|| "KIR generation preparation produced no artifact".to_string())?;
    let plan = prepare_ck_profile_kir(canonical, CkProfileKirMode::Generate)
        .map_err(|error| error.to_string())?;
    let result = run_kir_pass_pipeline(plan.module.clone(), KirOptimizationLevel::O0, None);
    if !result.errors.is_empty() {
        return Err(format!(
            "instrumented KIR verification failed: {}",
            result.errors.join("; ")
        ));
    }
    if emit_inspection {
        emit_kir_inspection(program, &result, args)?;
    }
    Ok(CompiledProfileGeneration {
        result,
        plan,
        prepared_contracts: prepared.contract_facts,
        semantic_graph_digest,
    })
}

#[cfg(feature = "native-toolchain")]
struct ProfileApplicationRequest<'a> {
    target: &'a NativeTarget,
    consumer: KirConsumer,
    overflow_mode: OverflowMode,
    bounds_mode: BoundsMode,
    opt_level: u8,
    cpu_policy: CpuPolicy,
}

#[cfg(feature = "native-toolchain")]
struct PreparedProfileApplication {
    analysis: CkProfileAnalysis,
    use_plan: calckernel::CkProfileKirPlan,
    contract: CkProfileContract,
    contracts: Option<calckernel::ContractFactSet>,
}

#[cfg(feature = "native-toolchain")]
fn prepare_profile_application(
    program: &CheckedProgram,
    args: &ParsedArgs,
    request: ProfileApplicationRequest<'_>,
) -> Result<PreparedProfileApplication, String> {
    let target_profile = request
        .target
        .kir_profile(request.consumer)
        .map_err(|error| error.to_string())?;
    let compiled = compile_profile_generation_kir(
        program,
        request.consumer,
        target_profile,
        request.overflow_mode,
        request.bounds_mode,
        args,
        false,
    )?;
    let identity = profile_generation_identity(
        request.target,
        &compiled,
        request.overflow_mode,
        request.bounds_mode,
        request.opt_level,
        request.cpu_policy,
        request.consumer,
    )?;
    let profile_path = absolutize(
        args.pgo_use
            .as_deref()
            .ok_or_else(|| "profile use path is missing".to_string())?,
    );
    let (profile, _) = read_profile_input(&profile_path).map_err(|error| error.to_string())?;
    let contract = profile.identity.contract.clone();
    let work_terms = profile_work_terms(&compiled.plan)?;
    let analysis = apply_profile(&profile, &identity, &compiled.plan.sites, &work_terms)
        .map_err(|error| error.to_string())?;
    let use_plan = prepare_ck_profile_kir(&compiled.plan.module, CkProfileKirMode::Use)
        .map_err(|error| error.to_string())?;
    let immutable = CkImmutableProfileAnalysis::new(analysis.clone());
    validate_profile_analysis_for_optimizer(&use_plan, &immutable, &compiled.result.proofs)?;
    Ok(PreparedProfileApplication {
        analysis,
        use_plan,
        contract,
        contracts: compiled.prepared_contracts,
    })
}

#[cfg(feature = "native-toolchain")]
fn compile_profile_guided_kir(
    program: &CheckedProgram,
    application: &PreparedProfileApplication,
    args: &ParsedArgs,
) -> Result<CompiledKir, String> {
    let immutable = CkImmutableProfileAnalysis::new(application.analysis.clone());
    let pre_tune = Some(prepare_kir_pre_tune_state(
        application.use_plan.module.clone(),
        application.contracts.as_ref(),
    )?);
    let result = run_profile_guided_kir_pass_pipeline(
        &application.use_plan,
        &immutable,
        &application.contract,
        application.contracts.as_ref(),
    );
    if !result.errors.is_empty() {
        return Err(format!(
            "KIR PGO verification failed: {}",
            result.errors.join("; ")
        ));
    }
    emit_kir_inspection(program, &result, args)?;
    Ok(CompiledKir { result, pre_tune })
}

#[cfg(feature = "native-toolchain")]
fn late_layout_plan_for_optimized_kir(
    compiled: &CompiledKir,
    application: &PreparedProfileApplication,
    optimized: &calckernel::OptimizedNativeModule<'_>,
) -> Result<CkLateProfileLayoutPlan, String> {
    let ordinary = compiled
        .result
        .artifact
        .as_ref()
        .ok_or_else(|| "optimized KIR artifact is missing".to_string())?;
    let use_plan = match prepare_ck_profile_kir(ordinary, CkProfileKirMode::Use) {
        Ok(plan) => plan,
        Err(_) => return Ok(CkLateProfileLayoutPlan::default()),
    };
    let immutable = CkImmutableProfileAnalysis::new(application.analysis.clone());
    if validate_profile_analysis_for_optimizer(&use_plan, &immutable, &compiled.result.proofs)
        .is_err()
    {
        return Ok(CkLateProfileLayoutPlan::default());
    }
    let proposed = build_late_profile_layout_plan(&use_plan, &application.analysis);
    let ir = optimized
        .to_ir_string()
        .map_err(|error| error.to_string())?;
    Ok(reconcile_late_layout_names(proposed, &ir))
}

#[cfg(feature = "native-toolchain")]
fn reconcile_late_layout_names(
    mut plan: CkLateProfileLayoutPlan,
    ir: &str,
) -> CkLateProfileLayoutPlan {
    for function in &mut plan.functions {
        let candidates = [
            function.llvm_function.clone(),
            format!("__ck_impl_{}", function.llvm_function),
        ];
        let Some((name, labels)) = candidates.into_iter().find_map(|candidate| {
            llvm_function_labels(ir, &candidate).map(|labels| (candidate, labels))
        }) else {
            return CkLateProfileLayoutPlan::default();
        };
        function.llvm_function = name;
        for block in &mut function.blocks {
            let matches = labels
                .iter()
                .filter(|label| *label == block || label.starts_with(&format!("{block}.")))
                .cloned()
                .collect::<Vec<_>>();
            if matches.len() != 1 {
                return CkLateProfileLayoutPlan::default();
            }
            *block = matches[0].clone();
        }
    }
    plan
}

#[cfg(feature = "native-toolchain")]
fn llvm_function_labels(ir: &str, function: &str) -> Option<Vec<String>> {
    let marker = format!("@{function}(");
    let mut inside = false;
    let mut labels = Vec::new();
    for line in ir.lines() {
        if line.starts_with("define ") && line.contains(&marker) {
            inside = true;
            continue;
        }
        if inside && line == "}" {
            return Some(labels);
        }
        if inside
            && let Some((label, _)) = line.trim().split_once(':')
            && label != "entry"
            && !label.is_empty()
        {
            labels.push(label.to_string());
        }
    }
    None
}

#[cfg(feature = "native-toolchain")]
fn emit_late_layout_report(
    report: &CkLateProfileLayoutReport,
    reason: &str,
    args: &ParsedArgs,
) -> Result<(), String> {
    if !args.explain_optimization {
        return Ok(());
    }
    let text = format!(
        "===== O2 LATE PROFILE LAYOUT =====\naccepted={} changed={} pre={} post={} structural={} repairs={:?} reason={reason}\n",
        report.accepted,
        report.changed,
        digest_hex(&report.pre_layout_digest),
        digest_hex(&report.post_layout_digest),
        digest_hex(&report.pre_structural_digest),
        report.repairs,
    );
    io::stderr()
        .lock()
        .write_all(text.as_bytes())
        .map_err(|error| format!("failed to write late layout report: {error}"))
}

#[cfg(feature = "native-toolchain")]
fn profile_work_terms(
    plan: &calckernel::CkProfileKirPlan,
) -> Result<Vec<CkProfileWorkTerm>, String> {
    plan.annotations
        .iter()
        .filter_map(|annotation| match annotation.event {
            CkProfileEvent::FunctionEntry { function, .. } => Some((annotation, function)),
            _ => None,
        })
        .map(|(annotation, function)| {
            let function = plan
                .module
                .functions
                .iter()
                .find(|candidate| candidate.id == function)
                .ok_or_else(|| "profile work term names an unknown function".to_string())?;
            let units = function
                .blocks
                .iter()
                .try_fold(0u64, |total, block| {
                    let instructions = u64::try_from(block.instructions.len())
                        .map_err(|_| "profile work term is too large".to_string())?;
                    total
                        .checked_add(instructions.saturating_add(1))
                        .ok_or_else(|| "profile work term overflow".to_string())
                })?
                .max(1);
            Ok(CkProfileWorkTerm {
                site_id: annotation.site_id,
                function_digest: annotation.descriptor.function_digest,
                static_cost_units: units,
            })
        })
        .collect()
}

#[cfg(feature = "native-toolchain")]
fn emit_profile_analysis(analysis: &CkProfileAnalysis, args: &ParsedArgs) -> Result<(), String> {
    if !args.explain_optimization {
        return Ok(());
    }
    let known = analysis
        .sites
        .iter()
        .filter(|site| !matches!(site.observation, CkProfileObservation::Unknown(_)))
        .count();
    let mut text = format!(
        "===== PROFILE ANALYSIS =====\nidentity={} coverage={}/{} proof-authority=false\n",
        digest_hex(&analysis.identity_digest),
        known,
        analysis.sites.len()
    );
    for site in &analysis.sites {
        let status = match site.observation {
            CkProfileObservation::Unknown(reason) => format!("unknown:{reason:?}"),
            _ => format!(
                "known:observations={}",
                site.observation.total().unwrap_or(0)
            ),
        };
        text.push_str(&format!(
            "site={} status={status}\n",
            digest_hex(&site.descriptor.id.0)
        ));
    }
    for function in &analysis.functions {
        text.push_str(&format!(
            "function={} work={} rank={} hot-root={}\n",
            digest_hex(&function.function_digest),
            function
                .dynamic_work
                .map_or_else(|| "unknown".to_string(), |value| value.to_string()),
            function
                .rank
                .map_or_else(|| "none".to_string(), |value| value.to_string()),
            function.hot_root
        ));
    }
    io::stderr()
        .lock()
        .write_all(text.as_bytes())
        .map_err(|error| format!("failed to write profile analysis: {error}"))
}

#[cfg(feature = "native-toolchain")]
fn emit_pgo_optimizer_report(
    result: &KirPassManagerResult,
    args: &ParsedArgs,
) -> Result<(), String> {
    if !args.explain_optimization {
        return Ok(());
    }
    let Some(plan) = result.pgo.as_ref() else {
        return Ok(());
    };
    let accepted = plan
        .decisions
        .iter()
        .filter(|decision| decision.accepted)
        .count();
    let mut text = format!(
        "===== O3 PGO OPTIMIZER =====\naudit={} accepted={}/{} functions={} branches={} loops={} values={} proof-authority=false\n",
        digest_hex(&plan.audit_digest),
        accepted,
        plan.decisions.len(),
        plan.functions.len(),
        plan.branches.len(),
        plan.loop_hints.len(),
        plan.value_hints.len(),
    );
    for decision in &plan.decisions {
        text.push_str(&format!(
            "site={} kind={:?} accepted={} observations={} class={} reason={}\n",
            digest_hex(&decision.site_id.0),
            decision.kind,
            decision.accepted,
            decision
                .observations
                .map_or_else(|| "unknown".to_string(), |value| value.to_string()),
            decision
                .selected_class
                .map_or_else(|| "none".to_string(), |value| value.to_string()),
            decision.reason,
        ));
    }
    io::stderr()
        .lock()
        .write_all(text.as_bytes())
        .map_err(|error| format!("failed to write O3 PGO report: {error}"))
}

#[cfg(feature = "native-toolchain")]
fn digest_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(feature = "native-toolchain")]
fn profile_generation_identity(
    target: &NativeTarget,
    compiled: &CompiledProfileGeneration,
    overflow_mode: OverflowMode,
    bounds_mode: BoundsMode,
    opt_level: u8,
    cpu_policy: CpuPolicy,
    consumer: KirConsumer,
) -> Result<CkProfileIdentity, String> {
    let triple = target.triple().map_err(|error| error.to_string())?;
    let cpu = target.cpu().map_err(|error| error.to_string())?;
    let features = target.features().map_err(|error| error.to_string())?;
    let target_set_digest = if cpu_policy == CpuPolicy::Multiversion {
        NativeMultiversionTargetSet::host(consumer)
            .map_err(|error| error.to_string())?
            .target_set()
            .digest
    } else {
        let mut target_set = Vec::new();
        target_set.extend_from_slice(triple.as_bytes());
        target_set.push(0);
        target_set.extend_from_slice(cpu.as_bytes());
        target_set.push(0);
        target_set.extend_from_slice(features.as_bytes());
        target_set.push(match cpu_policy {
            CpuPolicy::Baseline => 1,
            CpuPolicy::Native => 2,
            CpuPolicy::Multiversion => unreachable!("handled as a closed target set"),
        });
        hash_domain(b"CK-PROFILE-TARGET-SET\0", &target_set)
    };
    Ok(CkProfileIdentity {
        compiler: CkCompilerProfileIdentity {
            package_version: env!("CARGO_PKG_VERSION").to_string(),
            source_identity: compiler_source_identity(),
            profile_runtime_identity: parse_hex_digest(NATIVE_PROFILE_RUNTIME_SHA256)?,
        },
        module: CkModuleProfileIdentity {
            semantic_graph_digest: compiled.semantic_graph_digest,
            pre_profile_kir_digest: compiled.plan.pre_profile_kir_digest,
            site_table_digest: compiled.plan.site_table_digest,
        },
        schemas: CkProfileSchemaIdentity {
            language: 1,
            native_abi: calckernel::NATIVE_ABI_VERSION,
            runtime_abi: calckernel::RUNTIME_ABI_VERSION,
            kir: 3,
            proof: 3,
            cost_model: 3,
            target_profile: 1,
            llvm_bridge: calckernel::LLVM_BRIDGE_ABI_VERSION,
            cache: 4,
        },
        target: CkProfileTargetIdentity {
            triple,
            pointer_width: usize::BITS as u8,
            endianness: if cfg!(target_endian = "little") {
                CkProfileEndianness::Little
            } else {
                CkProfileEndianness::Big
            },
            object_format: match NativePlatform::host() {
                NativePlatform::Linux => CkProfileObjectFormat::Elf,
                NativePlatform::Darwin => CkProfileObjectFormat::MachO,
                NativePlatform::Windows => CkProfileObjectFormat::Coff,
            },
            os_abi: match NativePlatform::host() {
                NativePlatform::Linux => "linux-gnu",
                NativePlatform::Darwin => "darwin",
                NativePlatform::Windows => "windows-msvc",
            }
            .to_string(),
            target_set_digest,
        },
        modes: CkProfileModes {
            overflow_checked: overflow_mode == OverflowMode::Checked,
            bounds_checked: bounds_mode == BoundsMode::Checked,
            strict_float: true,
            sanitizer: false,
            topology: if consumer == KirConsumer::NativeExecutable {
                CkProfileTopology::NativeExecutable
            } else {
                CkProfileTopology::NativeLibrary
            },
            optimization_family: if opt_level == 2 {
                CkProfileOptimizationFamily::O2
            } else {
                CkProfileOptimizationFamily::O3
            },
            cpu_policy: match cpu_policy {
                CpuPolicy::Baseline => CkProfileCpuPolicy::Baseline,
                CpuPolicy::Native => CkProfileCpuPolicy::Native,
                CpuPolicy::Multiversion => CkProfileCpuPolicy::Multiversion,
            },
        },
        contract: CkProfileContract::schema1(),
    })
}

#[cfg(feature = "native-toolchain")]
pub(super) fn compiler_source_identity() -> [u8; 32] {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(env!("CARGO_PKG_VERSION").as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(env!("CKC_LLVM_MANIFEST_SHA256").as_bytes());
    bytes.extend_from_slice(&calckernel::LLVM_BRIDGE_ABI_VERSION.to_be_bytes());
    hash_domain(b"CK-COMPILER-SOURCE-IDENTITY\0", &bytes)
}

#[cfg(feature = "native-toolchain")]
fn parse_hex_digest(text: &str) -> Result<[u8; 32], String> {
    if text.len() != 64 {
        return Err("profile runtime digest is malformed".to_string());
    }
    let mut output = [0; 32];
    for (index, byte) in output.iter_mut().enumerate() {
        let pair = &text[index * 2..index * 2 + 2];
        *byte = u8::from_str_radix(pair, 16)
            .map_err(|_| "profile runtime digest is malformed".to_string())?;
    }
    Ok(output)
}

#[cfg(feature = "native-toolchain")]
fn hash_domain(domain: &[u8], bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    hasher.finalize().into()
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
