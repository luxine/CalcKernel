use super::super::transaction::CONTRACT_ENCODINGS;
use super::*;
use crate::{
    KirBoundsMode, KirBuildConfig, KirConsumer, KirOverflowMode, KirSanitizerMode, SourceFile,
    build_kir_module, check, import_contract_facts, lower_to_mir, prepare_kir_pre_tune_state,
    print_proof_arena,
};

fn state(source: &str, contracts: bool) -> Rc<KirVerifiedProgramState> {
    let checked = check(&SourceFile::new("contract-prefix.ck", source));
    assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let module = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::C,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .expect("KIR");
    let facts = contracts
        .then(|| import_contract_facts(&module, &checked.checked_program, 0).expect("facts"));
    Rc::new(prepare_kir_pre_tune_state(module, facts.as_ref()).expect("pre-tune state"))
}

fn original_evidence_digest(state: &KirVerifiedProgramState) -> String {
    let generation = state.evidence_generation();
    let contracts = state.contract_facts();
    let eliminated_guards = state.eliminated_guards();
    let original = format!(
        "generation={generation}\ncontracts={contracts:?}\n{}guards={eliminated_guards:?}",
        print_proof_arena(state.proofs()),
    );
    format!("{:x}", Sha256::digest(original.as_bytes()))
}

#[test]
fn contract_prefix_is_encoded_once_across_distinct_verified_search_phases() {
    for (source, contracts) in [
        ("export fn answer(n: i32) -> i32 { return n; }", false),
        (
            include_str!("../../benches/fixtures/pgo/branch_layout.ck"),
            true,
        ),
        (
            include_str!("../../benches/fixtures/pgo/call_constant_length.ck"),
            true,
        ),
    ] {
        let state = state(source, contracts);
        let expected_late = advance_to_tunable_late_o3(&state).expect("uncached late");
        let expected_finished = finish_selected_o3(&expected_late).expect("uncached finish");
        let checked = CheckedTuningSpace::enumerate(&state).expect("checked space");
        let mut replay = checked.search_replay();
        CONTRACT_ENCODINGS.with(|count| count.set(0));
        let late = replay
            .prefixes
            .analyze(ReplayPhase::Late, &state)
            .expect("cached late");
        let finished = replay
            .prefixes
            .analyze(ReplayPhase::Finish, &late)
            .expect("cached finish");
        let encodings = CONTRACT_ENCODINGS.with(|count| count.get());
        assert_eq!(late.as_ref(), &expected_late);
        assert_eq!(finished.as_ref(), &expected_finished);
        assert_eq!(
            late.verification_cache().evidence_digest,
            original_evidence_digest(&late)
        );
        assert_eq!(
            finished.verification_cache().evidence_digest,
            original_evidence_digest(&finished)
        );
        assert_eq!(
            encodings, 1,
            "unchanged contracts were encoded again by a distinct phase"
        );
    }
}

#[test]
fn contract_prefix_does_not_authorize_a_different_contract_set() {
    let root = state(
        include_str!("../../benches/fixtures/pgo/call_constant_length.ck"),
        true,
    );
    let different = state(
        include_str!("../../benches/fixtures/pgo/branch_layout.ck"),
        true,
    );
    assert_ne!(root.contract_facts(), different.contract_facts());
    let expected = advance_to_tunable_late_o3(&different).expect("uncached different state");
    let checked = CheckedTuningSpace::enumerate(&root).expect("space");
    let mut replay = checked.search_replay();
    replay
        .prefixes
        .analyze(ReplayPhase::Late, &root)
        .expect("prime root");
    let actual = replay
        .prefixes
        .analyze(ReplayPhase::Late, &different)
        .expect("different state");
    assert_eq!(actual.as_ref(), &expected);
    assert_eq!(
        actual.verification_cache().evidence_digest,
        original_evidence_digest(&actual)
    );
}

#[test]
fn contract_prefix_does_not_authorize_a_different_generation() {
    let root = state("export fn answer(n: i32) -> i32 { return n; }", false);
    let different = Rc::new(
        KirVerifiedProgramState::new(root.module().clone(), None, 7).expect("generation seven"),
    );
    let expected = advance_to_tunable_late_o3(&different).expect("uncached generation seven");
    let checked = CheckedTuningSpace::enumerate(&root).expect("space");
    let mut replay = checked.search_replay();
    replay
        .prefixes
        .analyze(ReplayPhase::Late, &root)
        .expect("prime root");
    let actual = replay
        .prefixes
        .analyze(ReplayPhase::Late, &different)
        .expect("generation seven");
    assert_eq!(actual.as_ref(), &expected);
    assert_eq!(
        actual.verification_cache().evidence_digest,
        original_evidence_digest(&actual)
    );
}

#[test]
fn contract_prefix_never_skips_full_structural_verification() {
    let root = state("export fn answer(n: i32) -> i32 { return n; }", false);
    let checked = CheckedTuningSpace::enumerate(&root).expect("space");
    let mut replay = checked.search_replay();
    replay
        .prefixes
        .analyze(ReplayPhase::Late, &root)
        .expect("prime root");
    let mut invalid = root.as_ref().clone();
    invalid.module_mut().functions[0].blocks[0].terminator = crate::KirTerminator::Jump {
        edge: crate::KirEdge {
            target: BlockId::from_index(999),
            args: Vec::new(),
            memory_args: Vec::new(),
        },
    };
    assert!(
        replay
            .prefixes
            .analyze(ReplayPhase::Late, &Rc::new(invalid))
            .is_err()
    );
}

#[test]
fn contract_prefix_creation_is_lazy_for_a_fresh_search() {
    let root = state(
        include_str!("../../benches/fixtures/pgo/call_constant_length.ck"),
        true,
    );
    let checked = CheckedTuningSpace::enumerate(&root).expect("space");
    CONTRACT_ENCODINGS.with(|count| count.set(0));
    let _replay = checked.search_replay();
    assert_eq!(CONTRACT_ENCODINGS.with(|count| count.get()), 0);
}

#[test]
fn contract_prefix_distinguishes_missing_from_present_empty_contracts() {
    let source = "export fn answer(n: i32) -> i32 { return n; }";
    let root = state(source, false);
    let different = state(source, true);
    assert!(root.contract_facts().is_none());
    assert!(different.contract_facts().is_some());
    let expected = advance_to_tunable_late_o3(&different).expect("uncached Some");
    let checked = CheckedTuningSpace::enumerate(&root).expect("space");
    let mut replay = checked.search_replay();
    replay
        .prefixes
        .analyze(ReplayPhase::Late, &root)
        .expect("prime None");
    let actual = replay
        .prefixes
        .analyze(ReplayPhase::Late, &different)
        .expect("Some");
    assert_eq!(actual.as_ref(), &expected);
    assert_eq!(
        actual.verification_cache().evidence_digest,
        original_evidence_digest(&actual)
    );
}

#[test]
fn contract_prefix_never_skips_invalid_proof_evidence() {
    let root = state("export fn answer(n: i32) -> i32 { return n; }", false);
    let checked = CheckedTuningSpace::enumerate(&root).expect("space");
    let mut replay = checked.search_replay();
    replay
        .prefixes
        .analyze(ReplayPhase::Late, &root)
        .expect("prime root");
    let mut invalid = root.as_ref().clone();
    let function = invalid.module().functions[0].id;
    let block = invalid.module().functions[0].blocks[0].id;
    invalid
        .proofs_mut()
        .try_insert(
            crate::FactUseSite {
                function,
                block,
                instruction: None,
                contract_instance: None,
            },
            vec![crate::ProofStep::TypeBounds {
                claim: crate::ScalarClaim::new(
                    crate::ValueId::from_index(999),
                    crate::ScalarInterval::new(i32::MIN.into(), i32::MAX.into()).unwrap(),
                    crate::ScalarFailure::None,
                ),
            }],
            crate::ProofStepId::from_index(0),
        )
        .expect("well-formed arena with an invalid module claim");
    assert!(
        crate::validate_kir_module(invalid.module())
            .errors
            .is_empty()
    );
    assert!(
        replay
            .prefixes
            .analyze(ReplayPhase::Finish, &Rc::new(invalid))
            .is_err()
    );
}

#[cfg(feature = "native-toolchain")]
#[test]
fn contract_prefix_counts_complete_native_standard_search() {
    use super::super::transaction::CONTRACT_PREFIX_USES;

    let target = crate::NativeTarget::host_with_cpu(crate::NativeCpu::Native).expect("host");
    let profile = target
        .kir_profile(KirConsumer::NativeLibrary)
        .expect("profile");
    for (name, source) in [
        (
            "branch",
            include_str!("../../benches/fixtures/pgo/branch_layout.ck"),
        ),
        (
            "call",
            include_str!("../../benches/fixtures/pgo/call_constant_length.ck"),
        ),
    ] {
        let checked = check(&SourceFile::new("prefix-coverage.ck", source));
        assert!(checked.diagnostics.is_empty());
        let mir = lower_to_mir(&checked.checked_program).expect("MIR");
        let module = crate::build_kir_module_with_profile(
            &mir,
            KirBuildConfig {
                consumer: KirConsumer::NativeLibrary,
                overflow_mode: KirOverflowMode::Unchecked,
                bounds_mode: KirBoundsMode::Unchecked,
                sanitizer_mode: KirSanitizerMode::Disabled,
            },
            profile.clone(),
        )
        .expect("native KIR");
        let facts = import_contract_facts(&module, &checked.checked_program, 0).expect("facts");
        let root = prepare_kir_pre_tune_state(module, Some(&facts)).expect("pre-tune");
        let checked = CheckedTuningSpace::enumerate(&root).expect("space");
        CONTRACT_ENCODINGS.with(|count| count.set(0));
        CONTRACT_PREFIX_USES.with(|count| count.set(0));
        tests::CONTRACT_PREFIX_PHASES.with(|counts| counts.borrow_mut().clear());
        let frontier = crate::run_checked_tuning_search(&checked, crate::TuneBudget::Standard)
            .expect("complete standard search");
        let encodings = CONTRACT_ENCODINGS.with(std::cell::Cell::get);
        let uses = CONTRACT_PREFIX_USES.with(std::cell::Cell::get);
        let phases = tests::CONTRACT_PREFIX_PHASES.with(|counts| counts.borrow().clone());
        assert!(!frontier.expansions.is_empty());
        assert!(encodings > 0 && uses > 0);
        assert_eq!(
            uses,
            phases
                .iter()
                .filter(|((_, prefix), _)| *prefix)
                .map(|(_, count)| count)
                .sum(),
            "every offered root prefix is rechecked and actually used"
        );
        eprintln!(
            "case={name} expansions={} contract_encodings={encodings} prefix_uses={uses} phases={phases:?}",
            frontier.expansions.len()
        );
    }
}
