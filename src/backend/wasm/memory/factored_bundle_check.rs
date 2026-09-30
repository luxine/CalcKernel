//! Independent source reconstruction for block-local SIMD address sharing.
use super::factored_bundle::BundleAddressPlan;
use crate::{
    KirArithmeticSemantics, KirFunction, KirInstruction, KirInstructionKind, KirLaneType,
    KirVectorMemoryAccess, MirBinaryOp, MirPrimitiveTypeName, MirType, ValueId,
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn validate(function: &KirFunction, plan: &BundleAddressPlan) -> bool {
    if plan.bases.is_empty() && plan.accesses.is_empty() {
        return true;
    }
    if function.blocks.len() > 256 || plan.bases.len() > 16 || plan.accesses.len() > 128 {
        return false;
    }
    let mut types = BTreeMap::new();
    for p in &function.params {
        types.insert(p.value, Some(&p.type_node));
    }
    for block in &function.blocks {
        for p in &block.params {
            types.insert(p.value, p.type_node.as_scalar());
        }
        for i in &block.instructions {
            for r in &i.results {
                types.insert(r.value, r.type_node.as_scalar());
            }
        }
    }
    let mut seen = BTreeSet::new();
    let mut covered = BTreeSet::new();
    let mut remaining = 32_768_usize;
    for base in &plan.bases {
        if base.id >= 16 || !seen.insert(base.id) {
            return false;
        }
        let Some(block) = function.blocks.iter().find(|b| b.id == base.block) else {
            return false;
        };
        if block.instructions.len() > 512 {
            return false;
        }
        let Some(first_position) = block.instructions.iter().position(|i| i.id == base.first)
        else {
            return false;
        };
        let Some(first_access) = source_access(&block.instructions[first_position]) else {
            return false;
        };
        if first_access.slice != base.slice
            || first_access.start != base.start
            || types.get(&base.slice).copied().flatten()
                != Some(&MirType::Slice(Box::new(MirType::Primitive(
                    MirPrimitiveTypeName::F64,
                ))))
            || plan
                .accesses
                .get(&base.first)
                .is_none_or(|a| a.base != base.id || a.delta_bytes != 0)
        {
            return false;
        }
        let mut definitions = BTreeMap::new();
        for i in &block.instructions {
            for r in &i.results {
                if definitions.insert(r.value, i).is_some() {
                    return false;
                }
            }
        }
        let mut deltas = BTreeSet::new();
        for (id, mapped) in &plan.accesses {
            if mapped.base != base.id {
                continue;
            }
            let Some(position) = block.instructions.iter().position(|i| i.id == *id) else {
                return false;
            };
            if position < first_position {
                return false;
            }
            let Some(access) = source_access(&block.instructions[position]) else {
                return false;
            };
            if access.slice != base.slice
                || access.end != first_access.end
                || !matches!(mapped.delta_bytes, 0 | 16 | 32 | 48)
            {
                return false;
            }
            let Some(delta) = difference(
                access.start,
                base.start,
                &definitions,
                &types,
                &mut remaining,
            ) else {
                return false;
            };
            if delta != mapped.delta_bytes / 8 {
                return false;
            }
            deltas.insert(delta);
            covered.insert(*id);
        }
        if deltas != BTreeSet::from([0, 2, 4, 6]) {
            return false;
        }
    }
    covered.len() == plan.accesses.len()
}

fn source_access(i: &KirInstruction) -> Option<&KirVectorMemoryAccess> {
    let access = match &i.kind {
        KirInstructionKind::VectorLoad { access, .. }
        | KirInstructionKind::VectorStore { access, .. } => access,
        _ => return None,
    };
    (access.lane == KirLaneType::F64 && access.lanes == 2).then_some(access)
}

/// Expand the *difference* independently; cancellation must leave only delta.
/// Values outside this block are opaque SSA terms. We neither follow phis nor
/// assume that two descriptors in a MemorySSA partition denote the same slice.
fn difference<'a>(
    left: ValueId,
    right: ValueId,
    definitions: &BTreeMap<ValueId, &'a KirInstruction>,
    types: &BTreeMap<ValueId, Option<&'a MirType>>,
    remaining: &mut usize,
) -> Option<u32> {
    let mut pending = vec![(left, 1_u32, 0_u8), (right, u32::MAX, 0_u8)];
    let mut terms = BTreeMap::<ValueId, u32>::new();
    let mut constant = 0_u32;
    while let Some((value, coefficient, depth)) = pending.pop() {
        *remaining = remaining.checked_sub(1)?;
        if depth > 32
            || types.get(&value).copied().flatten()
                != Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
        {
            return None;
        }
        if let Some(i) = definitions.get(&value)
            && i.memory.is_none()
            && i.effect.is_none()
        {
            match &i.kind {
                KirInstructionKind::ConstInt { value } => {
                    constant =
                        constant.wrapping_add(coefficient.wrapping_mul(value.parse::<u32>().ok()?));
                    continue;
                }
                KirInstructionKind::Copy { value } => {
                    pending.push((*value, coefficient, depth + 1));
                    continue;
                }
                KirInstructionKind::Binary {
                    op: MirBinaryOp::Add,
                    semantics: KirArithmeticSemantics::Modular,
                    left,
                    right,
                } => {
                    pending.push((*left, coefficient, depth + 1));
                    pending.push((*right, coefficient, depth + 1));
                    continue;
                }
                _ => {}
            }
        }
        let count = terms.entry(value).or_default();
        *count = count.wrapping_add(coefficient);
    }
    terms.values().all(|count| *count == 0).then_some(constant)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn module() -> crate::KirModule {
        use crate::*;
        let source = r#"
export unsafe fn column(a: slice<f64>, b: slice<f64>, out: slice<f64>, n: u32) -> void contract {
 requires n != 0 && n <= a.len && n <= b.len && n <= out.len;
 requires noalias(a,b) && noalias(a,out) && noalias(b,out);
 effects read(a), read(b), readwrite(out);
} {
 let row: u32 = 0;
 while row < n {
  let inner: u32 = 0;
  while inner < n {
   let col: u32 = 0;
   while col < n {
    let oi: u32 = row*n+col;
    out[oi] = out[oi] + a[row*n+inner] * b[inner*n+col];
    col = col+1;
   }
   inner = inner+1;
  }
  row = row+1;
 }
}"#;
        let checked = check(&SourceFile::new("bundle.ck", source));
        assert!(checked.diagnostics.is_empty());
        let module = build_kir_module_with_profile(
            &lower_to_mir(&checked.checked_program).unwrap(),
            KirBuildConfig {
                consumer: KirConsumer::WebAssembly,
                overflow_mode: KirOverflowMode::Unchecked,
                bounds_mode: KirBoundsMode::Unchecked,
                sanitizer_mode: KirSanitizerMode::Disabled,
            },
            KirTargetProfile::webassembly_with_features(KirWasmFeatures::Simd128),
        )
        .unwrap();
        let optimized = run_kir_pass_pipeline(module, KirOptimizationLevel::O3, None);
        assert!(optimized.errors.is_empty());
        let module = optimized.artifact.unwrap();
        let contracts = import_contract_facts(&module, &checked.checked_program, 0).unwrap();
        let state = KirVerifiedProgramState::from_parts(
            module,
            Some(contracts),
            optimized.proofs,
            optimized.eliminated_guards,
            0,
        )
        .unwrap();
        let candidate = discover_vectorization_candidates(&state)
            .candidates
            .into_iter()
            .find(|c| c.uf == 4 && c.wasm_affine.is_some())
            .unwrap();
        let trial = prepare_vectorization_trial(&state, &candidate).unwrap();
        check_vectorization_trial_independently(&state, &trial.trial, &trial.plan, &trial.charge)
            .unwrap();
        trial.trial.module().clone()
    }

    fn function() -> KirFunction {
        module().functions.remove(0)
    }

    #[test]
    fn bundle_checker_rejects_false_delta_slice_and_initialization() {
        let function = function();
        let plan = super::super::factored_bundle::propose(&function);
        assert_eq!(plan.bases.len(), 2);
        assert_eq!(plan.accesses.len(), 12);
        assert!(validate(&function, &plan));
        let mut bad = plan.clone();
        bad.accesses
            .values_mut()
            .find(|a| a.delta_bytes == 16)
            .unwrap()
            .delta_bytes = 32;
        assert!(
            !validate(&function, &bad),
            "a byte delta must match source modular arithmetic"
        );
        let mut bad = plan.clone();
        bad.bases[0].slice = plan.bases[1].slice;
        assert!(
            !validate(&function, &bad),
            "same partition is not same descriptor"
        );
        let mut bad = plan.clone();
        bad.bases[0].first = *bad
            .accesses
            .iter()
            .find(|(_, a)| a.base == 0 && a.delta_bytes == 16)
            .unwrap()
            .0;
        assert!(
            !validate(&function, &bad),
            "base must be initialized before its first use"
        );
        let mut bad = plan.clone();
        bad.accesses.remove(&plan.bases[0].first);
        assert!(
            !validate(&function, &bad),
            "initialization access is mandatory"
        );
        let mut bad = plan.clone();
        bad.bases[0].block = function.blocks[0].id;
        assert!(!validate(&function, &bad), "sharing is local to one block");
    }

    #[test]
    fn bundle_checker_rejects_changed_end_in_valid_vector_kir() {
        let mut module = module();
        let function = &mut module.functions[0];
        let plan = super::super::factored_bundle::propose(function);
        let changed = *plan
            .accesses
            .iter()
            .find(|(_, a)| a.delta_bytes == 16)
            .unwrap()
            .0;
        for instruction in function.blocks.iter_mut().flat_map(|b| &mut b.instructions) {
            if instruction.id == changed {
                match &mut instruction.kind {
                    KirInstructionKind::VectorLoad { access, .. }
                    | KirInstructionKind::VectorStore { access, .. } => access.end = access.start,
                    _ => unreachable!(),
                }
            }
        }
        assert!(!validate(function, &plan));
        let validation = crate::validate_kir_module(&module);
        assert!(validation.errors.is_empty(), "{:?}", validation.errors);
    }

    #[test]
    fn bundle_checker_reconstructs_changed_source_and_fails_closed_on_budget() {
        let mut module = module();
        let function = &mut module.functions[0];
        let plan = super::super::factored_bundle::propose(function);
        let changed = *plan
            .accesses
            .iter()
            .find(|(_, a)| a.delta_bytes == 16)
            .unwrap()
            .0;
        let first = plan.bases[plan.accesses[&changed].base as usize].start;
        for i in function.blocks.iter_mut().flat_map(|b| &mut b.instructions) {
            if i.id == changed {
                match &mut i.kind {
                    KirInstructionKind::VectorLoad { access, .. }
                    | KirInstructionKind::VectorStore { access, .. } => access.start = first,
                    _ => unreachable!(),
                }
            }
        }
        assert!(
            !validate(function, &plan),
            "a stale start map cannot be trusted"
        );
        let validation = crate::validate_kir_module(&module);
        assert!(validation.errors.is_empty(), "{:?}", validation.errors);
        let mut oversized = plan.clone();
        oversized.bases.resize(17, plan.bases[0].clone());
        assert!(!validate(&module.functions[0], &oversized));
    }
}
