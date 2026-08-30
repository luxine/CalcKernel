# Conservative SSA Phi Pruning Implementation Plan

> **For agentic workers:** Use `superpowers:executing-plans` task-by-task, entirely
> inline in the existing worktree. The user's no-subagent/no-main-merge constraints apply.

**Goal:** Remove unused scalar block parameters before expensive analyses without
changing instructions, ordered effects, Memory SSA, public parameters, or safety gates.

**Architecture:** Refine the already-approved CFG canonicalization/phi repair in
stage 05, as the stage-11 I20 performance repair. Compute conservative backward
liveness over actual SSA inputs, then atomically filter block parameters and all
corresponding edge arguments. Do not change O0 or introduce a new named pass.

**Tech Stack:** Existing Rust KIR, BTreeMap/BTreeSet, structural and evidence verifiers,
current C/WASM/Native differential corpus, Rust 1.90.0 / LLVM and Clang 22.1.8.

## Evidence and design boundary

At `7611fa6`, the original full gate reports Dijkstra `1112250 / 350000 ns`, above
the unchanged 3x limit. Read-only graph inspection finds 248 conservatively unused
scalar parameters among 456 in the input `dijkstra_matrix`, and 241 among 436 after
O3. Every instruction operand is a root in that inspection, even for pure instructions.
These counts are diagnostic evidence, not acceptance or a timing claim.

Three implementation locations were considered:

- Existing pre-guard CFG canonicalization: reduces all later analysis workloads and
  has no persistent guard certificates yet. Selected.
- Pruned SSA construction: would affect O0 and the builder contract. Not selected.
- Final DCE only: cannot reduce most preceding pass work. Not selected.

This is an implementation refinement, not a relaxed language or performance contract.
The pipeline already invokes CFG canonicalization only before persistent guard proofs.
Keep that boundary. Contract imports are refreshed through the existing path after a
CFG change. Protect all contract binding and fact-predicate ValueIds, not just names.

Liveness roots are all instruction operands, branch conditions, return values,
region origins and symbolic interval endpoints, and protected contract/fact values.
For a live target block parameter, every incoming argument is live, including both
arms when their predecessor and target are identical. A cycle without a root is dead;
a cycle reaching any root is retained. Never use source slot names as identity.

The worklist visits each newly-live ValueId once and each of its incoming dependencies
once. It terminates structurally, without wall-clock budgets or speculative facts.
Malformed/missing-target or mismatched scalar arity is a conservative no-op before
mutation; the unchanged enclosing verifier still rejects malformed KIR.

## Task 1: Lock the missing behavior

**Files:** modify `tests/optimizer/kir_o1.rs` using its existing build helpers.

- [x] Add the following regression and run it before implementation:

```rust
#[test]
fn kir_o1_cfg_should_prune_dead_phi_cycles_without_changing_o0() {
    let (_, kir, contracts) = build_with_overflow(
        "export fn count(n: u32) -> u32 { let unused: u32 = 42; let i: u32 = 0; while i < n { i = i + 1; } return i; }",
        KirOverflowMode::Unchecked,
    );
    let unused = |module: &calckernel::KirModule| module.functions.iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.params)
        .filter(|param| param.slot == "unused").count();
    assert!(unused(&kir) > 0);
    let o0 = run_kir_pass_pipeline(kir.clone(), KirOptimizationLevel::O0, contracts.as_ref());
    assert_eq!(o0.module, kir);
    let o1 = run_kir_pass_pipeline(kir, KirOptimizationLevel::O1, contracts.as_ref());
    assert!(o1.errors.is_empty(), "{:?}", o1.errors);
    assert_eq!(unused(o1.artifact.as_ref().expect("verified artifact")), 0);
}
```

Run `cargo +1.90.0 test --locked --test optimizer kir_o1_cfg_should_prune_dead_phi_cycles`.
Expected red: unused cyclic phi parameters remain, not a parser/build error. If the
fixture is already optimized, replace it with an unused nonconstant function parameter
forwarded around the same loop; do not weaken the assertion or alter the public ABI.

## Task 2: Implement a local, conservative graph transform

**Files:** create `src/optimizer/kir_passes/phi_prune.rs`; register private
`mod phi_prune;` in `src/optimizer/kir_passes/mod.rs`; integrate in `cfg.rs`.

- [x] Implement the following transform; existing `dce::instruction_uses` remains
  the operand source, avoiding a second instruction-effect classification:

```rust
use std::collections::{BTreeMap, BTreeSet};
use crate::{KirEdge, KirFunction, KirMemoryRegionOrigin, KirTerminator, ValueId};

fn edges(terminator: &KirTerminator) -> impl Iterator<Item = &KirEdge> {
    match terminator {
        KirTerminator::Return { .. } => [None, None],
        KirTerminator::Jump { edge } => [Some(edge), None],
        KirTerminator::Branch { then_edge, else_edge, .. } => [Some(then_edge), Some(else_edge)],
    }.into_iter().flatten()
}

pub(super) fn remove_dead_block_parameters(
    function: &mut KirFunction,
    protected: &BTreeSet<ValueId>,
) -> bool {
    let blocks = function.blocks.iter().map(|block| (block.id, block))
        .collect::<BTreeMap<_, _>>();
    if blocks.len() != function.blocks.len() { return false; }
    let mut live = protected.clone();
    let mut incoming = BTreeMap::<ValueId, Vec<ValueId>>::new();
    for region in &function.regions {
        match region.origin {
            KirMemoryRegionOrigin::Conservative => {}
            KirMemoryRegionOrigin::Parameter(value)
            | KirMemoryRegionOrigin::RawSlice(value)
            | KirMemoryRegionOrigin::Subslice(value) => { live.insert(value); }
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
            KirTerminator::Branch { condition, .. } => { live.insert(*condition); }
            KirTerminator::Jump { .. } => {}
        }
        for edge in edges(&block.terminator) {
            let Some(target) = blocks.get(&edge.target) else { return false };
            if edge.args.len() != target.params.len() { return false; }
            for (param, argument) in target.params.iter().zip(&edge.args) {
                incoming.entry(param.value).or_default().push(*argument);
            }
        }
    }
    let mut pending = live.iter().copied().collect::<Vec<_>>();
    while let Some(value) = pending.pop() {
        if let Some(arguments) = incoming.get(&value) {
            for &argument in arguments {
                if live.insert(argument) { pending.push(argument); }
            }
        }
    }
    let masks = function.blocks.iter().filter_map(|block| {
        let keep = block.params.iter().map(|param| live.contains(&param.value)).collect::<Vec<_>>();
        keep.iter().any(|keep| !keep).then_some((block.id, keep))
    }).collect::<BTreeMap<_, _>>();
    if masks.is_empty() { return false; }
    for block in &mut function.blocks {
        if let Some(keep) = masks.get(&block.id) {
            let mut index = 0;
            block.params.retain(|_| { let retain = keep[index]; index += 1; retain });
        }
        let outgoing = match &mut block.terminator {
            KirTerminator::Return { .. } => [None, None],
            KirTerminator::Jump { edge } => [Some(edge), None],
            KirTerminator::Branch { then_edge, else_edge, .. } => [Some(then_edge), Some(else_edge)],
        };
        for edge in outgoing.into_iter().flatten() {
            if let Some(keep) = masks.get(&edge.target) {
                let mut index = 0;
                edge.args.retain(|_| { let retain = keep[index]; index += 1; retain });
            }
        }
    }
    true
}
```

- [x] Add `protected_contract_values` in the same module; its exact closed-enum
  traversal is as follows:

```rust
use crate::{ContractFactAffineExpression, ContractFactAffineTerm, ContractFactPointer,
    ContractFactPredicate, ContractFactSet, FactPredicate};

fn affine_values(values: &mut BTreeSet<ValueId>, expression: &ContractFactAffineExpression) {
    values.extend(expression.terms.iter().map(|term| match term.term {
        ContractFactAffineTerm::Value(value) | ContractFactAffineTerm::SliceLength(value) => value,
    }));
}

pub(super) fn protected_contract_values(contracts: Option<&ContractFactSet>) -> BTreeSet<ValueId> {
    let mut values = BTreeSet::new();
    if let Some(contracts) = contracts {
        values.extend(contracts.instances().iter().flat_map(|instance| &instance.bindings)
            .map(|binding| binding.value));
        for fact in contracts.facts().facts() {
            match &fact.predicate {
                FactPredicate::ValueInterval { value, .. } => { values.insert(*value); }
                FactPredicate::Contract(predicate) => match predicate {
                    ContractFactPredicate::Comparison { left, right, .. } => {
                        affine_values(&mut values, left); affine_values(&mut values, right);
                    }
                    ContractFactPredicate::MultipleOf { value, .. } => affine_values(&mut values, value),
                    ContractFactPredicate::NoAlias { left, right } => { values.extend([*left, *right]); }
                    ContractFactPredicate::Aligned { pointer, .. } => {
                        let (ContractFactPointer::Value(value) | ContractFactPointer::SliceData(value)) = pointer;
                        values.insert(*value);
                    }
                    ContractFactPredicate::EffectCeiling { items, .. } => values.extend(items.iter().map(|(value, _)| *value)),
                },
            }
        }
    }
    values
}
```

- [x] In `run_cfg_canonicalize`, replace the binding-only `protected` construction
  with `super::phi_prune::protected_contract_values(contracts)`; after the existing
  final same-edge branch fold for each function, add:

```rust
changed |= super::phi_prune::remove_dead_block_parameters(function, &protected);
```

- [x] In `run_pre_guard_sccp` update its termination explanation: every further CFG
  round removes a branch, block, **or scalar block parameter**; no stage in that
  pre-guard loop adds such parameters. Keep all verifier calls and refresh logic.
- [x] Run the Task 1 regression, all `--lib`, `--test ir`, and `--test optimizer` tests.
  Expect the new behavior to pass with no changed exact pass-order assertions.

## Task 3: Adversarial preservation and total validation

**Files:** unit tests in `phi_prune.rs`; existing backend/optimizer/CLI acceptance suites.

- [x] Before broad acceptance, add these helper-level mutation tests. They preserve
  every instruction and memory/effect field, rather than merely checking a count:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;

    fn build_source(text: &str) -> (KirModule, CheckedProgram) {
        let checked = check(&SourceFile::new("phi-prune.ck", text));
        assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
        let mir = lower_to_mir(&checked.checked_program).expect("MIR");
        let module = build_kir_module(&mir, KirBuildConfig {
            consumer: KirConsumer::Inspection,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        }).expect("KIR");
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
            assert_eq!(std::mem::discriminant(&old.terminator), std::mem::discriminant(&new.terminator));
            match (&old.terminator, &new.terminator) {
                (KirTerminator::Return { .. }, KirTerminator::Return { .. }) => assert_eq!(old.terminator, new.terminator),
                (KirTerminator::Branch { condition: a, .. }, KirTerminator::Branch { condition: b, .. }) => assert_eq!(a, b),
                _ => {}
            }
            for (old_edge, new_edge) in edges(&old.terminator).zip(edges(&new.terminator)) {
                assert_eq!(old_edge.target, new_edge.target);
                assert_eq!(old_edge.memory_args, new_edge.memory_args);
                let old_target = before.blocks.iter().find(|block| block.id == old_edge.target).unwrap();
                let new_target = after.blocks.iter().find(|block| block.id == new_edge.target).unwrap();
                let expected = old_target.params.iter().zip(&old_edge.args)
                    .filter(|(param, _)| new_target.params.iter().any(|new| new.value == param.value))
                    .map(|(_, value)| *value).collect::<Vec<_>>();
                assert_eq!(new_edge.args, expected);
            }
        }
    }

    #[test]
    fn phi_prune_should_remove_unrooted_cycles_and_preserve_every_operation() {
        let before = fixture();
        let mut after = before.clone();
        assert!(remove_dead_block_parameters(&mut after, &BTreeSet::new()));
        assert!(!after.blocks.iter().flat_map(|block| &block.params).any(|param| param.slot == "unused"));
        assert_shape(&before, &after);
        assert!(!remove_dead_block_parameters(&mut after, &BTreeSet::new()));
    }

    #[test]
    fn phi_prune_should_preserve_protected_and_metadata_roots_without_slot_identity() {
        let original = fixture();
        let unused = original.blocks.iter().flat_map(|block| &block.params)
            .filter(|param| param.slot == "unused").map(|param| param.value).collect::<Vec<_>>();
        assert!(unused.len() >= 2);
        for metadata in [false, true] {
            let mut before = original.clone();
            for param in before.blocks.iter_mut().flat_map(|block| &mut block.params) {
                param.slot = "same-name".into();
            }
            let protected = if metadata {
                before.regions[0].byte_interval = Some(KirSymbolicByteInterval {
                    start: unused[0], end: unused[1],
                    element_type: MirType::Primitive(MirPrimitiveTypeName::U32),
                });
                BTreeSet::new()
            } else { BTreeSet::from([unused[0], unused[1]]) };
            let mut after = before.clone();
            remove_dead_block_parameters(&mut after, &protected);
            for value in &unused[..2] {
                assert!(after.blocks.iter().flat_map(|block| &block.params).any(|param| param.value == *value));
            }
            assert_shape(&before, &after);
        }
    }

    #[test]
    fn phi_prune_should_keep_parallel_live_arguments_and_ignore_storage_order() {
        let mut before = fixture();
        for block in &mut before.blocks {
            if let KirTerminator::Branch { then_edge, else_edge, .. } = &mut block.terminator {
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
            let edge = before.blocks.iter_mut().find_map(|block| match &mut block.terminator {
                KirTerminator::Jump { edge } => Some(edge),
                _ => None,
            }).unwrap();
            if missing_target { edge.target = BlockId::from_index(u32::MAX); }
            else { assert!(edge.args.pop().is_some()); }
            let mut after = before.clone();
            assert!(!remove_dead_block_parameters(&mut after, &BTreeSet::new()));
            assert_eq!(after, before);
        }
    }

    #[test]
    fn phi_roots_should_cover_every_fact_predicate() {
        let (module, checked) = build_source("export unsafe fn f(n: u32) -> u32 contract { requires n < 8; } { return n; }");
        let contracts = import_contract_facts(&module, &checked, 0).unwrap();
        let a = ValueId::from_index(900);
        let b = ValueId::from_index(901);
        let affine = |value| ContractFactAffineExpression {
            terms: vec![ContractFactAffineTermCoefficient { term: ContractFactAffineTerm::Value(value), coefficient: 1.into() }],
            constant: 0.into(),
        };
        let predicates = [
            (FactPredicate::ValueInterval { value: a, interval: ScalarInterval::new(0.into(), 1.into()).unwrap() }, vec![a]),
            (FactPredicate::Contract(ContractFactPredicate::Comparison { operator: "<".into(), left: affine(a), right: affine(b) }), vec![a, b]),
            (FactPredicate::Contract(ContractFactPredicate::MultipleOf { value: affine(a), modulus: 2.into() }), vec![a]),
            (FactPredicate::Contract(ContractFactPredicate::MultipleOf { value: ContractFactAffineExpression { terms: vec![ContractFactAffineTermCoefficient { term: ContractFactAffineTerm::SliceLength(a), coefficient: 1.into() }], constant: 0.into() }, modulus: 2.into() }), vec![a]),
            (FactPredicate::Contract(ContractFactPredicate::NoAlias { left: a, right: b }), vec![a, b]),
            (FactPredicate::Contract(ContractFactPredicate::Aligned { pointer: ContractFactPointer::Value(a), alignment: 8 }), vec![a]),
            (FactPredicate::Contract(ContractFactPredicate::Aligned { pointer: ContractFactPointer::SliceData(a), alignment: 8 }), vec![a]),
            (FactPredicate::Contract(ContractFactPredicate::EffectCeiling { is_none: false, items: vec![(a, ContractEffectKind::Read)] }), vec![a]),
        ];
        for (predicate, extra) in predicates {
            // Root extraction only: mutated facts are not asserted to be valid evidence.
            let mut candidate = contracts.clone();
            candidate.facts_mut().get_mut(FactId::from_index(0)).unwrap().predicate = predicate;
            let mut expected = candidate.instances().iter().flat_map(|instance| &instance.bindings)
                .map(|binding| binding.value).collect::<BTreeSet<_>>();
            expected.extend(extra);
            assert_eq!(protected_contract_values(Some(&candidate)), expected);
        }
    }
}
```

These tests refine the existing mutation and determinism gates rather than replacing them.
The existing empty-branch forwarding fixture carries an unused `flag` phi after folding.
Update its exact expectation to one live `n` phi/argument and its matching return, while
explicitly retaining both public parameters and the original complete memory transfer.
The old two-argument assertion encoded the redundant representation, not a safety contract.
The synthetic parallel-edge and metadata variants exercise helper invariants; they do
not replace executable well-typed backend tests or authorize malformed artifact emission.

- [x] Run default and all-feature tests **sequentially**. Native runs use the existing
  pinned LLVM prefix, Clang oracle, and configured TypeScript root; do not share a target
  directory with a simultaneous default build. Run release `--lib`, release `--test ir`,
  all-feature/all-target Clippy with `-D warnings`, `cargo fmt --check`, `git diff --check`.
- [x] Re-run all generated C/WASM/Native mode matrices, checked first-error/print/strict-FP
  and proof-cache fault tests (already included in the above suites). No ignore or corpus
  reduction is allowed. Inspect exact pass records and successful independent verification.
- [x] Re-run read-only phi counts and record reductions; output KIR may legitimately
  change now, so replace byte-identity evidence with structural and executable equivalence.
- [x] Commit code and evidence on the feature worktree; run the unchanged complete native
  benchmark/checker once at the new SHA without this task's concurrent builds. Preserve
  the first report whether passing or failing. Do not claim I20 closed before its gates pass.

## Self-review / scope check

Executed at `930f18d102266424bcc08256f8ea0c129c926dd0`: the first unchanged complete
benchmark and checker both exit 0. Dijkstra is `789625 / 350000 ns = 2.2561x`,
suite-median optimizer ratio `1.1114`; all runtime gates pass. See the I20 gate
record in the implementation review. This closes local I20, not the final remote matrix.

- No O0, builder, public ABI, language syntax, Memory SSA or new named loop pass change.
- No instruction removal in this helper; existing DCE remains responsible for dead pure
  instructions, with ordered and may-fail operations still protected.
- Both parallel edges, nonlocal SSA uses, metadata, contracts, cycles, and invalid-input
  atomicity have explicit preservation tests. Persistent proof pruning is out of scope.
- Current normalized runtime, proof-throughput and 2x/3x compile-time gates are unchanged.
- I14/I19 remain separate open issues; all ten final CI jobs must pass at one final SHA.
- This supplemental plan is subordinate to `00-master-control.md`; it does not restart
  finished phases, authorize agents, or permit main/PR/tag/Release mutations.
