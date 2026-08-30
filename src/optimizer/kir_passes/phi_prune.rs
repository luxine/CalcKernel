use std::collections::{BTreeMap, BTreeSet};

use crate::{
    ContractFactAffineExpression, ContractFactAffineTerm, ContractFactPointer,
    ContractFactPredicate, ContractFactSet, FactPredicate, KirEdge, KirFunction,
    KirMemoryRegionOrigin, KirTerminator, ValueId,
};

fn edges(terminator: &KirTerminator) -> impl Iterator<Item = &KirEdge> {
    match terminator {
        KirTerminator::Return { .. } => [None, None],
        KirTerminator::Jump { edge } => [Some(edge), None],
        KirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => [Some(then_edge), Some(else_edge)],
    }
    .into_iter()
    .flatten()
}

pub(super) fn remove_dead_block_parameters(
    function: &mut KirFunction,
    protected: &BTreeSet<ValueId>,
) -> bool {
    // Only pre-proof CFG canonicalization calls this transform. Instructions and
    // Memory SSA stay intact, so every instruction operand remains a liveness root.
    let blocks = function
        .blocks
        .iter()
        .map(|block| (block.id, block))
        .collect::<BTreeMap<_, _>>();
    if blocks.len() != function.blocks.len() {
        return false;
    }
    let mut live = protected.clone();
    let mut incoming = BTreeMap::<ValueId, Vec<ValueId>>::new();
    for region in &function.regions {
        match region.origin {
            KirMemoryRegionOrigin::Conservative => {}
            KirMemoryRegionOrigin::Parameter(value)
            | KirMemoryRegionOrigin::RawSlice(value)
            | KirMemoryRegionOrigin::Subslice(value) => {
                live.insert(value);
            }
        }
        if let Some(interval) = &region.byte_interval {
            live.extend([interval.start, interval.end]);
        }
    }
    for block in &function.blocks {
        for instruction in &block.instructions {
            live.extend(super::dce::instruction_uses(instruction));
        }
        match &block.terminator {
            KirTerminator::Return { value, .. } => live.extend(value),
            KirTerminator::Branch { condition, .. } => {
                live.insert(*condition);
            }
            KirTerminator::Jump { .. } => {}
        }
        for edge in edges(&block.terminator) {
            let Some(target) = blocks.get(&edge.target) else {
                return false;
            };
            if edge.args.len() != target.params.len() {
                return false;
            }
            for (param, argument) in target.params.iter().zip(&edge.args) {
                incoming.entry(param.value).or_default().push(*argument);
            }
        }
    }
    let mut pending = live.iter().copied().collect::<Vec<_>>();
    // Follow every incoming arm, including parallel edges. A phi-only cycle is
    // removable only when no instruction, terminator, metadata or contract roots it.
    while let Some(value) = pending.pop() {
        if let Some(arguments) = incoming.get(&value) {
            for &argument in arguments {
                if live.insert(argument) {
                    pending.push(argument);
                }
            }
        }
    }
    let masks = function
        .blocks
        .iter()
        .filter_map(|block| {
            let keep = block
                .params
                .iter()
                .map(|param| live.contains(&param.value))
                .collect::<Vec<_>>();
            keep.iter().any(|keep| !keep).then_some((block.id, keep))
        })
        .collect::<BTreeMap<_, _>>();
    if masks.is_empty() {
        return false;
    }
    for block in &mut function.blocks {
        if let Some(keep) = masks.get(&block.id) {
            let mut index = 0;
            block.params.retain(|_| {
                let retain = keep[index];
                index += 1;
                retain
            });
        }
        let outgoing = match &mut block.terminator {
            KirTerminator::Return { .. } => [None, None],
            KirTerminator::Jump { edge } => [Some(edge), None],
            KirTerminator::Branch {
                then_edge,
                else_edge,
                ..
            } => [Some(then_edge), Some(else_edge)],
        };
        for edge in outgoing.into_iter().flatten() {
            if let Some(keep) = masks.get(&edge.target) {
                let mut index = 0;
                edge.args.retain(|_| {
                    let retain = keep[index];
                    index += 1;
                    retain
                });
            }
        }
    }
    true
}

fn affine_values(values: &mut BTreeSet<ValueId>, expression: &ContractFactAffineExpression) {
    values.extend(expression.terms.iter().map(|term| match term.term {
        ContractFactAffineTerm::Value(value) | ContractFactAffineTerm::SliceLength(value) => value,
    }));
}

pub(super) fn protected_contract_values(contracts: Option<&ContractFactSet>) -> BTreeSet<ValueId> {
    let mut values = BTreeSet::new();
    if let Some(contracts) = contracts {
        values.extend(
            contracts
                .instances()
                .iter()
                .flat_map(|instance| &instance.bindings)
                .map(|binding| binding.value),
        );
        for fact in contracts.facts().facts() {
            match &fact.predicate {
                FactPredicate::ValueInterval { value, .. } => {
                    values.insert(*value);
                }
                FactPredicate::Contract(predicate) => match predicate {
                    ContractFactPredicate::Comparison { left, right, .. } => {
                        affine_values(&mut values, left);
                        affine_values(&mut values, right);
                    }
                    ContractFactPredicate::MultipleOf { value, .. } => {
                        affine_values(&mut values, value)
                    }
                    ContractFactPredicate::NoAlias { left, right } => {
                        values.extend([*left, *right]);
                    }
                    ContractFactPredicate::Aligned { pointer, .. } => {
                        let (ContractFactPointer::Value(value)
                        | ContractFactPointer::SliceData(value)) = pointer;
                        values.insert(*value);
                    }
                    ContractFactPredicate::EffectCeiling { items, .. } => {
                        values.extend(items.iter().map(|(value, _)| *value))
                    }
                },
            }
        }
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;

    fn build_source(text: &str) -> (KirModule, CheckedProgram) {
        let checked = check(&SourceFile::new("phi-prune.ck", text));
        assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
        let mir = lower_to_mir(&checked.checked_program).expect("MIR");
        let module = build_kir_module(
            &mir,
            KirBuildConfig {
                consumer: KirConsumer::Inspection,
                overflow_mode: KirOverflowMode::Unchecked,
                bounds_mode: KirBoundsMode::Unchecked,
                sanitizer_mode: KirSanitizerMode::Disabled,
            },
        )
        .expect("KIR");
        assert!(validate_kir_module(&module).errors.is_empty());
        (module, checked.checked_program)
    }

    fn fixture() -> KirFunction {
        build_source("export fn count(n: u32) -> u32 { let unused: u32 = 42; let i: u32 = 0; while i < n { i = i + 1; } return i; }").0.functions.remove(0)
    }

    fn assert_shape(before: &KirFunction, after: &KirFunction) {
        assert_eq!(before.params, after.params);
        assert_eq!(before.regions, after.regions);
        assert_eq!(before.initial_memory, after.initial_memory);
        assert_eq!(before.blocks.len(), after.blocks.len());
        for (old, new) in before.blocks.iter().zip(&after.blocks) {
            assert_eq!(old.id, new.id);
            assert_eq!(old.instructions, new.instructions);
            assert_eq!(old.memory_params, new.memory_params);
            assert_eq!(
                std::mem::discriminant(&old.terminator),
                std::mem::discriminant(&new.terminator)
            );
            match (&old.terminator, &new.terminator) {
                (KirTerminator::Return { .. }, KirTerminator::Return { .. }) => {
                    assert_eq!(old.terminator, new.terminator)
                }
                (
                    KirTerminator::Branch { condition: a, .. },
                    KirTerminator::Branch { condition: b, .. },
                ) => assert_eq!(a, b),
                _ => {}
            }
            for (old_edge, new_edge) in edges(&old.terminator).zip(edges(&new.terminator)) {
                assert_eq!(old_edge.target, new_edge.target);
                assert_eq!(old_edge.memory_args, new_edge.memory_args);
                let old_target = before
                    .blocks
                    .iter()
                    .find(|block| block.id == old_edge.target)
                    .unwrap();
                let new_target = after
                    .blocks
                    .iter()
                    .find(|block| block.id == new_edge.target)
                    .unwrap();
                let expected = old_target
                    .params
                    .iter()
                    .zip(&old_edge.args)
                    .filter(|(param, _)| {
                        new_target.params.iter().any(|new| new.value == param.value)
                    })
                    .map(|(_, value)| *value)
                    .collect::<Vec<_>>();
                assert_eq!(new_edge.args, expected);
            }
        }
    }

    #[test]
    fn phi_prune_should_remove_unrooted_cycles_and_preserve_every_operation() {
        let before = fixture();
        let mut after = before.clone();
        assert!(remove_dead_block_parameters(&mut after, &BTreeSet::new()));
        assert!(
            !after
                .blocks
                .iter()
                .flat_map(|block| &block.params)
                .any(|param| param.slot == "unused")
        );
        assert_shape(&before, &after);
        assert!(!remove_dead_block_parameters(&mut after, &BTreeSet::new()));
    }

    #[test]
    fn phi_prune_should_preserve_protected_and_metadata_roots_without_slot_identity() {
        let original = fixture();
        let unused = original
            .blocks
            .iter()
            .flat_map(|block| &block.params)
            .filter(|param| param.slot == "unused")
            .map(|param| param.value)
            .collect::<Vec<_>>();
        assert!(unused.len() >= 2);
        for metadata in [false, true] {
            let mut before = original.clone();
            for param in before.blocks.iter_mut().flat_map(|block| &mut block.params) {
                param.slot = "same-name".into();
            }
            let protected = if metadata {
                before.regions[0].byte_interval = Some(KirSymbolicByteInterval {
                    start: unused[0],
                    end: unused[1],
                    element_type: MirType::Primitive(MirPrimitiveTypeName::U32),
                });
                BTreeSet::new()
            } else {
                BTreeSet::from([unused[0], unused[1]])
            };
            let mut after = before.clone();
            remove_dead_block_parameters(&mut after, &protected);
            for value in &unused[..2] {
                assert!(
                    after
                        .blocks
                        .iter()
                        .flat_map(|block| &block.params)
                        .any(|param| param.value == *value)
                );
            }
            assert_shape(&before, &after);
        }
    }

    #[test]
    fn phi_prune_should_keep_parallel_live_arguments_and_ignore_storage_order() {
        let mut before = fixture();
        for block in &mut before.blocks {
            if let KirTerminator::Branch {
                then_edge,
                else_edge,
                ..
            } = &mut block.terminator
            {
                *else_edge = then_edge.clone();
                // Differing but defined incoming values must remain separate.
                else_edge.args.rotate_left(1);
            }
        }
        let mut normal = before.clone();
        remove_dead_block_parameters(&mut normal, &BTreeSet::new());
        assert_shape(&before, &normal);
        let mut reordered = before;
        reordered.blocks[1..].reverse();
        remove_dead_block_parameters(&mut reordered, &BTreeSet::new());
        reordered.blocks.sort_by_key(|block| block.id);
        normal.blocks.sort_by_key(|block| block.id);
        assert_eq!(reordered, normal);
    }

    #[test]
    fn phi_prune_should_not_partially_mutate_malformed_cfg() {
        for missing_target in [false, true] {
            let mut before = fixture();
            let edge = before
                .blocks
                .iter_mut()
                .find_map(|block| match &mut block.terminator {
                    KirTerminator::Jump { edge } => Some(edge),
                    _ => None,
                })
                .unwrap();
            if missing_target {
                edge.target = BlockId::from_index(u32::MAX);
            } else {
                assert!(edge.args.pop().is_some());
            }
            let mut after = before.clone();
            assert!(!remove_dead_block_parameters(&mut after, &BTreeSet::new()));
            assert_eq!(after, before);
        }
    }

    #[test]
    fn phi_roots_should_cover_every_fact_predicate() {
        let (module, checked) = build_source(
            "export unsafe fn f(n: u32) -> u32 contract { requires n < 8; } { return n; }",
        );
        let contracts = import_contract_facts(&module, &checked, 0).unwrap();
        let a = ValueId::from_index(900);
        let b = ValueId::from_index(901);
        let affine = |value| ContractFactAffineExpression {
            terms: vec![ContractFactAffineTermCoefficient {
                term: ContractFactAffineTerm::Value(value),
                coefficient: 1.into(),
            }],
            constant: 0.into(),
        };
        let predicates = [
            (
                FactPredicate::ValueInterval {
                    value: a,
                    interval: ScalarInterval::new(0.into(), 1.into()).unwrap(),
                },
                vec![a],
            ),
            (
                FactPredicate::Contract(ContractFactPredicate::Comparison {
                    operator: "<".into(),
                    left: affine(a),
                    right: affine(b),
                }),
                vec![a, b],
            ),
            (
                FactPredicate::Contract(ContractFactPredicate::MultipleOf {
                    value: affine(a),
                    modulus: 2.into(),
                }),
                vec![a],
            ),
            (
                FactPredicate::Contract(ContractFactPredicate::MultipleOf {
                    value: ContractFactAffineExpression {
                        terms: vec![ContractFactAffineTermCoefficient {
                            term: ContractFactAffineTerm::SliceLength(a),
                            coefficient: 1.into(),
                        }],
                        constant: 0.into(),
                    },
                    modulus: 2.into(),
                }),
                vec![a],
            ),
            (
                FactPredicate::Contract(ContractFactPredicate::NoAlias { left: a, right: b }),
                vec![a, b],
            ),
            (
                FactPredicate::Contract(ContractFactPredicate::Aligned {
                    pointer: ContractFactPointer::Value(a),
                    alignment: 8,
                }),
                vec![a],
            ),
            (
                FactPredicate::Contract(ContractFactPredicate::Aligned {
                    pointer: ContractFactPointer::SliceData(a),
                    alignment: 8,
                }),
                vec![a],
            ),
            (
                FactPredicate::Contract(ContractFactPredicate::EffectCeiling {
                    is_none: false,
                    items: vec![(a, ContractEffectKind::Read)],
                }),
                vec![a],
            ),
        ];
        for (predicate, extra) in predicates {
            // Root extraction only: mutated facts are not asserted to be valid evidence.
            let mut candidate = contracts.clone();
            candidate
                .facts_mut()
                .get_mut(FactId::from_index(0))
                .unwrap()
                .predicate = predicate;
            let mut expected = candidate
                .instances()
                .iter()
                .flat_map(|instance| &instance.bindings)
                .map(|binding| binding.value)
                .collect::<BTreeSet<_>>();
            expected.extend(extra);
            assert_eq!(protected_contract_values(Some(&candidate)), expected);
        }
    }
}
