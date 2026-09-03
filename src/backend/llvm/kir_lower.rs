use std::collections::{BTreeMap, HashMap, HashSet};

use sha2::Digest;

use crate::*;

use super::{
    EmitLlvmOptions,
    abi::{add_export_thunks, implementation_name},
    builder::{NativeBlock, NativeBuilder, NativeFunction, NativeType, NativeValue},
    context::NativeContext,
    entry::add_entry_wrapper,
    error::NativeError,
    fact_audit::{NativeFactProperty, NativeFactSource, NativeStrengtheningKind},
    ffi::{BridgeBinaryOp, BridgeCastOp, BridgeCompareOp, BridgeMemoryEffects, BridgeOverflowOp},
    layout::LlvmStructLayout,
    lower_shared::{
        TypeRegistry, binary_op, compare_op, lowering_error, runtime_signature, unary_op,
    },
    module::NativeModule,
    names::llvm_source_file_name,
    profile_generation::NativeProfileGeneration,
    target::NativeTarget,
};

/// Lowers one optimized, evidence-verified Native KIR artifact directly to LLVM.
pub fn lower_native_kir_module<'context>(
    context: &'context NativeContext,
    target: &NativeTarget,
    result: &KirPassManagerResult,
    options: &EmitLlvmOptions,
) -> Result<NativeModule<'context>, NativeError> {
    lower_native_kir_module_inner(context, target, result, None, options)
}

/// Lowers the immutable baseline module and installs baseline-safe per-root
/// dispatchers without merging any enhanced LLVM module into it.
pub fn lower_native_multiversion_baseline_module<'context>(
    context: &'context NativeContext,
    target: &NativeTarget,
    result: &KirPassManagerResult,
    bundle: &KirMultiversionBundle,
    options: &EmitLlvmOptions,
) -> Result<NativeModule<'context>, NativeError> {
    validate_multiversion_lowering_input(result, bundle)?;
    let mut module = lower_native_kir_module(context, target, result, options)?;
    let namespace = dispatch_namespace(&bundle.target_set.digest);
    for entry in &bundle.dispatch_plan {
        if entry.ranked_tiers.len() != entry.implementation_symbols.len()
            || entry.ranked_tiers.last() != Some(&KirMultiversionTierId::Baseline)
            || entry.implementation_symbols.last() != Some(&entry.public_symbol)
        {
            return Err(lowering_error(
                "multiversion dispatch entry is malformed or lacks final baseline",
            ));
        }
        if entry.ranked_tiers.len() == 1 {
            continue;
        }
        let function = bundle
            .baseline
            .functions
            .iter()
            .find(|function| function.id == entry.root)
            .ok_or_else(|| lowering_error("multiversion dispatch root is missing"))?;
        let implementation = if bundle.baseline.config.consumer == KirConsumer::NativeExecutable
            && bundle
                .baseline
                .entry
                .as_ref()
                .is_some_and(|candidate| candidate.function_name == function.name)
        {
            "__ck_user_main".to_string()
        } else if function.exported {
            format!("__ck_impl_{}", function.name)
        } else {
            function.name.clone()
        };
        let baseline_hidden = format!("__ck_mv_{namespace}_{}_baseline", entry.public_symbol);
        let variants = entry
            .ranked_tiers
            .iter()
            .zip(&entry.implementation_symbols)
            .take(entry.ranked_tiers.len() - 1)
            .map(|(tier, symbol)| {
                Ok::<_, NativeError>((symbol.as_str(), dispatch_capabilities(*tier)?))
            })
            .collect::<Result<Vec<_>, _>>()?;
        module.add_multiversion_dispatch(
            &entry.public_symbol,
            &implementation,
            &baseline_hidden,
            &namespace,
            &variants,
        )?;
    }
    Ok(module)
}

/// Lowers one checked enhanced KIR module and exposes only its root symbol as
/// hidden external linkage. Helpers remain local to this object.
pub fn lower_native_multiversion_variant_module<'context>(
    context: &'context NativeContext,
    target: &NativeTarget,
    result: &KirPassManagerResult,
    variant: &KirMultiversionVariant,
    options: &EmitLlvmOptions,
) -> Result<NativeModule<'context>, NativeError> {
    if result.artifact.as_ref() != Some(&variant.module) {
        return Err(lowering_error(
            "multiversion variant result does not match the checked variant module",
        ));
    }
    let root_symbol = variant
        .hidden_symbols
        .iter()
        .find(|symbol| {
            variant
                .module
                .functions
                .iter()
                .find(|function| function.id == variant.root)
                .is_some_and(|function| function.name == symbol.hidden_name)
        })
        .map(|symbol| symbol.hidden_name.as_str())
        .ok_or_else(|| lowering_error("multiversion variant root symbol is missing"))?;
    let module = lower_native_kir_module(context, target, result, options)?;
    module.expose_hidden_function(root_symbol)?;
    Ok(module)
}

fn validate_multiversion_lowering_input(
    result: &KirPassManagerResult,
    bundle: &KirMultiversionBundle,
) -> Result<(), NativeError> {
    bundle.target_set.validate().map_err(lowering_error)?;
    let expected_digest: [u8; 32] =
        sha2::Sha256::digest(bundle.canonical_bytes_without_digest()).into();
    if result.artifact.as_ref() != Some(&bundle.baseline) || bundle.digest != expected_digest {
        return Err(lowering_error(
            "multiversion baseline or canonical bundle digest is stale",
        ));
    }
    Ok(())
}

fn dispatch_capabilities(tier: KirMultiversionTierId) -> Result<u32, NativeError> {
    match tier {
        KirMultiversionTierId::X86_64V3 => Ok(1),
        KirMultiversionTierId::X86_64V4 => Ok(3),
        KirMultiversionTierId::AArch64Sve => Ok(4),
        KirMultiversionTierId::AArch64Sve2 => Ok(12),
        KirMultiversionTierId::Baseline => Err(lowering_error(
            "baseline cannot appear in the enhanced dispatch prefix",
        )),
    }
}

fn dispatch_namespace(digest: &[u8; 32]) -> String {
    let mut output = String::with_capacity(16);
    for byte in &digest[..8] {
        use std::fmt::Write;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

/// Test-only seam for exercising the exact LLVM thunk transformation on a
/// host baseline module even when the host target set has no enhanced tier.
#[doc(hidden)]
pub fn test_add_multiversion_dispatch(
    module: &mut NativeModule<'_>,
    public_name: &str,
    implementation_name: &str,
    variant_name: &str,
    required_capabilities: u32,
) -> Result<(), NativeError> {
    module.add_multiversion_dispatch(
        public_name,
        implementation_name,
        &format!("__ck_mv_0102030405060708_{public_name}_baseline"),
        "0102030405060708",
        &[(variant_name, required_capabilities)],
    )
}

/// Lowers one canonical generation plan with CK-owned instrumentation.
pub fn lower_native_profile_generation_module<'context>(
    context: &'context NativeContext,
    target: &NativeTarget,
    result: &KirPassManagerResult,
    profile: &NativeProfileGeneration,
    options: &EmitLlvmOptions,
) -> Result<NativeModule<'context>, NativeError> {
    lower_native_kir_module_inner(context, target, result, Some(profile), options)
}

fn lower_native_kir_module_inner<'context>(
    context: &'context NativeContext,
    target: &NativeTarget,
    result: &KirPassManagerResult,
    profile_generation: Option<&NativeProfileGeneration>,
    options: &EmitLlvmOptions,
) -> Result<NativeModule<'context>, NativeError> {
    if !result.errors.is_empty() {
        return Err(lowering_error(format!(
            "KIR pipeline is not verified: {}",
            result.errors.join("; ")
        )));
    }
    let kir = result
        .artifact
        .as_ref()
        .ok_or_else(|| lowering_error("KIR pipeline has no verified artifact"))?;
    let evidence = validate_kir_optimization_evidence(
        kir,
        result.contract_facts.as_ref(),
        &result.proofs,
        &result.eliminated_guards,
        result.proofs.generation(),
    );
    if !evidence.errors.is_empty() {
        return Err(lowering_error(format!(
            "KIR artifact changed after verification: {}",
            evidence
                .errors
                .iter()
                .map(|error| error.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        )));
    }
    if !matches!(
        kir.config.consumer,
        KirConsumer::NativeLibrary | KirConsumer::NativeExecutable
    ) {
        return Err(lowering_error(
            "native LLVM lowering requires a native KIR consumer",
        ));
    }
    if let Some(pgo) = result.pgo.as_ref() {
        validate_pgo_plan_for_kir(kir, pgo).map_err(lowering_error)?;
    }
    if kir.profile.vector_operations_enabled() {
        let expected = target.kir_profile(kir.config.consumer)?;
        if kir.profile.digest_hex() != expected.digest_hex() {
            return Err(lowering_error(format!(
                "KIR target profile does not match the lowering TargetMachine: artifact={}, target={}",
                kir.profile.digest_hex(),
                expected.digest_hex()
            )));
        }
    }
    if let Some(requested) = options.target_triple.as_deref() {
        let actual = target.triple()?;
        if requested != actual {
            return Err(lowering_error(format!(
                "requested target triple '{requested}' does not match native target '{actual}'"
            )));
        }
    }
    if let Some(profile) = profile_generation {
        validate_native_profile_generation(kir, profile)?;
    }

    let shape = mir_shape(kir);
    let mut module = NativeModule::empty(context)?;
    module.configure(
        target,
        &llvm_source_file_name(options.source_file_name.as_deref()),
    )?;
    let sanitized = kir.config.sanitizer_mode == KirSanitizerMode::Contracts;
    // Entry contracts are not valid LLVM promises until the sanitizer has
    // dynamically established them. Attribute/assume/metadata strengthening
    // would otherwise make the check itself undefined and removable.
    let wrap_proofs = if sanitized {
        BTreeMap::new()
    } else {
        collect_wrap_proofs(kir, result)
    };
    let contract_attributes = if sanitized {
        BTreeMap::new()
    } else {
        collect_contract_attributes(kir, result)
    };
    let contract_assumes = if sanitized {
        BTreeMap::new()
    } else {
        collect_contract_assumes(kir, result)
    };
    let contract_memory_effects = if sanitized {
        BTreeMap::new()
    } else {
        collect_contract_memory_effects(kir, result)
    };
    let scoped_alias_facts = if sanitized {
        BTreeMap::new()
    } else {
        collect_scoped_alias_facts(kir, result)
    };
    let contract_checks = collect_contract_checks(kir, result);
    let pgo_functions = result
        .pgo
        .as_ref()
        .map(|plan| {
            plan.functions
                .iter()
                .filter_map(|profile| {
                    let cold = plan.function_is_profile_cold(kir, profile.function);
                    (profile.confident || cold).then_some((profile.function, (profile, cold)))
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let pgo_branches = result
        .pgo
        .as_ref()
        .map(|plan| {
            plan.branches
                .iter()
                .map(|profile| {
                    (
                        (profile.function, profile.block),
                        (profile.then_count, profile.else_count),
                    )
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    for ((function, instruction), (proof, kind)) in &wrap_proofs {
        let function_name = kir
            .functions
            .iter()
            .find(|candidate| candidate.id == *function)
            .map(|candidate| candidate.name.clone())
            .ok_or_else(|| lowering_error("wrap proof names an unknown KIR function"))?;
        module.register_fact_property(NativeFactProperty {
            kind: *kind,
            source: NativeFactSource::Proof(*proof),
            function: function_name,
            subject: format!("i{}", instruction.index()),
        });
    }
    for attribute in contract_attributes.values().flatten() {
        module.register_fact_property(attribute.property.clone());
    }
    for (function, assumes) in &contract_assumes {
        let function_name = kir
            .functions
            .iter()
            .find(|candidate| candidate.id == *function)
            .map(|candidate| candidate.name.clone())
            .ok_or_else(|| lowering_error("contract assume names an unknown KIR function"))?;
        for assumption in assumes {
            for kind in [
                NativeStrengtheningKind::Range,
                NativeStrengtheningKind::Assume,
            ] {
                module.register_fact_property(NativeFactProperty {
                    kind,
                    source: NativeFactSource::Fact(assumption.fact),
                    function: function_name.clone(),
                    subject: format!("contract.fact{}", assumption.fact.index()),
                });
            }
        }
    }
    for effect in contract_memory_effects.values() {
        module.register_fact_property(effect.property.clone());
    }
    for (function, facts) in &scoped_alias_facts {
        let function_name = kir
            .functions
            .iter()
            .find(|candidate| candidate.id == *function)
            .map(|candidate| candidate.name.clone())
            .ok_or_else(|| lowering_error("alias fact names an unknown KIR function"))?;
        for (fact, left, right) in facts {
            module.register_fact_property(NativeFactProperty {
                kind: NativeStrengtheningKind::AliasScope,
                source: NativeFactSource::Fact(*fact),
                function: function_name.clone(),
                subject: format!("v{}<->v{}", left.index(), right.index()),
            });
        }
    }
    let lowering_facts = NativeKirFacts {
        wrap_proofs: &wrap_proofs,
        contract_assumes: &contract_assumes,
        scoped_alias_facts: &scoped_alias_facts,
        contract_checks: &contract_checks,
    };
    {
        let types = TypeRegistry::new(context, &shape)?;
        let profile_runtime = profile_generation
            .map(|profile| add_native_profile_support(context, &module, &types, profile))
            .transpose()?;
        let status = status_abi(kir);
        let mut functions = HashMap::new();
        for (kir_function, mir_function) in kir.functions.iter().zip(&shape.functions) {
            let mut params = physical_param_types(&types, &kir_function.params)?;
            if status && kir_function.return_type != MirType::Void {
                params.push(types.pointer);
            }
            let implementation = if kir.config.consumer == KirConsumer::NativeExecutable
                && kir
                    .entry
                    .as_ref()
                    .is_some_and(|entry| entry.function_name == kir_function.name)
            {
                "__ck_user_main".to_string()
            } else {
                implementation_name(mir_function)
            };
            let handle = module.add_function(
                &implementation,
                if status {
                    types.i32
                } else {
                    types.get(&kir_function.return_type)?
                },
                &params,
                false,
            )?;
            if let Some((profile, cold)) = pgo_functions.get(&kir_function.id) {
                handle.set_profile(profile.entries, profile.hot, *cold)?;
            }
            for attribute in contract_attributes
                .get(&kir_function.id)
                .into_iter()
                .flatten()
            {
                apply_param_attribute(handle, attribute)?;
            }
            if let Some(effect) = contract_memory_effects.get(&kir_function.id) {
                handle.set_memory_effects(effect.effects)?;
            }
            functions.insert(kir_function.name.clone(), handle);
        }
        if let Some(entry) = &kir.entry {
            module.preserve_function(require_function(&functions, &entry.function_name)?)?;
        }
        for intrinsic in used_runtime_intrinsics(kir) {
            let (name, parameter) = runtime_signature(intrinsic);
            let params = parameter
                .as_ref()
                .map(|type_node| types.get(type_node))
                .transpose()?
                .into_iter()
                .collect::<Vec<_>>();
            functions.insert(
                name.to_string(),
                module.add_function(name, types.void, &params, true)?,
            );
        }
        let layout = LlvmStructLayout::new(&shape);
        let environment = KirLoweringEnvironment {
            types: &types,
            functions: &functions,
            layout: &layout,
            structs: &shape.structs,
            status_abi: status,
            facts: &lowering_facts,
            profile: profile_runtime.as_ref(),
            pgo_branches: &pgo_branches,
        };
        for function in &kir.functions {
            lower_function(context, &module, function, &environment)?;
        }
        add_export_thunks(context, &module, target, &shape, &types, &functions, status)?;
        if kir.config.consumer == KirConsumer::NativeExecutable {
            add_entry_wrapper(
                context,
                &module,
                &shape,
                &types,
                &functions,
                status,
                profile_runtime
                    .as_ref()
                    .map(|profile| profile.flush_control),
            )?;
        }
    }
    Ok(module)
}

#[derive(Debug, Clone)]
struct NativeProfileLoop {
    site: u32,
    function: FunctionId,
    header: BlockId,
    latches: Vec<BlockId>,
    exits: Vec<(BlockId, BlockId)>,
}

#[derive(Debug, Clone, Copy)]
struct NativeProfileCandidate {
    site: u32,
    observed: ValueId,
    candidate: i64,
}

struct NativeProfileRuntime<'module> {
    ensure: NativeFunction<'module>,
    increment: NativeFunction<'module>,
    add: NativeFunction<'module>,
    observe_u32: NativeFunction<'module>,
    observe_trip: NativeFunction<'module>,
    candidate_i64: NativeFunction<'module>,
    flush_control: NativeFunction<'module>,
    entries: BTreeMap<FunctionId, u32>,
    edges: BTreeMap<(FunctionId, BlockId, BlockId), u32>,
    loops: Vec<NativeProfileLoop>,
    slice_lengths: BTreeMap<(FunctionId, InstructionId), (u32, ValueId)>,
    candidates: BTreeMap<(FunctionId, InstructionId), NativeProfileCandidate>,
}

fn validate_native_profile_generation(
    kir: &KirModule,
    profile: &NativeProfileGeneration,
) -> Result<(), NativeError> {
    if profile.plan.mode != CkProfileKirMode::Generate {
        return Err(lowering_error(
            "native profile generation requires generate-mode KIR",
        ));
    }
    validate_ck_profile_kir_plan(&profile.plan)
        .map_err(|error| lowering_error(error.to_string()))?;
    if profile.plan.module != *kir {
        return Err(lowering_error(
            "profile generation plan does not match the verified KIR artifact",
        ));
    }
    if profile.identity.module.pre_profile_kir_digest != profile.plan.pre_profile_kir_digest
        || profile.identity.module.site_table_digest != profile.plan.site_table_digest
    {
        return Err(lowering_error(
            "profile identity does not match the generation topology",
        ));
    }
    let expected_runtime = decode_profile_runtime_identity()?;
    if profile.identity.compiler.profile_runtime_identity != expected_runtime {
        return Err(lowering_error("profile runtime identity is stale"));
    }
    let topology = match kir.config.consumer {
        KirConsumer::NativeExecutable => CkProfileTopology::NativeExecutable,
        KirConsumer::NativeLibrary => CkProfileTopology::NativeLibrary,
        KirConsumer::C | KirConsumer::WebAssembly | KirConsumer::Inspection => {
            return Err(lowering_error(
                "profile generation requires a Native consumer",
            ));
        }
    };
    if profile.identity.modes.topology != topology || profile.identity.modes.sanitizer {
        return Err(lowering_error(
            "profile identity consumer or sanitizer mode is incompatible",
        ));
    }
    profile
        .identity
        .digest()
        .map_err(|error| lowering_error(error.to_string()))?;
    Ok(())
}

fn decode_profile_runtime_identity() -> Result<[u8; 32], NativeError> {
    let text = crate::backend::native_runtime::NATIVE_PROFILE_RUNTIME_SHA256.as_bytes();
    if text.len() != 64 {
        return Err(lowering_error("profile runtime digest is malformed"));
    }
    let mut output = [0; 32];
    for (index, byte) in output.iter_mut().enumerate() {
        let high = decode_hex_nibble(text[index * 2])?;
        let low = decode_hex_nibble(text[index * 2 + 1])?;
        *byte = high << 4 | low;
    }
    Ok(output)
}

fn decode_hex_nibble(byte: u8) -> Result<u8, NativeError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(lowering_error("profile runtime digest is malformed")),
    }
}

fn add_native_profile_support<'module, 'context>(
    context: &'context NativeContext,
    module: &'module NativeModule<'context>,
    types: &TypeRegistry<'context>,
    profile: &NativeProfileGeneration,
) -> Result<NativeProfileRuntime<'module>, NativeError> {
    let template =
        create_profile_shard_template(profile.identity.clone(), profile.plan.sites.clone())
            .map_err(|error| lowering_error(error.to_string()))?;
    let directory_text = profile
        .directory
        .path
        .to_str()
        .ok_or_else(|| lowering_error("profile generation directory is not canonical UTF-8"))?;
    if directory_text.as_bytes().contains(&0) {
        return Err(lowering_error("profile generation directory contains NUL"));
    }
    let mut directory_bytes = directory_text.as_bytes().to_vec();
    directory_bytes.push(0);
    let shard = module.add_global_bytes("__ck_profile_shard", &template.bytes, true, 8)?;
    let counter_offsets = module.add_global_u32_array(
        "__ck_profile_counter_offsets",
        &template.counter_offsets,
        4,
    )?;
    let site_first =
        module.add_global_u32_array("__ck_profile_site_first", &template.site_first_counters, 4)?;
    let site_counts = module.add_global_u32_array(
        "__ck_profile_site_counts",
        &template.site_counter_counts,
        4,
    )?;
    let site_saturation = module.add_global_u32_array(
        "__ck_profile_site_saturation",
        &template.site_saturation_offsets,
        4,
    )?;
    let directory =
        module.add_global_bytes("__ck_profile_directory", &directory_bytes, false, 1)?;
    let initialize = module.add_function(
        "__ck_profile_initialize",
        types.i32,
        &[
            types.pointer,
            types.i64,
            types.pointer,
            types.i32,
            types.pointer,
            types.pointer,
            types.pointer,
            types.i32,
            types.i32,
            types.i32,
            types.i32,
            types.pointer,
            types.i32,
            types.i64,
            types.i64,
        ],
        true,
    )?;
    let increment =
        module.add_function("__ck_profile_increment", types.void, &[types.i32], true)?;
    let add = module.add_function(
        "__ck_profile_add",
        types.void,
        &[types.i32, types.i64],
        true,
    )?;
    let observe_u32 = module.add_function(
        "__ck_profile_observe_u32",
        types.void,
        &[types.i32, types.i32],
        true,
    )?;
    let observe_trip = module.add_function(
        "__ck_profile_observe_trip",
        types.void,
        &[types.i32, types.i64],
        true,
    )?;
    let candidate_i64 = module.add_function(
        "__ck_profile_candidate_i64",
        types.void,
        &[types.i32, types.i64, types.i64],
        true,
    )?;
    let runtime_flush = module.add_function("__ck_profile_flush", types.i32, &[], true)?;

    let ensure = module.add_function("__ck_profile_ensure", types.i32, &[], false)?;
    let entry = ensure.append_block("entry")?;
    let mut builder = NativeBuilder::new(context, module)?;
    builder.position(entry)?;
    let constants = [
        template.bytes.len().to_string(),
        template.counter_offsets.len().to_string(),
        template.site_first_counters.len().to_string(),
        template.run_id_offset.to_string(),
        template.overflow_flag_offset.to_string(),
        template.digest_offset.to_string(),
        directory_text.len().to_string(),
        profile.directory.identity.first.to_string(),
        profile.directory.identity.second.to_string(),
    ];
    let shard_length = builder.const_int(types.i64, &constants[0])?;
    let counter_count = builder.const_int(types.i32, &constants[1])?;
    let site_count = builder.const_int(types.i32, &constants[2])?;
    let run_id_offset = builder.const_int(types.i32, &constants[3])?;
    let overflow_flag_offset = builder.const_int(types.i32, &constants[4])?;
    let digest_offset = builder.const_int(types.i32, &constants[5])?;
    let directory_length = builder.const_int(types.i32, &constants[6])?;
    let identity_first = builder.const_int(types.i64, &constants[7])?;
    let identity_second = builder.const_int(types.i64, &constants[8])?;
    let status = builder.call(
        initialize,
        &[
            shard,
            shard_length,
            counter_offsets,
            counter_count,
            site_first,
            site_counts,
            site_saturation,
            site_count,
            run_id_offset,
            overflow_flag_offset,
            digest_offset,
            directory,
            directory_length,
            identity_first,
            identity_second,
        ],
        "ck.profile.init.status",
    )?;
    builder.return_value(status)?;

    let flush_name = profile
        .flush_symbol()
        .map_err(|error| lowering_error(error.to_string()))?;
    let flush_control = module.add_function(&flush_name, types.i32, &[], true)?;
    let entry = flush_control.append_block("entry")?;
    let failed = flush_control.append_block("initialization.failed")?;
    let ready = flush_control.append_block("ready")?;
    let mut builder = NativeBuilder::new(context, module)?;
    builder.position(entry)?;
    let status = builder.call(ensure, &[], "ck.profile.ensure.status")?;
    let zero = builder.const_int(types.i32, "0")?;
    let rejected = builder.compare(
        BridgeCompareOp::IcmpNe,
        status,
        zero,
        "ck.profile.ensure.failed",
    )?;
    builder.cond_branch(rejected, failed, ready)?;
    builder.position(failed)?;
    builder.return_value(status)?;
    builder.position(ready)?;
    let status = builder.call(runtime_flush, &[], "ck.profile.flush.status")?;
    builder.return_value(status)?;

    let mut runtime = NativeProfileRuntime {
        ensure,
        increment,
        add,
        observe_u32,
        observe_trip,
        candidate_i64,
        flush_control,
        entries: BTreeMap::new(),
        edges: BTreeMap::new(),
        loops: Vec::new(),
        slice_lengths: BTreeMap::new(),
        candidates: BTreeMap::new(),
    };
    for (site, annotation) in profile.plan.annotations.iter().enumerate() {
        let site = u32::try_from(site).map_err(|_| lowering_error("profile site overflow"))?;
        match &annotation.event {
            CkProfileEvent::FunctionEntry { function, .. } => {
                runtime.entries.insert(*function, site);
            }
            CkProfileEvent::Edge { function, from, to } => {
                runtime.edges.insert((*function, *from, *to), site);
            }
            CkProfileEvent::LoopTrip {
                function,
                header,
                latches,
                exits,
                ..
            } => runtime.loops.push(NativeProfileLoop {
                site,
                function: *function,
                header: *header,
                latches: latches.clone(),
                exits: exits.clone(),
            }),
            CkProfileEvent::SliceLength {
                function,
                instruction,
                value,
                ..
            } => {
                runtime
                    .slice_lengths
                    .insert((*function, *instruction), (site, *value));
            }
            CkProfileEvent::CandidateConstant {
                function,
                instruction,
                observed,
                ..
            } => {
                let CkProfileSiteKind::CandidateConstant { candidates, .. } =
                    &annotation.descriptor.kind
                else {
                    return Err(lowering_error("candidate profile descriptor is invalid"));
                };
                let [candidate] = candidates.as_slice() else {
                    return Err(lowering_error(
                        "native candidate instrumentation requires one canonical candidate",
                    ));
                };
                runtime.candidates.insert(
                    (*function, *instruction),
                    NativeProfileCandidate {
                        site,
                        observed: *observed,
                        candidate: *candidate,
                    },
                );
            }
        }
    }
    Ok(runtime)
}

struct NativeKirFacts<'a> {
    wrap_proofs: &'a WrapProofMap,
    contract_assumes: &'a BTreeMap<FunctionId, Vec<ContractAssume>>,
    scoped_alias_facts: &'a ScopedAliasFactMap,
    contract_checks: &'a ContractCheckMap,
}

struct KirLoweringEnvironment<'module, 'context, 'a> {
    types: &'a TypeRegistry<'context>,
    functions: &'a HashMap<String, NativeFunction<'module>>,
    layout: &'a LlvmStructLayout,
    structs: &'a [MirStruct],
    status_abi: bool,
    facts: &'a NativeKirFacts<'a>,
    profile: Option<&'a NativeProfileRuntime<'module>>,
    pgo_branches: &'a BTreeMap<(FunctionId, BlockId), (u64, u64)>,
}

type ScopedAliasFactMap = BTreeMap<FunctionId, Vec<(FactId, ValueId, ValueId)>>;
type ContractCheckMap = BTreeMap<FunctionId, Vec<(FactId, ContractFactPredicate)>>;

fn collect_contract_checks(module: &KirModule, result: &KirPassManagerResult) -> ContractCheckMap {
    if module.config.sanitizer_mode != KirSanitizerMode::Contracts {
        return BTreeMap::new();
    }
    let Some(contracts) = result.contract_facts.as_ref() else {
        return BTreeMap::new();
    };
    let mut checks: ContractCheckMap = BTreeMap::new();
    for fact in contracts.facts().facts() {
        let FactScope::FunctionEntry(function) = fact.scope else {
            continue;
        };
        let FactPredicate::Contract(predicate) = &fact.predicate else {
            continue;
        };
        if matches!(predicate, ContractFactPredicate::EffectCeiling { .. }) {
            continue;
        }
        checks
            .entry(function)
            .or_default()
            .push((fact.id, predicate.clone()));
    }
    checks
}

fn collect_scoped_alias_facts(
    module: &KirModule,
    result: &KirPassManagerResult,
) -> ScopedAliasFactMap {
    let Some(contracts) = result.contract_facts.as_ref() else {
        return BTreeMap::new();
    };
    let mut collected = BTreeMap::new();
    for function in &module.functions {
        let params = function
            .params
            .iter()
            .map(|param| param.value)
            .collect::<HashSet<_>>();
        let facts = contracts
            .facts()
            .facts()
            .iter()
            .filter(|fact| matches!(fact.scope, FactScope::FunctionEntry(id) if id == function.id))
            .filter_map(|fact| match &fact.predicate {
                FactPredicate::Contract(ContractFactPredicate::NoAlias { left, right })
                    if params.contains(left) && params.contains(right) =>
                {
                    Some((fact.id, *left, *right))
                }
                _ => None,
            })
            .filter(|(_, left, right)| function_emits_scoped_alias(function, *left, *right))
            .collect::<Vec<_>>();
        if !facts.is_empty() {
            collected.insert(function.id, facts);
        }
    }
    collected
}

fn function_emits_scoped_alias(function: &KirFunction, left: ValueId, right: ValueId) -> bool {
    function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .filter_map(|instruction| match &instruction.kind {
            KirInstructionKind::Load { place } | KirInstructionKind::Store { place, .. } => {
                Some(place)
            }
            _ => None,
        })
        .filter_map(|place| root_parameter_for_region(function, kir_place_region(place)))
        .any(|root| root == left || root == right)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FunctionMemoryEffect {
    effects: BridgeMemoryEffects,
    property: NativeFactProperty,
}

fn collect_contract_memory_effects(
    module: &KirModule,
    result: &KirPassManagerResult,
) -> BTreeMap<FunctionId, FunctionMemoryEffect> {
    let Some(contracts) = result.contract_facts.as_ref() else {
        return BTreeMap::new();
    };
    let mut collected = BTreeMap::new();
    for function in &module.functions {
        if (status_abi(module) && function.return_type != MirType::Void)
            || function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .any(|instruction| {
                    matches!(
                        instruction.kind,
                        KirInstructionKind::Call { .. } | KirInstructionKind::RuntimeCall { .. }
                    )
                })
        {
            continue;
        }
        let candidate = contracts.facts().facts().iter().find_map(|fact| {
            if !matches!(fact.scope, FactScope::FunctionEntry(id) if id == function.id) {
                return None;
            }
            let FactPredicate::Contract(ContractFactPredicate::EffectCeiling { is_none, items }) =
                &fact.predicate
            else {
                return None;
            };
            let effects = if *is_none
                || items
                    .iter()
                    .all(|(_, effect)| *effect == ContractEffectKind::None)
            {
                BridgeMemoryEffects::None
            } else if items
                .iter()
                .all(|(_, effect)| *effect == ContractEffectKind::Read)
            {
                BridgeMemoryEffects::Read
            } else if items
                .iter()
                .all(|(_, effect)| *effect == ContractEffectKind::Write)
            {
                BridgeMemoryEffects::Write
            } else {
                return None;
            };
            Some(FunctionMemoryEffect {
                effects,
                property: NativeFactProperty {
                    kind: NativeStrengtheningKind::MemoryEffects,
                    source: NativeFactSource::Fact(fact.id),
                    function: function.name.clone(),
                    subject: "function memory effects".to_string(),
                },
            })
        });
        if let Some(candidate) = candidate {
            collected.insert(function.id, candidate);
        }
    }
    collected
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AssumeOperand {
    Value(ValueId),
    SliceLength(ValueId),
    Constant(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ContractAssume {
    fact: FactId,
    op: MirCompareOp,
    left: AssumeOperand,
    right: AssumeOperand,
    type_node: MirType,
}

fn collect_contract_assumes(
    module: &KirModule,
    result: &KirPassManagerResult,
) -> BTreeMap<FunctionId, Vec<ContractAssume>> {
    let Some(contracts) = result.contract_facts.as_ref() else {
        return BTreeMap::new();
    };
    let mut collected = BTreeMap::new();
    for function in &module.functions {
        let types = value_types(function);
        let assumptions = contracts
            .facts()
            .facts()
            .iter()
            .filter(|fact| matches!(fact.scope, FactScope::FunctionEntry(id) if id == function.id))
            .filter_map(|fact| {
                let FactPredicate::Contract(ContractFactPredicate::Comparison {
                    operator,
                    left,
                    right,
                }) = &fact.predicate
                else {
                    return None;
                };
                let left = assume_operand(left)?;
                let right = assume_operand(right)?;
                let left_type = assume_operand_type(&left, &types);
                let right_type = assume_operand_type(&right, &types);
                let type_node = left_type.or(right_type)?;
                if left_type.is_some_and(|candidate| candidate != type_node)
                    || right_type.is_some_and(|candidate| candidate != type_node)
                    || !matches!(
                        type_node,
                        MirType::Primitive(
                            MirPrimitiveTypeName::I32
                                | MirPrimitiveTypeName::I64
                                | MirPrimitiveTypeName::U32
                                | MirPrimitiveTypeName::U64
                        )
                    )
                {
                    return None;
                }
                Some(ContractAssume {
                    fact: fact.id,
                    op: comparison_operator(operator)?,
                    left,
                    right,
                    type_node: type_node.clone(),
                })
            })
            .collect::<Vec<_>>();
        if !assumptions.is_empty() {
            collected.insert(function.id, assumptions);
        }
    }
    collected
}

fn assume_operand(expression: &ContractFactAffineExpression) -> Option<AssumeOperand> {
    match expression.terms.as_slice() {
        [] => Some(AssumeOperand::Constant(expression.constant.to_string())),
        [term] if term.coefficient == 1.into() && expression.constant == 0.into() => {
            Some(match term.term {
                ContractFactAffineTerm::Value(value) => AssumeOperand::Value(value),
                ContractFactAffineTerm::SliceLength(value) => AssumeOperand::SliceLength(value),
            })
        }
        _ => None,
    }
}

fn assume_operand_type<'a>(
    operand: &AssumeOperand,
    types: &'a BTreeMap<ValueId, MirType>,
) -> Option<&'a MirType> {
    match operand {
        AssumeOperand::Value(value) => types.get(value),
        AssumeOperand::SliceLength(_) => Some(&MirType::Primitive(MirPrimitiveTypeName::U32)),
        AssumeOperand::Constant(_) => None,
    }
}

fn comparison_operator(operator: &str) -> Option<MirCompareOp> {
    match operator {
        "==" => Some(MirCompareOp::Eq),
        "!=" => Some(MirCompareOp::Ne),
        "<" => Some(MirCompareOp::Lt),
        "<=" => Some(MirCompareOp::Le),
        ">" => Some(MirCompareOp::Gt),
        ">=" => Some(MirCompareOp::Ge),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParamAttributeKind {
    NoAlias,
    ReadOnly,
    WriteOnly,
    Alignment(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParamFactAttribute {
    physical_index: usize,
    attribute: ParamAttributeKind,
    property: NativeFactProperty,
}

fn apply_param_attribute(
    function: NativeFunction<'_>,
    attribute: &ParamFactAttribute,
) -> Result<(), NativeError> {
    match attribute.attribute {
        ParamAttributeKind::NoAlias => function.add_param_noalias(attribute.physical_index),
        ParamAttributeKind::ReadOnly => function.add_param_readonly(attribute.physical_index),
        ParamAttributeKind::WriteOnly => function.add_param_writeonly(attribute.physical_index),
        ParamAttributeKind::Alignment(alignment) => {
            function.add_param_alignment(attribute.physical_index, alignment)
        }
    }
}

fn collect_contract_attributes(
    module: &KirModule,
    result: &KirPassManagerResult,
) -> BTreeMap<FunctionId, Vec<ParamFactAttribute>> {
    let Some(contracts) = result.contract_facts.as_ref() else {
        return BTreeMap::new();
    };
    let mut collected = BTreeMap::new();
    for function in &module.functions {
        let entry_facts = contracts
            .facts()
            .facts()
            .iter()
            .filter(|fact| matches!(fact.scope, FactScope::FunctionEntry(id) if id == function.id))
            .collect::<Vec<_>>();
        let pointer_params = function
            .params
            .iter()
            .filter(|param| is_pointer_like(&param.type_node))
            .map(|param| param.value)
            .collect::<Vec<_>>();
        let noalias_facts = entry_facts
            .iter()
            .filter_map(|fact| match &fact.predicate {
                FactPredicate::Contract(ContractFactPredicate::NoAlias { left, right }) => {
                    Some((fact.id, normalized_pair(*left, *right)))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let complete_noalias = pointer_params.len() == 2
            && pointer_params.iter().enumerate().all(|(left_index, left)| {
                pointer_params.iter().skip(left_index + 1).all(|right| {
                    let pair = normalized_pair(*left, *right);
                    noalias_facts
                        .iter()
                        .any(|(_, candidate)| *candidate == pair)
                })
            })
            && !is_pointer_like(&function.return_type)
            && !(status_abi(module) && function.return_type != MirType::Void)
            && !function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .any(|instruction| {
                    matches!(
                        instruction.kind,
                        KirInstructionKind::Call { .. } | KirInstructionKind::RuntimeCall { .. }
                    )
                });
        let mut attributes = Vec::new();
        if complete_noalias {
            for value in &pointer_params {
                if let (Some(index), Some((source, _))) = (
                    physical_parameter_index(function, *value),
                    noalias_facts
                        .iter()
                        .find(|(_, pair)| pair.0 == *value || pair.1 == *value),
                ) {
                    attributes.push(ParamFactAttribute {
                        physical_index: index,
                        attribute: ParamAttributeKind::NoAlias,
                        property: NativeFactProperty {
                            kind: NativeStrengtheningKind::ParameterNoAlias,
                            source: NativeFactSource::Fact(*source),
                            function: function.name.clone(),
                            subject: parameter_subject(function, *value),
                        },
                    });
                }
            }
        }
        for fact in &entry_facts {
            match &fact.predicate {
                FactPredicate::Contract(ContractFactPredicate::Aligned { pointer, alignment }) => {
                    let value = match pointer {
                        ContractFactPointer::Value(value)
                        | ContractFactPointer::SliceData(value) => *value,
                    };
                    if let Some(index) = physical_parameter_index(function, value) {
                        attributes.push(ParamFactAttribute {
                            physical_index: index,
                            attribute: ParamAttributeKind::Alignment(*alignment),
                            property: NativeFactProperty {
                                kind: NativeStrengtheningKind::Alignment,
                                source: NativeFactSource::Fact(fact.id),
                                function: function.name.clone(),
                                subject: parameter_subject(function, value),
                            },
                        });
                    }
                }
                FactPredicate::Contract(ContractFactPredicate::EffectCeiling { items, .. }) => {
                    for (value, effect) in items {
                        let Some(index) = physical_parameter_index(function, *value) else {
                            continue;
                        };
                        let (attribute, kind) = match effect {
                            ContractEffectKind::Read => (
                                ParamAttributeKind::ReadOnly,
                                NativeStrengtheningKind::ReadOnly,
                            ),
                            ContractEffectKind::Write => (
                                ParamAttributeKind::WriteOnly,
                                NativeStrengtheningKind::WriteOnly,
                            ),
                            ContractEffectKind::None | ContractEffectKind::ReadWrite => continue,
                        };
                        attributes.push(ParamFactAttribute {
                            physical_index: index,
                            attribute,
                            property: NativeFactProperty {
                                kind,
                                source: NativeFactSource::Fact(fact.id),
                                function: function.name.clone(),
                                subject: parameter_subject(function, *value),
                            },
                        });
                    }
                }
                _ => {}
            }
        }
        if !attributes.is_empty() {
            collected.insert(function.id, attributes);
        }
    }
    collected
}

fn normalized_pair(left: ValueId, right: ValueId) -> (ValueId, ValueId) {
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}

fn is_pointer_like(type_node: &MirType) -> bool {
    matches!(type_node, MirType::Pointer(_) | MirType::Slice(_))
}

fn physical_parameter_index(function: &KirFunction, value: ValueId) -> Option<usize> {
    let mut physical = 0;
    for param in &function.params {
        if param.value == value {
            return is_pointer_like(&param.type_node).then_some(physical);
        }
        physical += usize::from(matches!(param.type_node, MirType::Slice(_))) + 1;
    }
    None
}

fn parameter_subject(function: &KirFunction, value: ValueId) -> String {
    function
        .params
        .iter()
        .find(|param| param.value == value)
        .map_or_else(|| format!("v{}", value.index()), |param| param.name.clone())
}

type WrapProofMap = BTreeMap<(FunctionId, InstructionId), (ProofId, NativeStrengtheningKind)>;

fn collect_wrap_proofs(module: &KirModule, result: &KirPassManagerResult) -> WrapProofMap {
    let mut proofs = BTreeMap::new();
    for elimination in &result.eliminated_guards {
        let Some(proof) = elimination.proof else {
            continue;
        };
        let Some(instruction) = module
            .functions
            .iter()
            .find(|function| function.id == elimination.function)
            .into_iter()
            .flat_map(|function| &function.blocks)
            .flat_map(|block| &block.instructions)
            .find(|instruction| instruction.id == elimination.condition_instruction)
        else {
            continue;
        };
        let KirInstructionKind::Binary {
            op: MirBinaryOp::Add | MirBinaryOp::Sub | MirBinaryOp::Mul,
            semantics: KirArithmeticSemantics::Checked,
            ..
        } = instruction.kind
        else {
            continue;
        };
        let kind = match instruction.results.first().map(|result| &result.type_node) {
            Some(KirValueType::Scalar(MirType::Primitive(
                MirPrimitiveTypeName::U32 | MirPrimitiveTypeName::U64,
            ))) => NativeStrengtheningKind::NoUnsignedWrap,
            Some(KirValueType::Scalar(MirType::Primitive(
                MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::I64,
            ))) => NativeStrengtheningKind::NoSignedWrap,
            _ => continue,
        };
        proofs.insert(
            (elimination.function, elimination.condition_instruction),
            (proof, kind),
        );
    }
    proofs
}

fn status_abi(module: &KirModule) -> bool {
    module.config.overflow_mode == KirOverflowMode::Checked
        || module.config.bounds_mode == KirBoundsMode::Checked
        || module.config.sanitizer_mode == KirSanitizerMode::Contracts
}

fn mir_shape(module: &KirModule) -> MirModule {
    MirModule {
        entry: module.entry.clone(),
        structs: module.structs.clone(),
        functions: module
            .functions
            .iter()
            .map(|function| MirFunction {
                name: function.name.clone(),
                exported: function.exported,
                params: function
                    .params
                    .iter()
                    .map(|param| MirParam {
                        name: param.name.clone(),
                        type_node: param.type_node.clone(),
                    })
                    .collect(),
                return_type: function.return_type.clone(),
                locals: Vec::new(),
                blocks: Vec::new(),
            })
            .collect(),
    }
}

fn physical_param_types<'context>(
    types: &TypeRegistry<'context>,
    params: &[KirParam],
) -> Result<Vec<NativeType<'context>>, NativeError> {
    let mut physical = Vec::new();
    for param in params {
        if matches!(param.type_node, MirType::Slice(_)) {
            physical.extend([types.pointer, types.i32]);
        } else {
            physical.push(types.get(&param.type_node)?);
        }
    }
    Ok(physical)
}

fn used_runtime_intrinsics(module: &KirModule) -> Vec<MirRuntimeIntrinsic> {
    let mut seen = HashSet::new();
    module
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.instructions)
        .filter_map(|instruction| match instruction.kind {
            KirInstructionKind::RuntimeCall { intrinsic, .. } if seen.insert(intrinsic) => {
                Some(intrinsic)
            }
            _ => None,
        })
        .collect()
}

#[derive(Clone, Copy)]
struct Storage<'module> {
    pointer: NativeValue<'module>,
}

struct KirFunctionLowerer<'module, 'context, 'a> {
    context: &'context NativeContext,
    builder: NativeBuilder<'module, 'context>,
    types: &'a TypeRegistry<'context>,
    functions: &'a HashMap<String, NativeFunction<'module>>,
    layout: &'a LlvmStructLayout,
    structs: &'a [MirStruct],
    handle: NativeFunction<'module>,
    function: &'a KirFunction,
    status_abi: bool,
    result_pointer: Option<NativeValue<'module>>,
    blocks: BTreeMap<BlockId, NativeBlock<'module>>,
    current_block: Option<NativeBlock<'module>>,
    storage: BTreeMap<ValueId, Storage<'module>>,
    guard_conditions: HashSet<ValueId>,
    facts: &'a NativeKirFacts<'a>,
    profile: Option<&'a NativeProfileRuntime<'module>>,
    pgo_branches: &'a BTreeMap<(FunctionId, BlockId), (u64, u64)>,
    loop_storage: BTreeMap<u32, Storage<'module>>,
    temporary: u32,
}

fn lower_function<'module, 'context>(
    context: &'context NativeContext,
    module: &'module NativeModule<'context>,
    function: &KirFunction,
    environment: &KirLoweringEnvironment<'module, 'context, '_>,
) -> Result<(), NativeError> {
    let handle = require_function(environment.functions, &function.name)?;
    let entry = handle.append_block("entry")?;
    let blocks = function
        .blocks
        .iter()
        .map(|block| {
            handle
                .append_block(&format!("kir.bb{}", block.id.index()))
                .map(|native| (block.id, native))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let mut lowerer = KirFunctionLowerer {
        context,
        builder: NativeBuilder::new(context, module)?,
        types: environment.types,
        functions: environment.functions,
        layout: environment.layout,
        structs: environment.structs,
        handle,
        function,
        status_abi: environment.status_abi,
        result_pointer: None,
        blocks,
        current_block: Some(entry),
        storage: BTreeMap::new(),
        guard_conditions: function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .filter_map(|instruction| match instruction.kind {
                KirInstructionKind::Guard { condition, .. } => Some(condition),
                _ => None,
            })
            .collect(),
        facts: environment.facts,
        profile: environment.profile,
        pgo_branches: environment.pgo_branches,
        loop_storage: BTreeMap::new(),
        temporary: 0,
    };
    lowerer.builder.position(entry)?;
    lowerer.allocate_values()?;
    lowerer.allocate_profile_loop_counters()?;
    lowerer.store_parameters()?;
    lowerer.emit_profile_function_entry()?;
    lowerer.emit_contract_checks()?;
    lowerer.emit_contract_assumes()?;
    let Some(first) = function.blocks.first() else {
        return if lowerer.status_abi {
            let ok = lowerer.status(0)?;
            lowerer.builder.return_value(ok)
        } else if function.return_type == MirType::Void {
            lowerer.builder.return_void()
        } else {
            Err(lowering_error(format!(
                "non-void KIR function '{}' has no blocks",
                function.name
            )))
        };
    };
    lowerer.builder.branch(lowerer.block(first.id)?)?;
    for block in &function.blocks {
        let native_block = lowerer.block(block.id)?;
        lowerer.builder.position(native_block)?;
        lowerer.current_block = Some(native_block);
        for instruction in &block.instructions {
            lowerer.instruction(instruction)?;
            lowerer.emit_profile_instruction(instruction)?;
        }
        lowerer.terminator(block.id, &block.terminator)?;
    }
    Ok(())
}

impl<'module, 'context> KirFunctionLowerer<'module, 'context, '_> {
    fn name(&mut self, prefix: &str) -> String {
        let name = format!("{prefix}.{}", self.temporary);
        self.temporary += 1;
        name
    }

    fn block(&self, id: BlockId) -> Result<NativeBlock<'module>, NativeError> {
        self.blocks
            .get(&id)
            .copied()
            .ok_or_else(|| lowering_error(format!("unknown KIR block b{}", id.index())))
    }

    fn allocate_values(&mut self) -> Result<(), NativeError> {
        for (value, type_node) in kir_value_types(self.function) {
            let pointer = self.builder.alloca(
                self.types.get_kir(&type_node)?,
                &format!("v{}.addr", value.index()),
            )?;
            self.storage.insert(value, Storage { pointer });
        }
        Ok(())
    }

    fn allocate_profile_loop_counters(&mut self) -> Result<(), NativeError> {
        let sites = self
            .profile
            .map(|profile| {
                profile
                    .loops
                    .iter()
                    .filter(|event| event.function == self.function.id)
                    .map(|event| event.site)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for site in sites {
            let pointer = self
                .builder
                .alloca(self.types.i64, &format!("ck.profile.loop.{site}"))?;
            let zero = self.builder.const_int(self.types.i64, "0")?;
            self.builder.store(zero, pointer)?;
            self.loop_storage.insert(site, Storage { pointer });
        }
        Ok(())
    }

    fn emit_profile_function_entry(&mut self) -> Result<(), NativeError> {
        let Some(profile) = self.profile else {
            return Ok(());
        };
        let ensure = profile.ensure;
        let increment = profile.increment;
        let site = profile.entries.get(&self.function.id).copied();
        let _ = self.builder.call(ensure, &[], "ck.profile.ensure.status")?;
        if let Some(site) = site {
            let site = self.builder.const_int(self.types.i32, &site.to_string())?;
            let _ = self.builder.call(increment, &[site], "")?;
        }
        Ok(())
    }

    fn emit_profile_instruction(
        &mut self,
        instruction: &KirInstruction,
    ) -> Result<(), NativeError> {
        let Some(profile) = self.profile else {
            return Ok(());
        };
        let slice = profile
            .slice_lengths
            .get(&(self.function.id, instruction.id))
            .copied();
        let candidate = profile
            .candidates
            .get(&(self.function.id, instruction.id))
            .copied();
        let observe = profile.observe_u32;
        let candidate_function = profile.candidate_i64;
        if let Some((site, value)) = slice {
            let site = self.builder.const_int(self.types.i32, &site.to_string())?;
            let value = self.load(value)?;
            let _ = self.builder.call(observe, &[site, value], "")?;
        }
        if let Some(candidate) = candidate {
            let site = self
                .builder
                .const_int(self.types.i32, &candidate.site.to_string())?;
            let observed_type = self.type_of(candidate.observed)?.clone();
            let observed = self.load(candidate.observed)?;
            let observed = match observed_type {
                MirType::Primitive(MirPrimitiveTypeName::I32) => {
                    let name = self.name("ck.profile.candidate.i64");
                    self.builder
                        .cast(BridgeCastOp::Sext, observed, self.types.i64, &name)?
                }
                MirType::Primitive(MirPrimitiveTypeName::U32) => {
                    let name = self.name("ck.profile.candidate.u64");
                    self.builder
                        .cast(BridgeCastOp::Zext, observed, self.types.i64, &name)?
                }
                MirType::Primitive(MirPrimitiveTypeName::I64 | MirPrimitiveTypeName::U64) => {
                    observed
                }
                _ => {
                    return Err(lowering_error(
                        "candidate profile observation is not an integer",
                    ));
                }
            };
            let expected = self
                .builder
                .const_int(self.types.i64, &candidate.candidate.to_string())?;
            let _ = self
                .builder
                .call(candidate_function, &[site, observed, expected], "")?;
        }
        Ok(())
    }

    fn store_parameters(&mut self) -> Result<(), NativeError> {
        let mut physical = 0;
        for param in &self.function.params {
            if matches!(param.type_node, MirType::Slice(_)) {
                let data = self
                    .handle
                    .param(physical, &format!("{}.data", param.name))?;
                let len = self
                    .handle
                    .param(physical + 1, &format!("{}.len", param.name))?;
                physical += 2;
                let slice = self.make_slice(data, len)?;
                self.store(param.value, slice)?;
            } else {
                let value = self.handle.param(physical, &param.name)?;
                physical += 1;
                self.store(param.value, value)?;
            }
        }
        if self.status_abi && self.function.return_type != MirType::Void {
            let result = self.handle.param(physical, "ck_return")?;
            self.result_pointer = Some(result);
            let zero = self.builder.const_int(self.types.i64, "0")?;
            let name = self.name("result.null");
            let null =
                self.builder
                    .cast(BridgeCastOp::IntToPtr, zero, self.types.pointer, &name)?;
            let name = self.name("result.is_null");
            let failed = self
                .builder
                .compare(BridgeCompareOp::IcmpEq, result, null, &name)?;
            self.guard_status(failed, self.status(3)?)?;
        }
        Ok(())
    }

    fn emit_contract_checks(&mut self) -> Result<(), NativeError> {
        let checks = self
            .facts
            .contract_checks
            .get(&self.function.id)
            .cloned()
            .unwrap_or_default();
        for (fact, predicate) in checks {
            let failed = self.contract_predicate_failed(&predicate, fact)?;
            self.guard_status(failed, self.status(7)?)?;
        }
        Ok(())
    }

    fn contract_predicate_failed(
        &mut self,
        predicate: &ContractFactPredicate,
        fact: FactId,
    ) -> Result<NativeValue<'module>, NativeError> {
        match predicate {
            ContractFactPredicate::Comparison {
                operator,
                left,
                right,
            } => {
                let bits = contract_integer_width(&[left, right], None);
                let left = self.contract_affine_value(left, bits, fact)?;
                let right = self.contract_affine_value(right, bits, fact)?;
                let name = self.name(&format!("contract.sanitize.fact{}.holds", fact.index()));
                let holds =
                    self.builder
                        .compare(contract_compare_op(operator)?, left, right, &name)?;
                self.invert_contract_condition(holds, fact)
            }
            ContractFactPredicate::MultipleOf { value, modulus } => {
                let bits = contract_integer_width(&[value], Some(modulus));
                let type_node = NativeType::int(self.context, bits)?;
                let value = self.contract_affine_value(value, bits, fact)?;
                let modulus = self.builder.const_int(type_node, &modulus.to_string())?;
                let name = self.name(&format!("contract.sanitize.fact{}.remainder", fact.index()));
                let remainder = self
                    .builder
                    .binary(BridgeBinaryOp::SRem, value, modulus, &name)?;
                let zero = self.builder.const_int(type_node, "0")?;
                let name = self.name(&format!("contract.sanitize.fact{}.failed", fact.index()));
                self.builder
                    .compare(BridgeCompareOp::IcmpNe, remainder, zero, &name)
            }
            ContractFactPredicate::NoAlias { left, right } => {
                self.contract_noalias_failed(*left, *right, fact)
            }
            ContractFactPredicate::Aligned { pointer, alignment } => {
                let pointer = match pointer {
                    ContractFactPointer::Value(value) => self.load(*value)?,
                    ContractFactPointer::SliceData(value) => {
                        let slice = self.load(*value)?;
                        let name = self.name(&format!(
                            "contract.sanitize.fact{}.aligned.data",
                            fact.index()
                        ));
                        self.builder.extract_value(slice, 0, &name)?
                    }
                };
                let name = self.name(&format!(
                    "contract.sanitize.fact{}.aligned.address",
                    fact.index()
                ));
                let address =
                    self.builder
                        .cast(BridgeCastOp::PtrToInt, pointer, self.types.i64, &name)?;
                let divisor = self
                    .builder
                    .const_int(self.types.i64, &alignment.to_string())?;
                let name = self.name(&format!(
                    "contract.sanitize.fact{}.aligned.remainder",
                    fact.index()
                ));
                let remainder =
                    self.builder
                        .binary(BridgeBinaryOp::URem, address, divisor, &name)?;
                let zero = self.builder.const_int(self.types.i64, "0")?;
                let name = self.name(&format!(
                    "contract.sanitize.fact{}.aligned.failed",
                    fact.index()
                ));
                self.builder
                    .compare(BridgeCompareOp::IcmpNe, remainder, zero, &name)
            }
            ContractFactPredicate::EffectCeiling { .. } => Err(lowering_error(
                "effect ceilings are compile-time-only contract predicates",
            )),
        }
    }

    fn contract_affine_value(
        &mut self,
        expression: &ContractFactAffineExpression,
        bits: u32,
        fact: FactId,
    ) -> Result<NativeValue<'module>, NativeError> {
        let wide = NativeType::int(self.context, bits)?;
        let mut value = self
            .builder
            .const_int(wide, &expression.constant.to_string())?;
        for term in &expression.terms {
            let operand = match term.term {
                ContractFactAffineTerm::Value(value) => {
                    let operand = self.load(value)?;
                    let op = match self.type_of(value)? {
                        MirType::Primitive(
                            MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::I64,
                        ) => BridgeCastOp::Sext,
                        MirType::Primitive(
                            MirPrimitiveTypeName::U32 | MirPrimitiveTypeName::U64,
                        ) => BridgeCastOp::Zext,
                        _ => {
                            return Err(lowering_error(
                                "contract affine operand is not an integer value",
                            ));
                        }
                    };
                    let name =
                        self.name(&format!("contract.sanitize.fact{}.operand", fact.index()));
                    self.builder.cast(op, operand, wide, &name)?
                }
                ContractFactAffineTerm::SliceLength(value) => {
                    let slice = self.load(value)?;
                    let name =
                        self.name(&format!("contract.sanitize.fact{}.slice.len", fact.index()));
                    let len = self.builder.extract_value(slice, 1, &name)?;
                    let name = self.name(&format!(
                        "contract.sanitize.fact{}.slice.len.wide",
                        fact.index()
                    ));
                    self.builder.cast(BridgeCastOp::Zext, len, wide, &name)?
                }
            };
            let coefficient = self
                .builder
                .const_int(wide, &term.coefficient.to_string())?;
            let name = self.name(&format!("contract.sanitize.fact{}.product", fact.index()));
            let product = self
                .builder
                .binary(BridgeBinaryOp::Mul, coefficient, operand, &name)?;
            let name = self.name(&format!("contract.sanitize.fact{}.sum", fact.index()));
            value = self
                .builder
                .binary(BridgeBinaryOp::Add, value, product, &name)?;
        }
        Ok(value)
    }

    fn contract_noalias_failed(
        &mut self,
        left: ValueId,
        right: ValueId,
        fact: FactId,
    ) -> Result<NativeValue<'module>, NativeError> {
        let left_element = match self.type_of(left)? {
            MirType::Slice(element) => element.as_ref(),
            _ => {
                return Err(lowering_error(
                    "contract noalias left operand is not a slice",
                ));
            }
        };
        let right_element = match self.type_of(right)? {
            MirType::Slice(element) => element.as_ref(),
            _ => {
                return Err(lowering_error(
                    "contract noalias right operand is not a slice",
                ));
            }
        };
        let left_size = mir_type_layout(left_element, self.structs)?.0;
        let right_size = mir_type_layout(right_element, self.structs)?.0;
        let left_slice = self.load(left)?;
        let right_slice = self.load(right)?;
        let left_data_name =
            self.name(&format!("contract.sanitize.fact{}.left.data", fact.index()));
        let left_data = self.builder.extract_value(left_slice, 0, &left_data_name)?;
        let left_len_name = self.name(&format!("contract.sanitize.fact{}.left.len", fact.index()));
        let left_len = self.builder.extract_value(left_slice, 1, &left_len_name)?;
        let right_data_name = self.name(&format!(
            "contract.sanitize.fact{}.right.data",
            fact.index()
        ));
        let right_data = self
            .builder
            .extract_value(right_slice, 0, &right_data_name)?;
        let right_len_name =
            self.name(&format!("contract.sanitize.fact{}.right.len", fact.index()));
        let right_len = self
            .builder
            .extract_value(right_slice, 1, &right_len_name)?;

        let address_width = NativeType::int(self.context, 192)?;
        let left_address_name = self.name(&format!(
            "contract.sanitize.fact{}.left.address",
            fact.index()
        ));
        let left_address = self.builder.cast(
            BridgeCastOp::PtrToInt,
            left_data,
            self.types.i64,
            &left_address_name,
        )?;
        let left_address_wide_name = self.name(&format!(
            "contract.sanitize.fact{}.left.address.wide",
            fact.index()
        ));
        let left_address = self.builder.cast(
            BridgeCastOp::Zext,
            left_address,
            address_width,
            &left_address_wide_name,
        )?;
        let right_address_name = self.name(&format!(
            "contract.sanitize.fact{}.right.address",
            fact.index()
        ));
        let right_address = self.builder.cast(
            BridgeCastOp::PtrToInt,
            right_data,
            self.types.i64,
            &right_address_name,
        )?;
        let right_address_wide_name = self.name(&format!(
            "contract.sanitize.fact{}.right.address.wide",
            fact.index()
        ));
        let right_address = self.builder.cast(
            BridgeCastOp::Zext,
            right_address,
            address_width,
            &right_address_wide_name,
        )?;
        let left_len_wide_name = self.name(&format!(
            "contract.sanitize.fact{}.left.len.wide",
            fact.index()
        ));
        let left_len_wide = self.builder.cast(
            BridgeCastOp::Zext,
            left_len,
            address_width,
            &left_len_wide_name,
        )?;
        let right_len_wide_name = self.name(&format!(
            "contract.sanitize.fact{}.right.len.wide",
            fact.index()
        ));
        let right_len_wide = self.builder.cast(
            BridgeCastOp::Zext,
            right_len,
            address_width,
            &right_len_wide_name,
        )?;
        let left_size = self
            .builder
            .const_int(address_width, &left_size.to_string())?;
        let right_size = self
            .builder
            .const_int(address_width, &right_size.to_string())?;
        let left_bytes_name = self.name(&format!(
            "contract.sanitize.fact{}.left.bytes",
            fact.index()
        ));
        let left_bytes = self.builder.binary(
            BridgeBinaryOp::Mul,
            left_len_wide,
            left_size,
            &left_bytes_name,
        )?;
        let right_bytes_name = self.name(&format!(
            "contract.sanitize.fact{}.right.bytes",
            fact.index()
        ));
        let right_bytes = self.builder.binary(
            BridgeBinaryOp::Mul,
            right_len_wide,
            right_size,
            &right_bytes_name,
        )?;
        let left_end_name = self.name(&format!("contract.sanitize.fact{}.left.end", fact.index()));
        let left_end = self.builder.binary(
            BridgeBinaryOp::Add,
            left_address,
            left_bytes,
            &left_end_name,
        )?;
        let right_end_name =
            self.name(&format!("contract.sanitize.fact{}.right.end", fact.index()));
        let right_end = self.builder.binary(
            BridgeBinaryOp::Add,
            right_address,
            right_bytes,
            &right_end_name,
        )?;
        let address_max = self
            .builder
            .const_int(address_width, "18446744073709551615")?;
        let left_invalid_name = self.name(&format!(
            "contract.sanitize.fact{}.left.interval.invalid",
            fact.index()
        ));
        let left_invalid = self.builder.compare(
            BridgeCompareOp::IcmpUgt,
            left_end,
            address_max,
            &left_invalid_name,
        )?;
        let right_invalid_name = self.name(&format!(
            "contract.sanitize.fact{}.right.interval.invalid",
            fact.index()
        ));
        let right_invalid = self.builder.compare(
            BridgeCompareOp::IcmpUgt,
            right_end,
            address_max,
            &right_invalid_name,
        )?;
        let left_before_right_name = self.name(&format!(
            "contract.sanitize.fact{}.left.before.right",
            fact.index()
        ));
        let left_before_right = self.builder.compare(
            BridgeCompareOp::IcmpUle,
            left_end,
            right_address,
            &left_before_right_name,
        )?;
        let right_before_left_name = self.name(&format!(
            "contract.sanitize.fact{}.right.before.left",
            fact.index()
        ));
        let right_before_left = self.builder.compare(
            BridgeCompareOp::IcmpUle,
            right_end,
            left_address,
            &right_before_left_name,
        )?;
        let separated = self.contract_bool_or(left_before_right, right_before_left, fact)?;
        let overlap = self.invert_contract_condition(separated, fact)?;
        let zero = self.builder.const_int(self.types.i32, "0")?;
        let left_empty_name = self.name(&format!(
            "contract.sanitize.fact{}.left.empty",
            fact.index()
        ));
        let left_empty =
            self.builder
                .compare(BridgeCompareOp::IcmpEq, left_len, zero, &left_empty_name)?;
        let right_empty_name = self.name(&format!(
            "contract.sanitize.fact{}.right.empty",
            fact.index()
        ));
        let right_empty =
            self.builder
                .compare(BridgeCompareOp::IcmpEq, right_len, zero, &right_empty_name)?;
        let left_nonempty = self.invert_contract_condition(left_empty, fact)?;
        let right_nonempty = self.invert_contract_condition(right_empty, fact)?;
        let both_nonempty = self.contract_bool_and(left_nonempty, right_nonempty, fact)?;
        let overlapping_nonempty = self.contract_bool_and(both_nonempty, overlap, fact)?;
        let invalid = self.contract_bool_or(left_invalid, right_invalid, fact)?;
        self.contract_bool_or(invalid, overlapping_nonempty, fact)
    }

    fn invert_contract_condition(
        &mut self,
        condition: NativeValue<'module>,
        fact: FactId,
    ) -> Result<NativeValue<'module>, NativeError> {
        let false_value = self.builder.const_bool(false)?;
        let name = self.name(&format!("contract.sanitize.fact{}.inverted", fact.index()));
        self.builder
            .compare(BridgeCompareOp::IcmpEq, condition, false_value, &name)
    }

    fn contract_bool_or(
        &mut self,
        left: NativeValue<'module>,
        right: NativeValue<'module>,
        fact: FactId,
    ) -> Result<NativeValue<'module>, NativeError> {
        let true_value = self.builder.const_bool(true)?;
        let name = self.name(&format!("contract.sanitize.fact{}.or", fact.index()));
        self.builder.select(left, true_value, right, &name)
    }

    fn contract_bool_and(
        &mut self,
        left: NativeValue<'module>,
        right: NativeValue<'module>,
        fact: FactId,
    ) -> Result<NativeValue<'module>, NativeError> {
        let false_value = self.builder.const_bool(false)?;
        let name = self.name(&format!("contract.sanitize.fact{}.and", fact.index()));
        self.builder.select(left, right, false_value, &name)
    }

    fn emit_contract_assumes(&mut self) -> Result<(), NativeError> {
        let assumptions = self
            .facts
            .contract_assumes
            .get(&self.function.id)
            .cloned()
            .unwrap_or_default();
        for assumption in assumptions {
            let left = self.assume_operand(&assumption.left, &assumption.type_node)?;
            let right = self.assume_operand(&assumption.right, &assumption.type_node)?;
            let name = self.name("contract.assume.condition");
            let condition = self.builder.compare(
                compare_op(assumption.op, &assumption.type_node),
                left,
                right,
                &name,
            )?;
            self.builder.assume(condition)?;
        }
        Ok(())
    }

    fn assume_operand(
        &mut self,
        operand: &AssumeOperand,
        type_node: &MirType,
    ) -> Result<NativeValue<'module>, NativeError> {
        match operand {
            AssumeOperand::Value(value) => self.load(*value),
            AssumeOperand::SliceLength(value) => {
                let slice = self.load(*value)?;
                let name = self.name("contract.slice.len");
                self.builder.extract_value(slice, 1, &name)
            }
            AssumeOperand::Constant(value) => {
                self.builder.const_int(self.types.get(type_node)?, value)
            }
        }
    }

    fn instruction(&mut self, instruction: &KirInstruction) -> Result<(), NativeError> {
        match &instruction.kind {
            KirInstructionKind::Undef { .. } => {
                let result = &instruction.results[0];
                let value = self.builder.undef(self.types.get_kir(&result.type_node)?)?;
                self.store(result.value, value)
            }
            KirInstructionKind::ConstInt { value } => {
                let result = &instruction.results[0];
                let value = self
                    .builder
                    .const_int(self.types.get_kir(&result.type_node)?, value)?;
                self.store(result.value, value)
            }
            KirInstructionKind::ConstFloat { value } => {
                let result = &instruction.results[0];
                let value = self
                    .builder
                    .const_float(self.types.get_kir(&result.type_node)?, value)?;
                self.store(result.value, value)
            }
            KirInstructionKind::ConstBool { value } => {
                let result = instruction.results[0].value;
                let value = self.builder.const_bool(*value)?;
                self.store(result, value)
            }
            KirInstructionKind::Copy { value } => {
                let loaded = self.load(*value)?;
                self.store(instruction.results[0].value, loaded)
            }
            KirInstructionKind::Binary {
                op,
                left,
                right,
                semantics,
            } => self.binary(instruction, *op, *left, *right, *semantics),
            KirInstructionKind::Unary {
                op,
                operand,
                semantics,
            } => self.unary(instruction, *op, *operand, *semantics),
            KirInstructionKind::Compare { op, left, right } => {
                let left_value = self.load(*left)?;
                let right_value = self.load(*right)?;
                let name = self.name("compare");
                let value = self.builder.compare(
                    compare_op(*op, self.type_of(*left)?),
                    left_value,
                    right_value,
                    &name,
                )?;
                self.store(instruction.results[0].value, value)
            }
            KirInstructionKind::Cast { op, value } => {
                let value = self.load(*value)?;
                let op = match op {
                    MirCastOp::I32ToF64 => BridgeCastOp::Sitofp,
                    MirCastOp::U32ToF64 => BridgeCastOp::Uitofp,
                };
                let result = &instruction.results[0];
                let name = self.name("cast");
                let value =
                    self.builder
                        .cast(op, value, self.types.get_kir(&result.type_node)?, &name)?;
                self.store(result.value, value)
            }
            KirInstructionKind::CheckCondition { kind, args } => {
                if !self
                    .guard_conditions
                    .contains(&instruction.results[0].value)
                {
                    return Ok(());
                }
                let value = self.check_condition(*kind, args)?;
                self.store(instruction.results[0].value, value)
            }
            KirInstructionKind::Guard { condition, failure } => {
                let failed = self.load(*condition)?;
                let code = match failure {
                    KirFailureKind::Overflow => 1,
                    KirFailureKind::DivisionByZero => 2,
                    KirFailureKind::OutOfBounds | KirFailureKind::ContractViolation => 4,
                };
                let status = self.status(code)?;
                self.guard_status(failed, status)
            }
            KirInstructionKind::Address { place } => {
                let pointer = self.place_pointer(place)?;
                self.store(instruction.results[0].value, pointer)
            }
            KirInstructionKind::Load { place } => {
                let pointer = self.place_pointer(place)?;
                let result = &instruction.results[0];
                let name = self.name("place.load");
                let (alias_scopes, noalias_scopes) = self.alias_metadata(place)?;
                let value = if alias_scopes.is_empty() && noalias_scopes.is_empty() {
                    self.builder
                        .load(self.types.get_kir(&result.type_node)?, pointer, &name)?
                } else {
                    self.builder.load_scoped_alias(
                        self.types.get_kir(&result.type_node)?,
                        pointer,
                        &alias_scopes,
                        &noalias_scopes,
                        &name,
                    )?
                };
                self.store(result.value, value)
            }
            KirInstructionKind::Store { place, value } => {
                let pointer = self.place_pointer(place)?;
                let value = self.load(*value)?;
                let (alias_scopes, noalias_scopes) = self.alias_metadata(place)?;
                if alias_scopes.is_empty() && noalias_scopes.is_empty() {
                    self.builder.store(value, pointer)
                } else {
                    self.builder
                        .store_scoped_alias(value, pointer, &alias_scopes, &noalias_scopes)
                }
            }
            KirInstructionKind::MakeSlice { data, len } => {
                let data = self.load(*data)?;
                let len = self.load(*len)?;
                let value = self.make_slice(data, len)?;
                self.store(instruction.results[0].value, value)
            }
            KirInstructionKind::SliceData { slice } => {
                let slice = self.load(*slice)?;
                let name = self.name("slice.data");
                let value = self.builder.extract_value(slice, 0, &name)?;
                self.store(instruction.results[0].value, value)
            }
            KirInstructionKind::SliceLen { slice } => {
                let slice = self.load(*slice)?;
                let name = self.name("slice.len");
                let value = self.builder.extract_value(slice, 1, &name)?;
                self.store(instruction.results[0].value, value)
            }
            KirInstructionKind::Subslice { slice, start, end } => {
                self.subslice(instruction, *slice, *start, *end)
            }
            KirInstructionKind::VersionPredicate { predicate } => {
                let value = self.version_predicate(predicate)?;
                self.store(instruction.results[0].value, value)
            }
            KirInstructionKind::VectorSplat { scalar, .. } => {
                let scalar = self.load(*scalar)?;
                let lanes = vector_lanes(&instruction.results[0].type_node)?;
                let name = self.name("vector.splat");
                let value = self.builder.vector_splat(u32::from(lanes), scalar, &name)?;
                self.store(instruction.results[0].value, value)
            }
            KirInstructionKind::VectorLoad { access, .. } => {
                let pointer = self.vector_access_pointer(access)?;
                let result = &instruction.results[0];
                let name = self.name("vector.load");
                let value = self.builder.vector_load(
                    self.types.get_kir(&result.type_node)?,
                    pointer,
                    u32::from(access.required_alignment),
                    &name,
                )?;
                self.store(result.value, value)
            }
            KirInstructionKind::VectorStore { access, value, .. } => {
                let pointer = self.vector_access_pointer(access)?;
                let value = self.load(*value)?;
                self.builder
                    .vector_store(value, pointer, u32::from(access.required_alignment))
            }
            KirInstructionKind::VectorBinary {
                op, left, right, ..
            } => {
                let (lane, _) = fixed_vector_type(&instruction.results[0].type_node)?;
                let left = self.load(*left)?;
                let right = self.load(*right)?;
                let name = self.name("vector.binary");
                let value = self
                    .builder
                    .binary(vector_binary_op(*op, lane), left, right, &name)?;
                self.store(instruction.results[0].value, value)
            }
            KirInstructionKind::VectorUnary { op, operand, .. } => {
                let operand = self.load(*operand)?;
                let bridge_op = match op {
                    KirVectorUnaryOp::MaskNot => super::ffi::BridgeUnaryOp::Not,
                    KirVectorUnaryOp::Negate => {
                        let (lane, _) = fixed_vector_type(&instruction.results[0].type_node)?;
                        if lane == KirLaneType::F64 {
                            super::ffi::BridgeUnaryOp::FNeg
                        } else {
                            super::ffi::BridgeUnaryOp::Neg
                        }
                    }
                };
                let name = self.name("vector.unary");
                let value = self.builder.unary(bridge_op, operand, &name)?;
                self.store(instruction.results[0].value, value)
            }
            KirInstructionKind::VectorCompare {
                op, left, right, ..
            } => {
                let (lane, _) = self.fixed_vector_value_type(*left)?;
                let left = self.load(*left)?;
                let right = self.load(*right)?;
                let name = self.name("vector.compare");
                let value = self.builder.compare(
                    compare_op(*op, &lane_mir_type(lane)),
                    left,
                    right,
                    &name,
                )?;
                self.store(instruction.results[0].value, value)
            }
            KirInstructionKind::VectorSelect {
                mask,
                when_true,
                when_false,
                ..
            } => {
                let mask = self.load(*mask)?;
                let when_true = self.load(*when_true)?;
                let when_false = self.load(*when_false)?;
                let name = self.name("vector.select");
                let value = self.builder.select(mask, when_true, when_false, &name)?;
                self.store(instruction.results[0].value, value)
            }
            KirInstructionKind::VectorCast { op, value, .. } => {
                let value = self.load(*value)?;
                let bridge_op = match op {
                    KirVectorCastOp::I32ToF64 => BridgeCastOp::Sitofp,
                    KirVectorCastOp::U32ToF64 => BridgeCastOp::Uitofp,
                };
                let result = &instruction.results[0];
                let name = self.name("vector.cast");
                let value = self.builder.cast(
                    bridge_op,
                    value,
                    self.types.get_kir(&result.type_node)?,
                    &name,
                )?;
                self.store(result.value, value)
            }
            KirInstructionKind::VectorInsert {
                vector,
                scalar,
                lane_index,
                ..
            } => {
                let vector = self.load(*vector)?;
                let scalar = self.load(*scalar)?;
                let name = self.name("vector.insert");
                let value =
                    self.builder
                        .vector_insert(vector, scalar, u32::from(*lane_index), &name)?;
                self.store(instruction.results[0].value, value)
            }
            KirInstructionKind::VectorExtract {
                vector, lane_index, ..
            } => {
                let vector = self.load(*vector)?;
                let name = self.name("vector.extract");
                let value = self
                    .builder
                    .vector_extract(vector, u32::from(*lane_index), &name)?;
                self.store(instruction.results[0].value, value)
            }
            KirInstructionKind::VectorReduce { op, vector, .. } => {
                let (lane, _) = self.fixed_vector_value_type(*vector)?;
                let reduction = match (op, lane) {
                    (KirVectorReductionOp::ModularAdd, _) => 1,
                    (KirVectorReductionOp::ModularMultiply, _) => 6,
                    (KirVectorReductionOp::ModularMin, KirLaneType::I32 | KirLaneType::I64) => 2,
                    (KirVectorReductionOp::ModularMin, KirLaneType::U32 | KirLaneType::U64) => 3,
                    (KirVectorReductionOp::ModularMax, KirLaneType::I32 | KirLaneType::I64) => 4,
                    (KirVectorReductionOp::ModularMax, KirLaneType::U32 | KirLaneType::U64) => 5,
                    (_, KirLaneType::F64) => {
                        return Err(lowering_error("f64 vector reduction is unsupported"));
                    }
                };
                let vector = self.load(*vector)?;
                let name = self.name("vector.reduce");
                let value = self.builder.vector_reduce(reduction, vector, &name)?;
                self.store(instruction.results[0].value, value)
            }
            KirInstructionKind::Call {
                function_name,
                args,
            } => self.call(instruction, function_name, args),
            KirInstructionKind::RuntimeCall { intrinsic, args } => {
                let (name, _) = runtime_signature(*intrinsic);
                let function = require_function(self.functions, name)?;
                let args = self.physical_args(args)?;
                self.builder.call(function, &args, "").map(|_| ())
            }
        }
    }

    fn binary(
        &mut self,
        instruction: &KirInstruction,
        op: MirBinaryOp,
        left: ValueId,
        right: ValueId,
        semantics: KirArithmeticSemantics,
    ) -> Result<(), NativeError> {
        let left_value = self.load(left)?;
        let right_value = self.load(right)?;
        let type_node = instruction.results[0]
            .type_node
            .as_scalar()
            .ok_or_else(|| lowering_error("scalar binary produced a vector value"))?;
        if semantics == KirArithmeticSemantics::Checked
            && instruction.results.len() == 2
            && self
                .guard_conditions
                .contains(&instruction.results[1].value)
        {
            let unsigned = matches!(
                type_node,
                MirType::Primitive(MirPrimitiveTypeName::U32 | MirPrimitiveTypeName::U64)
            );
            let overflow_op = match (op, unsigned) {
                (MirBinaryOp::Add, false) => BridgeOverflowOp::SignedAdd,
                (MirBinaryOp::Add, true) => BridgeOverflowOp::UnsignedAdd,
                (MirBinaryOp::Sub, false) => BridgeOverflowOp::SignedSub,
                (MirBinaryOp::Sub, true) => BridgeOverflowOp::UnsignedSub,
                (MirBinaryOp::Mul, false) => BridgeOverflowOp::SignedMul,
                (MirBinaryOp::Mul, true) => BridgeOverflowOp::UnsignedMul,
                _ => return Err(lowering_error("invalid checked KIR binary pair")),
            };
            let name = self.name("overflow.pair");
            let pair = self
                .builder
                .overflow(overflow_op, left_value, right_value, &name)?;
            let name = self.name("overflow.value");
            let value = self.builder.extract_value(pair, 0, &name)?;
            let name = self.name("overflow.flag");
            let overflow = self.builder.extract_value(pair, 1, &name)?;
            self.store(instruction.results[0].value, value)?;
            return self.store(instruction.results[1].value, overflow);
        }
        let name = self.name("binary");
        let wrap = self
            .facts
            .wrap_proofs
            .get(&(self.function.id, instruction.id));
        let value = self.builder.binary_with_flags(
            binary_op(op, type_node)?,
            left_value,
            right_value,
            matches!(wrap, Some((_, NativeStrengtheningKind::NoUnsignedWrap))),
            matches!(wrap, Some((_, NativeStrengtheningKind::NoSignedWrap))),
            &name,
        )?;
        self.store(instruction.results[0].value, value)
    }

    fn version_predicate(
        &mut self,
        predicate: &KirVersionPredicate,
    ) -> Result<NativeValue<'module>, NativeError> {
        let mut combined = None;
        for conjunct in &predicate.conjuncts {
            let condition = match conjunct {
                KirVersionPredicateConjunct::TripThreshold { value, minimum } => {
                    let value = self.load(*value)?;
                    let minimum = self
                        .builder
                        .const_int(self.types.i32, &minimum.to_string())?;
                    let name = self.name("version.trip");
                    self.builder
                        .compare(BridgeCompareOp::IcmpUge, value, minimum, &name)?
                }
                KirVersionPredicateConjunct::AddressIntervalsDisjoint {
                    left,
                    left_count,
                    left_element_bytes,
                    right,
                    right_count,
                    right_element_bytes,
                } => self.version_disjoint_intervals(
                    predicate.address_bits,
                    *left,
                    *left_count,
                    *left_element_bytes,
                    *right,
                    *right_count,
                    *right_element_bytes,
                )?,
            };
            combined = Some(if let Some(previous) = combined {
                self.bool_and(previous, condition, "version.and")?
            } else {
                condition
            });
        }
        combined.ok_or_else(|| lowering_error("version predicate has no conjuncts"))
    }

    #[allow(clippy::too_many_arguments)]
    fn version_disjoint_intervals(
        &mut self,
        address_bits: u8,
        left: ValueId,
        left_count: ValueId,
        left_element_bytes: u32,
        right: ValueId,
        right_count: ValueId,
        right_element_bytes: u32,
    ) -> Result<NativeValue<'module>, NativeError> {
        let left_slice = self.load(left)?;
        let right_slice = self.load(right)?;
        let name = self.name("version.left.data");
        let left_pointer = self.builder.extract_value(left_slice, 0, &name)?;
        let name = self.name("version.right.data");
        let right_pointer = self.builder.extract_value(right_slice, 0, &name)?;
        let name = self.name("version.left.address");
        let left_address =
            self.builder
                .cast(BridgeCastOp::PtrToInt, left_pointer, self.types.i64, &name)?;
        let name = self.name("version.right.address");
        let right_address =
            self.builder
                .cast(BridgeCastOp::PtrToInt, right_pointer, self.types.i64, &name)?;
        let left_count32 = self.load(left_count)?;
        let right_count32 = self.load(right_count)?;
        let name = self.name("version.left.count");
        let left_count64 =
            self.builder
                .cast(BridgeCastOp::Zext, left_count32, self.types.i64, &name)?;
        let name = self.name("version.right.count");
        let right_count64 =
            self.builder
                .cast(BridgeCastOp::Zext, right_count32, self.types.i64, &name)?;
        let left_width = self
            .builder
            .const_int(self.types.i64, &left_element_bytes.to_string())?;
        let right_width = self
            .builder
            .const_int(self.types.i64, &right_element_bytes.to_string())?;
        let name = self.name("version.left.bytes");
        let left_bytes =
            self.builder
                .binary(BridgeBinaryOp::Mul, left_count64, left_width, &name)?;
        let name = self.name("version.right.bytes");
        let right_bytes =
            self.builder
                .binary(BridgeBinaryOp::Mul, right_count64, right_width, &name)?;
        let name = self.name("version.left.end");
        let left_end = self
            .builder
            .binary(BridgeBinaryOp::Add, left_address, left_bytes, &name)?;
        let name = self.name("version.right.end");
        let right_end =
            self.builder
                .binary(BridgeBinaryOp::Add, right_address, right_bytes, &name)?;
        let name = self.name("version.left.no-wrap");
        let left_no_wrap =
            self.builder
                .compare(BridgeCompareOp::IcmpUge, left_end, left_address, &name)?;
        let name = self.name("version.right.no-wrap");
        let right_no_wrap =
            self.builder
                .compare(BridgeCompareOp::IcmpUge, right_end, right_address, &name)?;
        let maximum = if address_bits == 32 {
            u32::MAX.to_string()
        } else {
            u64::MAX.to_string()
        };
        let maximum = self.builder.const_int(self.types.i64, &maximum)?;
        let name = self.name("version.left.in-range");
        let left_in_range =
            self.builder
                .compare(BridgeCompareOp::IcmpUle, left_end, maximum, &name)?;
        let name = self.name("version.right.in-range");
        let right_in_range =
            self.builder
                .compare(BridgeCompareOp::IcmpUle, right_end, maximum, &name)?;
        let left_valid = self.bool_and(left_no_wrap, left_in_range, "version.left.valid")?;
        let right_valid = self.bool_and(right_no_wrap, right_in_range, "version.right.valid")?;
        let valid = self.bool_and(left_valid, right_valid, "version.address.valid")?;
        let name = self.name("version.left.before-right");
        let left_before_right =
            self.builder
                .compare(BridgeCompareOp::IcmpUle, left_end, right_address, &name)?;
        let name = self.name("version.right.before-left");
        let right_before_left =
            self.builder
                .compare(BridgeCompareOp::IcmpUle, right_end, left_address, &name)?;
        let disjoint = self.bool_or(left_before_right, right_before_left, "version.disjoint")?;
        let zero = self.builder.const_int(self.types.i32, "0")?;
        let name = self.name("version.left.empty");
        let left_empty =
            self.builder
                .compare(BridgeCompareOp::IcmpEq, left_count32, zero, &name)?;
        let name = self.name("version.right.empty");
        let right_empty =
            self.builder
                .compare(BridgeCompareOp::IcmpEq, right_count32, zero, &name)?;
        let empty = self.bool_or(left_empty, right_empty, "version.empty")?;
        let separated_or_empty = self.bool_or(empty, disjoint, "version.safe-interval")?;
        self.bool_and(valid, separated_or_empty, "version.total")
    }

    fn bool_and(
        &mut self,
        left: NativeValue<'module>,
        right: NativeValue<'module>,
        prefix: &str,
    ) -> Result<NativeValue<'module>, NativeError> {
        let false_value = self.builder.const_bool(false)?;
        let name = self.name(prefix);
        self.builder.select(left, right, false_value, &name)
    }

    fn bool_or(
        &mut self,
        left: NativeValue<'module>,
        right: NativeValue<'module>,
        prefix: &str,
    ) -> Result<NativeValue<'module>, NativeError> {
        let true_value = self.builder.const_bool(true)?;
        let name = self.name(prefix);
        self.builder.select(left, true_value, right, &name)
    }

    fn vector_access_pointer(
        &mut self,
        access: &KirVectorMemoryAccess,
    ) -> Result<NativeValue<'module>, NativeError> {
        let slice = self.load(access.slice)?;
        let name = self.name("vector.slice.data");
        let data = self.builder.extract_value(slice, 0, &name)?;
        let start = self.load(access.start)?;
        let start_type = self.type_of(access.start)?.clone();
        let index = self.index_to_i64(start, &start_type)?;
        let name = self.name("vector.slice.index");
        self.builder
            .gep(self.types.lane(access.lane), data, &[index], &name)
    }

    fn fixed_vector_value_type(&self, value: ValueId) -> Result<(KirLaneType, u16), NativeError> {
        let type_node = self.value_type_of(value)?;
        fixed_vector_type(&type_node)
    }

    fn unary(
        &mut self,
        instruction: &KirInstruction,
        op: MirUnaryOp,
        operand: ValueId,
        semantics: KirArithmeticSemantics,
    ) -> Result<(), NativeError> {
        let operand = self.load(operand)?;
        let type_node = instruction.results[0]
            .type_node
            .as_scalar()
            .ok_or_else(|| lowering_error("scalar unary produced a vector value"))?;
        if semantics == KirArithmeticSemantics::Checked
            && instruction.results.len() == 2
            && self
                .guard_conditions
                .contains(&instruction.results[1].value)
        {
            let unsigned = matches!(
                type_node,
                MirType::Primitive(MirPrimitiveTypeName::U32 | MirPrimitiveTypeName::U64)
            );
            let op = if unsigned {
                BridgeOverflowOp::UnsignedSub
            } else {
                BridgeOverflowOp::SignedSub
            };
            let zero = self.builder.const_int(self.types.get(type_node)?, "0")?;
            let name = self.name("negate.pair");
            let pair = self.builder.overflow(op, zero, operand, &name)?;
            let name = self.name("negate.value");
            let value = self.builder.extract_value(pair, 0, &name)?;
            let name = self.name("negate.overflow");
            let overflow = self.builder.extract_value(pair, 1, &name)?;
            self.store(instruction.results[0].value, value)?;
            return self.store(instruction.results[1].value, overflow);
        }
        let name = self.name("unary");
        let value = self
            .builder
            .unary(unary_op(op, type_node), operand, &name)?;
        self.store(instruction.results[0].value, value)
    }

    fn check_condition(
        &mut self,
        kind: KirCheckConditionKind,
        args: &[ValueId],
    ) -> Result<NativeValue<'module>, NativeError> {
        match kind {
            KirCheckConditionKind::ArithmeticOverflow => self.builder.const_bool(false),
            KirCheckConditionKind::DivisionByZero => {
                let value = self.load(args[0])?;
                let zero = self
                    .builder
                    .const_int(self.types.get(self.type_of(args[0])?)?, "0")?;
                let name = self.name("division.by_zero");
                self.builder
                    .compare(BridgeCompareOp::IcmpEq, value, zero, &name)
            }
            KirCheckConditionKind::SignedDivisionOverflow => {
                let left = self.load(args[0])?;
                let right = self.load(args[1])?;
                let type_node = self.type_of(args[0])?;
                let minimum = match type_node {
                    MirType::Primitive(MirPrimitiveTypeName::I32) => "-2147483648",
                    MirType::Primitive(MirPrimitiveTypeName::I64) => "-9223372036854775808",
                    _ => return Err(lowering_error("signed division check type is invalid")),
                };
                let llvm_type = self.types.get(type_node)?;
                let minimum = self.builder.const_int(llvm_type, minimum)?;
                let negative_one = self.builder.const_int(llvm_type, "-1")?;
                let name = self.name("division.minimum");
                let is_min = self
                    .builder
                    .compare(BridgeCompareOp::IcmpEq, left, minimum, &name)?;
                let name = self.name("division.negative_one");
                let is_negative_one =
                    self.builder
                        .compare(BridgeCompareOp::IcmpEq, right, negative_one, &name)?;
                let false_value = self.builder.const_bool(false)?;
                let name = self.name("division.overflows");
                self.builder
                    .select(is_min, is_negative_one, false_value, &name)
            }
            KirCheckConditionKind::SliceOutOfBounds => {
                let slice = self.load(args[0])?;
                let index = self.load(args[1])?;
                let name = self.name("slice.len");
                let len = self.builder.extract_value(slice, 1, &name)?;
                let name = self.name("slice.out_of_bounds");
                self.builder
                    .compare(BridgeCompareOp::IcmpUge, index, len, &name)
            }
            KirCheckConditionKind::InvalidSubslice => {
                let slice = self.load(args[0])?;
                let start = self.load(args[1])?;
                let end = self.load(args[2])?;
                let name = self.name("subslice.len");
                let len = self.builder.extract_value(slice, 1, &name)?;
                let name = self.name("subslice.start_after_end");
                let invalid_order =
                    self.builder
                        .compare(BridgeCompareOp::IcmpUgt, start, end, &name)?;
                let name = self.name("subslice.end_after_len");
                let invalid_end =
                    self.builder
                        .compare(BridgeCompareOp::IcmpUgt, end, len, &name)?;
                let true_value = self.builder.const_bool(true)?;
                let name = self.name("subslice.invalid");
                self.builder
                    .select(invalid_order, true_value, invalid_end, &name)
            }
        }
    }

    fn subslice(
        &mut self,
        instruction: &KirInstruction,
        slice: ValueId,
        start: ValueId,
        end: ValueId,
    ) -> Result<(), NativeError> {
        let MirType::Slice(element) = self.type_of(slice)? else {
            return Err(lowering_error("KIR subslice source is not a slice"));
        };
        let element = element.clone();
        let descriptor = self.load(slice)?;
        let name = self.name("subslice.data");
        let data = self.builder.extract_value(descriptor, 0, &name)?;
        let start_value = self.load(start)?;
        let end_value = self.load(end)?;
        let start_type = self.type_of(start)?.clone();
        let start64 = self.index_to_i64(start_value, &start_type)?;
        let name = self.name("subslice.gep");
        let advanced = self
            .builder
            .gep(self.types.get(&element)?, data, &[start64], &name)?;
        let zero = self.builder.const_int(self.types.i32, "0")?;
        let name = self.name("subslice.zero");
        let is_zero = self
            .builder
            .compare(BridgeCompareOp::IcmpEq, start_value, zero, &name)?;
        let name = self.name("subslice.selected");
        let selected = self.builder.select(is_zero, data, advanced, &name)?;
        let name = self.name("subslice.length");
        let len = self.builder.binary(
            super::ffi::BridgeBinaryOp::Sub,
            end_value,
            start_value,
            &name,
        )?;
        let value = self.make_slice(selected, len)?;
        self.store(instruction.results[0].value, value)
    }

    fn call(
        &mut self,
        instruction: &KirInstruction,
        name: &str,
        args: &[ValueId],
    ) -> Result<(), NativeError> {
        let function = require_function(self.functions, name)?;
        let mut args = self.physical_args(args)?;
        if self.status_abi {
            if let Some(result) = instruction.results.first() {
                args.push(self.storage(result.value)?.pointer);
            }
            let call_name = self.name("call");
            let status = self.builder.call(function, &args, &call_name)?;
            let zero = self.status(0)?;
            let compare_name = self.name("call.failed");
            let failed =
                self.builder
                    .compare(BridgeCompareOp::IcmpNe, status, zero, &compare_name)?;
            self.guard_status(failed, status)
        } else if let Some(result) = instruction.results.first() {
            let call_name = self.name("call");
            let value = self.builder.call(function, &args, &call_name)?;
            self.store(result.value, value)
        } else {
            self.builder.call(function, &args, "").map(|_| ())
        }
    }

    fn terminator(
        &mut self,
        source: BlockId,
        terminator: &KirTerminator,
    ) -> Result<(), NativeError> {
        match terminator {
            KirTerminator::Return { value, .. } => {
                if self.status_abi {
                    if let Some(value) = value {
                        let value = self.load(*value)?;
                        let pointer = self.result_pointer.ok_or_else(|| {
                            lowering_error("checked KIR return is missing result pointer")
                        })?;
                        self.builder.store(value, pointer)?;
                    }
                    let ok = self.status(0)?;
                    self.builder.return_value(ok)
                } else if let Some(value) = value {
                    let value = self.load(*value)?;
                    self.builder.return_value(value)
                } else {
                    self.builder.return_void()
                }
            }
            KirTerminator::Jump { edge } => {
                let native_source = self.current_block()?;
                let target = self.edge_block(source, edge)?;
                self.builder.position(native_source)?;
                self.builder.branch(target)
            }
            KirTerminator::Branch {
                condition,
                then_edge,
                else_edge,
            } => {
                let condition = self.load(*condition)?;
                let native_source = self.current_block()?;
                let then_block = self.edge_block(source, then_edge)?;
                self.builder.position(native_source)?;
                let else_block = self.edge_block(source, else_edge)?;
                self.builder.position(native_source)?;
                if let Some((then_count, else_count)) =
                    self.pgo_branches.get(&(self.function.id, source)).copied()
                {
                    self.builder.cond_branch_weighted(
                        condition, then_block, else_block, then_count, else_count,
                    )
                } else {
                    self.builder.cond_branch(condition, then_block, else_block)
                }
            }
        }
    }

    fn edge_block(
        &mut self,
        source: BlockId,
        edge: &KirEdge,
    ) -> Result<NativeBlock<'module>, NativeError> {
        let values = edge
            .args
            .iter()
            .map(|value| self.load(*value))
            .collect::<Result<Vec<_>, _>>()?;
        let name = self.name("kir.edge");
        let edge_block = self.handle.append_block(&name)?;
        self.builder.position(edge_block)?;
        let target = self
            .function
            .blocks
            .iter()
            .find(|block| block.id == edge.target)
            .ok_or_else(|| lowering_error("KIR edge target is missing"))?;
        for (param, value) in target.params.iter().zip(values) {
            self.store(param.value, value)?;
        }
        self.emit_profile_edge(source, edge.target)?;
        self.builder.branch(self.block(edge.target)?)?;
        Ok(edge_block)
    }

    fn emit_profile_edge(&mut self, source: BlockId, target: BlockId) -> Result<(), NativeError> {
        let Some(profile) = self.profile else {
            return Ok(());
        };
        let loop_events = profile
            .loops
            .iter()
            .filter(|event| event.function == self.function.id)
            .cloned()
            .collect::<Vec<_>>();
        let observe_trip = profile.observe_trip;
        let increment = profile.increment;
        let add = profile.add;
        let edge_site = profile
            .edges
            .get(&(self.function.id, source, target))
            .copied();
        let mut aggregate_edge = false;
        for event in loop_events {
            let storage =
                self.loop_storage.get(&event.site).copied().ok_or_else(|| {
                    lowering_error("profile loop event has no local trip counter")
                })?;
            if event.latches.contains(&source) && target == event.header {
                let value =
                    self.builder
                        .load(self.types.i64, storage.pointer, "ck.profile.loop.trip")?;
                let one = self.builder.const_int(self.types.i64, "1")?;
                let value =
                    self.builder
                        .binary(BridgeBinaryOp::Add, value, one, "ck.profile.loop.next")?;
                self.builder.store(value, storage.pointer)?;
                aggregate_edge |= event.latches.len() == 1 && edge_site.is_some();
            }
            if event.exits.contains(&(source, target)) {
                let value = self.builder.load(
                    self.types.i64,
                    storage.pointer,
                    "ck.profile.loop.completed",
                )?;
                let site = self
                    .builder
                    .const_int(self.types.i32, &event.site.to_string())?;
                let _ = self.builder.call(observe_trip, &[site, value], "")?;
                if event.latches.len() == 1 {
                    let latch = event.latches[0];
                    if let Some(edge_site) = profile
                        .edges
                        .get(&(self.function.id, latch, event.header))
                        .copied()
                    {
                        let edge_site = self
                            .builder
                            .const_int(self.types.i32, &edge_site.to_string())?;
                        let _ = self.builder.call(add, &[edge_site, value], "")?;
                    }
                }
                let zero = self.builder.const_int(self.types.i64, "0")?;
                self.builder.store(zero, storage.pointer)?;
            }
        }
        if let Some(site) = edge_site.filter(|_| !aggregate_edge) {
            let site = self.builder.const_int(self.types.i32, &site.to_string())?;
            let _ = self.builder.call(increment, &[site], "")?;
        }
        Ok(())
    }

    fn current_block(&self) -> Result<NativeBlock<'module>, NativeError> {
        self.current_block
            .ok_or_else(|| lowering_error("KIR lowering has no active LLVM block"))
    }

    fn physical_args(
        &mut self,
        args: &[ValueId],
    ) -> Result<Vec<NativeValue<'module>>, NativeError> {
        let mut physical = Vec::new();
        for value in args {
            let loaded = self.load(*value)?;
            if matches!(self.type_of(*value)?, MirType::Slice(_)) {
                let name = self.name("arg.data");
                physical.push(self.builder.extract_value(loaded, 0, &name)?);
                let name = self.name("arg.len");
                physical.push(self.builder.extract_value(loaded, 1, &name)?);
            } else {
                physical.push(loaded);
            }
        }
        Ok(physical)
    }

    fn place_pointer(&mut self, place: &KirPlace) -> Result<NativeValue<'module>, NativeError> {
        match place {
            KirPlace::Value {
                value, type_node, ..
            } => {
                if matches!(type_node, MirType::Pointer(_)) {
                    self.load(*value)
                } else {
                    Ok(self.storage(*value)?.pointer)
                }
            }
            KirPlace::Deref { pointer, .. } => self.load(*pointer),
            KirPlace::Index { base, index, .. } => {
                let MirType::Pointer(element) = kir_place_type(base) else {
                    return Err(lowering_error("KIR index base is not a pointer"));
                };
                let element = element.clone();
                let base = self.place_pointer(base)?;
                let index_value = self.load(*index)?;
                let index_type = self.type_of(*index)?.clone();
                let index64 = self.index_to_i64(index_value, &index_type)?;
                let name = self.name("index");
                self.builder
                    .gep(self.types.get(&element)?, base, &[index64], &name)
            }
            KirPlace::SliceIndex { slice, index, .. } => {
                let MirType::Slice(element) = self.type_of(*slice)? else {
                    return Err(lowering_error("KIR slice index base is not a slice"));
                };
                let element = element.clone();
                let slice = self.load(*slice)?;
                let name = self.name("slice.data");
                let data = self.builder.extract_value(slice, 0, &name)?;
                let index_value = self.load(*index)?;
                let index_type = self.type_of(*index)?.clone();
                let index64 = self.index_to_i64(index_value, &index_type)?;
                let name = self.name("slice.index");
                self.builder
                    .gep(self.types.get(&element)?, data, &[index64], &name)
            }
            KirPlace::Field {
                base, field_name, ..
            } => {
                let MirType::Struct(struct_name) = kir_place_type(base) else {
                    return Err(lowering_error("KIR field base is not a struct"));
                };
                let struct_name = struct_name.clone();
                let base = self.place_pointer(base)?;
                let zero = self.builder.const_int(self.types.i32, "0")?;
                let field = self.builder.const_int(
                    self.types.i32,
                    &self
                        .layout
                        .field_index(&struct_name, field_name)
                        .to_string(),
                )?;
                let name = self.name("field");
                self.builder.gep(
                    self.types.get(&MirType::Struct(struct_name))?,
                    base,
                    &[zero, field],
                    &name,
                )
            }
        }
    }

    fn alias_metadata(&self, place: &KirPlace) -> Result<(Vec<u32>, Vec<u32>), NativeError> {
        let Some(root) = root_parameter_for_region(self.function, kir_place_region(place)) else {
            return Ok((Vec::new(), Vec::new()));
        };
        let facts = self
            .facts
            .scoped_alias_facts
            .get(&self.function.id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let mut noalias = facts
            .iter()
            .filter_map(|(_, left, right)| {
                if *left == root {
                    Some(*right)
                } else if *right == root {
                    Some(*left)
                } else {
                    None
                }
            })
            .map(alias_scope_id)
            .collect::<Result<Vec<_>, _>>()?;
        noalias.sort_unstable();
        noalias.dedup();
        if noalias.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        Ok((vec![alias_scope_id(root)?], noalias))
    }

    fn index_to_i64(
        &mut self,
        value: NativeValue<'module>,
        type_node: &MirType,
    ) -> Result<NativeValue<'module>, NativeError> {
        match type_node {
            MirType::Primitive(MirPrimitiveTypeName::I32) => {
                let name = self.name("index64");
                self.builder
                    .cast(BridgeCastOp::Sext, value, self.types.i64, &name)
            }
            MirType::Primitive(MirPrimitiveTypeName::U32) => {
                let name = self.name("index64");
                self.builder
                    .cast(BridgeCastOp::Zext, value, self.types.i64, &name)
            }
            MirType::Primitive(MirPrimitiveTypeName::I64 | MirPrimitiveTypeName::U64) => Ok(value),
            _ => Err(lowering_error("KIR index is not an integer")),
        }
    }

    fn make_slice(
        &mut self,
        data: NativeValue<'module>,
        len: NativeValue<'module>,
    ) -> Result<NativeValue<'module>, NativeError> {
        let undef = self.builder.undef(self.types.slice)?;
        let name = self.name("slice.data");
        let with_data = self.builder.insert_value(undef, data, 0, &name)?;
        let name = self.name("slice.value");
        self.builder.insert_value(with_data, len, 1, &name)
    }

    fn guard_status(
        &mut self,
        failed: NativeValue<'module>,
        status: NativeValue<'module>,
    ) -> Result<(), NativeError> {
        let continue_name = self.name("checked.continue");
        let continuation = self.handle.append_block(&continue_name)?;
        let failure_name = self.name("checked.failure");
        let failure = self.handle.append_block(&failure_name)?;
        self.builder.cond_branch(failed, failure, continuation)?;
        self.builder.position(failure)?;
        self.builder.return_value(status)?;
        self.builder.position(continuation)?;
        self.current_block = Some(continuation);
        Ok(())
    }

    fn status(&self, code: i32) -> Result<NativeValue<'module>, NativeError> {
        self.builder.const_int(self.types.i32, &code.to_string())
    }

    fn load(&mut self, value: ValueId) -> Result<NativeValue<'module>, NativeError> {
        let storage = self.storage(value)?;
        let type_node = self.value_type_of(value)?.clone();
        let name = self.name("load");
        self.builder
            .load(self.types.get_kir(&type_node)?, storage.pointer, &name)
    }

    fn store(&mut self, value: ValueId, native: NativeValue<'module>) -> Result<(), NativeError> {
        self.builder.store(native, self.storage(value)?.pointer)
    }

    fn storage(&self, value: ValueId) -> Result<Storage<'module>, NativeError> {
        self.storage
            .get(&value)
            .copied()
            .ok_or_else(|| lowering_error(format!("missing KIR storage for v{}", value.index())))
    }

    fn value_type_of(&self, value: ValueId) -> Result<KirValueType, NativeError> {
        self.function
            .params
            .iter()
            .find(|param| param.value == value)
            .map(|param| KirValueType::Scalar(param.type_node.clone()))
            .or_else(|| {
                self.function
                    .blocks
                    .iter()
                    .flat_map(|block| &block.params)
                    .find(|param| param.value == value)
                    .map(|param| param.type_node.clone())
            })
            .or_else(|| {
                self.function
                    .blocks
                    .iter()
                    .flat_map(|block| &block.instructions)
                    .flat_map(|instruction| &instruction.results)
                    .find(|result| result.value == value)
                    .map(|result| result.type_node.clone())
            })
            .ok_or_else(|| lowering_error(format!("unknown KIR value v{}", value.index())))
    }

    fn type_of(&self, value: ValueId) -> Result<&MirType, NativeError> {
        self.function
            .params
            .iter()
            .find(|param| param.value == value)
            .map(|param| &param.type_node)
            .or_else(|| {
                self.function
                    .blocks
                    .iter()
                    .flat_map(|block| &block.params)
                    .find(|param| param.value == value)
                    .and_then(|param| param.type_node.as_scalar())
            })
            .or_else(|| {
                self.function
                    .blocks
                    .iter()
                    .flat_map(|block| &block.instructions)
                    .flat_map(|instruction| &instruction.results)
                    .find(|result| result.value == value)
                    .and_then(|result| result.type_node.as_scalar())
            })
            .ok_or_else(|| {
                lowering_error(format!(
                    "KIR value v{} is unknown or is not scalar",
                    value.index()
                ))
            })
    }
}

fn contract_integer_width(
    expressions: &[&ContractFactAffineExpression],
    modulus: Option<&num_bigint::BigInt>,
) -> u32 {
    let mut decimal_digits = modulus
        .map(ToString::to_string)
        .map_or(1, |value| value.trim_start_matches('-').len());
    let mut term_count = 0usize;
    for expression in expressions {
        decimal_digits = decimal_digits.max(
            expression
                .constant
                .to_string()
                .trim_start_matches('-')
                .len(),
        );
        term_count += expression.terms.len();
        for term in &expression.terms {
            decimal_digits =
                decimal_digits.max(term.coefficient.to_string().trim_start_matches('-').len());
        }
    }
    let mut sum_bits = 0u32;
    let mut terms = term_count.max(1) - 1;
    while terms != 0 {
        sum_bits += 1;
        terms >>= 1;
    }
    let decimal_bits = u32::try_from(decimal_digits)
        .unwrap_or(u32::MAX / 4)
        .saturating_mul(4);
    let required = decimal_bits
        .saturating_add(64)
        .saturating_add(sum_bits)
        .saturating_add(4)
        .max(128);
    required.saturating_add(63) / 64 * 64
}

fn contract_compare_op(operator: &str) -> Result<BridgeCompareOp, NativeError> {
    match operator {
        "==" => Ok(BridgeCompareOp::IcmpEq),
        "!=" => Ok(BridgeCompareOp::IcmpNe),
        "<" => Ok(BridgeCompareOp::IcmpSlt),
        "<=" => Ok(BridgeCompareOp::IcmpSle),
        ">" => Ok(BridgeCompareOp::IcmpSgt),
        ">=" => Ok(BridgeCompareOp::IcmpSge),
        _ => Err(lowering_error(format!(
            "unknown contract comparison operator '{operator}'"
        ))),
    }
}

fn mir_type_layout(type_node: &MirType, structs: &[MirStruct]) -> Result<(u64, u64), NativeError> {
    mir_type_layout_inner(type_node, structs, &mut HashSet::new())
}

fn mir_type_layout_inner(
    type_node: &MirType,
    structs: &[MirStruct],
    active: &mut HashSet<String>,
) -> Result<(u64, u64), NativeError> {
    match type_node {
        MirType::Primitive(MirPrimitiveTypeName::Bool) => Ok((1, 1)),
        MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32) => Ok((4, 4)),
        MirType::Primitive(
            MirPrimitiveTypeName::I64 | MirPrimitiveTypeName::U64 | MirPrimitiveTypeName::F64,
        )
        | MirType::Pointer(_) => Ok((8, 8)),
        MirType::Slice(_) => Ok((16, 8)),
        MirType::Struct(name) => {
            if !active.insert(name.clone()) {
                return Err(lowering_error(format!(
                    "recursive struct '{name}' has no finite contract byte range"
                )));
            }
            let structure = structs
                .iter()
                .find(|structure| structure.name == *name)
                .ok_or_else(|| lowering_error(format!("unknown KIR struct '{name}'")))?;
            let mut size = 0u64;
            let mut alignment = 1u64;
            for field in &structure.fields {
                let (field_size, field_alignment) =
                    mir_type_layout_inner(&field.type_node, structs, active)?;
                size = align_contract_size(size, field_alignment)?;
                size = size.checked_add(field_size).ok_or_else(|| {
                    lowering_error(format!("struct '{name}' is too large for contract layout"))
                })?;
                alignment = alignment.max(field_alignment);
            }
            active.remove(name);
            Ok((align_contract_size(size, alignment)?, alignment))
        }
        MirType::Void => Err(lowering_error("void has no contract element layout")),
    }
}

fn align_contract_size(value: u64, alignment: u64) -> Result<u64, NativeError> {
    let remainder = value % alignment;
    if remainder == 0 {
        Ok(value)
    } else {
        value
            .checked_add(alignment - remainder)
            .ok_or_else(|| lowering_error("contract element layout overflows u64"))
    }
}

fn value_types(function: &KirFunction) -> BTreeMap<ValueId, MirType> {
    function
        .params
        .iter()
        .map(|param| (param.value, param.type_node.clone()))
        .chain(function.blocks.iter().flat_map(|block| {
            block
                .params
                .iter()
                .filter_map(|param| {
                    param
                        .type_node
                        .as_scalar()
                        .cloned()
                        .map(|type_node| (param.value, type_node))
                })
                .chain(block.instructions.iter().flat_map(|instruction| {
                    instruction.results.iter().filter_map(|result| {
                        result
                            .type_node
                            .as_scalar()
                            .cloned()
                            .map(|type_node| (result.value, type_node))
                    })
                }))
        }))
        .collect()
}

fn kir_value_types(function: &KirFunction) -> BTreeMap<ValueId, KirValueType> {
    function
        .params
        .iter()
        .map(|param| (param.value, KirValueType::Scalar(param.type_node.clone())))
        .chain(function.blocks.iter().flat_map(|block| {
            block
                .params
                .iter()
                .map(|param| (param.value, param.type_node.clone()))
                .chain(block.instructions.iter().flat_map(|instruction| {
                    instruction
                        .results
                        .iter()
                        .map(|result| (result.value, result.type_node.clone()))
                }))
        }))
        .collect()
}

fn vector_lanes(type_node: &KirValueType) -> Result<u16, NativeError> {
    match type_node {
        KirValueType::FixedVector { lanes, .. } | KirValueType::Mask { lanes } => Ok(*lanes),
        KirValueType::Scalar(_) => Err(lowering_error("KIR value is not a vector")),
    }
}

fn fixed_vector_type(type_node: &KirValueType) -> Result<(KirLaneType, u16), NativeError> {
    match type_node {
        KirValueType::FixedVector { lane, lanes } => Ok((*lane, *lanes)),
        KirValueType::Scalar(_) | KirValueType::Mask { .. } => {
            Err(lowering_error("KIR value is not a fixed lane vector"))
        }
    }
}

const fn lane_mir_type(lane: KirLaneType) -> MirType {
    MirType::Primitive(match lane {
        KirLaneType::I32 => MirPrimitiveTypeName::I32,
        KirLaneType::I64 => MirPrimitiveTypeName::I64,
        KirLaneType::U32 => MirPrimitiveTypeName::U32,
        KirLaneType::U64 => MirPrimitiveTypeName::U64,
        KirLaneType::F64 => MirPrimitiveTypeName::F64,
    })
}

const fn vector_binary_op(op: KirVectorBinaryOp, lane: KirLaneType) -> BridgeBinaryOp {
    let floating = matches!(lane, KirLaneType::F64);
    let unsigned = matches!(lane, KirLaneType::U32 | KirLaneType::U64);
    match (op, floating, unsigned) {
        (KirVectorBinaryOp::Add, true, _) => BridgeBinaryOp::FAdd,
        (KirVectorBinaryOp::Subtract, true, _) => BridgeBinaryOp::FSub,
        (KirVectorBinaryOp::Multiply, true, _) => BridgeBinaryOp::FMul,
        (KirVectorBinaryOp::Divide, true, _) => BridgeBinaryOp::FDiv,
        (KirVectorBinaryOp::Add, false, _) => BridgeBinaryOp::Add,
        (KirVectorBinaryOp::Subtract, false, _) => BridgeBinaryOp::Sub,
        (KirVectorBinaryOp::Multiply, false, _) => BridgeBinaryOp::Mul,
        (KirVectorBinaryOp::Divide, false, false) => BridgeBinaryOp::SDiv,
        (KirVectorBinaryOp::Divide, false, true) => BridgeBinaryOp::UDiv,
        (KirVectorBinaryOp::Remainder, false, false) => BridgeBinaryOp::SRem,
        (KirVectorBinaryOp::Remainder, false, true) => BridgeBinaryOp::URem,
        (KirVectorBinaryOp::Remainder, true, _) => BridgeBinaryOp::FDiv,
    }
}

fn kir_place_type(place: &KirPlace) -> &MirType {
    match place {
        KirPlace::Value { type_node, .. }
        | KirPlace::Deref { type_node, .. }
        | KirPlace::Index { type_node, .. }
        | KirPlace::SliceIndex { type_node, .. }
        | KirPlace::Field { type_node, .. } => type_node,
    }
}

fn kir_place_region(place: &KirPlace) -> MemoryRegionId {
    match place {
        KirPlace::Value { region, .. }
        | KirPlace::Deref { region, .. }
        | KirPlace::Index { region, .. }
        | KirPlace::SliceIndex { region, .. }
        | KirPlace::Field { region, .. } => *region,
    }
}

fn root_parameter_for_region(
    function: &KirFunction,
    mut region: MemoryRegionId,
) -> Option<ValueId> {
    let mut visited = HashSet::new();
    while visited.insert(region) {
        let descriptor = function
            .regions
            .iter()
            .find(|candidate| candidate.id == region)?;
        match descriptor.origin {
            KirMemoryRegionOrigin::Parameter(value) | KirMemoryRegionOrigin::RawSlice(value)
                if function.params.iter().any(|param| param.value == value) =>
            {
                return Some(value);
            }
            _ => {}
        }
        region = descriptor.parent?;
    }
    None
}

fn alias_scope_id(value: ValueId) -> Result<u32, NativeError> {
    value
        .index()
        .checked_add(1)
        .ok_or_else(|| lowering_error("KIR alias scope identity overflow"))
}

fn require_function<'module>(
    functions: &HashMap<String, NativeFunction<'module>>,
    name: &str,
) -> Result<NativeFunction<'module>, NativeError> {
    functions
        .get(name)
        .copied()
        .ok_or_else(|| lowering_error(format!("unknown KIR function '{name}'")))
}
