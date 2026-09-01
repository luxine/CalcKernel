use calckernel::{
    BlockId, CandidateBudgetCharge, CandidateDisposition, CandidateKey, FactUseSite, FunctionId,
    InstructionId, KirBoundsMode, KirBuildConfig, KirConsumer, KirOptimizationAuditState,
    KirOverflowMode, KirSanitizerMode, KirVerifiedProgramState, LoopCandidateKind,
    LoopCandidateVariant, LoopId, ProofStep, ProofStepId, SourceFile, TransactionCheckError,
    TransactionOutcome, build_kir_module, check, execute_verified_transaction, lower_to_mir,
    order_candidate_keys, print_optimization_audit,
};

fn state(source: &str) -> KirVerifiedProgramState {
    let checked = check(&SourceFile::new("transaction.ck", source));
    assert_eq!(checked.diagnostics, []);
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
    KirVerifiedProgramState::new(module, None, 0).expect("verified state")
}

fn loop_key(function: FunctionId, ordinal: u32) -> CandidateKey {
    CandidateKey::LoopFrontier {
        function,
        loop_id: LoopId::from_index(ordinal),
        kind: LoopCandidateKind::LoopSimd,
        variant: LoopCandidateVariant::Scalar,
        vf: 4,
        uf: 1,
    }
}

#[test]
fn transaction_rejection_should_restore_all_program_state_but_keep_audit_debits() {
    let mut state = state("export fn answer(n: i32) -> i32 { return n; }");
    let before = state.clone();
    let function = state.module().functions[0].id;
    let mut audit = KirOptimizationAuditState::for_module(state.module());
    let initial = audit.ledger().budget(function).expect("budget");
    let outcome = execute_verified_transaction(
        &mut state,
        &mut audit,
        loop_key(function, 0),
        CandidateBudgetCharge::single(function, 3, 5),
        |trial| {
            trial.module_mut().functions[0].name = "trial-only".to_string();
            let use_block = trial.module().functions[0].blocks[0].id;
            let parameter = trial.module().functions[0].params[0].value;
            trial
                .proofs_mut()
                .try_insert(
                    FactUseSite {
                        function,
                        block: use_block,
                        instruction: None,
                        contract_instance: None,
                    },
                    vec![ProofStep::TypeBounds {
                        claim: calckernel::ScalarClaim::new(
                            parameter,
                            calckernel::ScalarInterval::new(
                                (-2147483648_i64).into(),
                                2147483647_i64.into(),
                            )
                            .unwrap(),
                            calckernel::ScalarFailure::None,
                        ),
                    }],
                    ProofStepId::from_index(0),
                )
                .map_err(|error| error.to_string())?;
            Ok(())
        },
        |_pre, _trial| Err(TransactionCheckError::reject("not-profitable")),
    );

    assert_eq!(outcome, TransactionOutcome::Rejected);
    assert_eq!(state, before, "program state must roll back byte-for-byte");
    let remaining = audit.ledger().budget(function).expect("remaining budget");
    assert_eq!(remaining.proposer_remaining, initial.proposer_remaining - 3);
    assert_eq!(remaining.checker_remaining, initial.checker_remaining - 5);
    assert_eq!(audit.attempts().len(), 1);
    assert_eq!(
        audit.attempts()[0].disposition,
        CandidateDisposition::Rejected
    );
    assert_eq!(audit.attempts()[0].reason, "not-profitable");
}

#[test]
fn transaction_acceptance_should_swap_the_complete_verified_state_once() {
    let mut state = state("export fn answer() -> i32 { return 42; }");
    let before = state.clone();
    let function = state.module().functions[0].id;
    let key = loop_key(function, 0);
    let mut audit = KirOptimizationAuditState::for_module(state.module());
    let outcome = execute_verified_transaction(
        &mut state,
        &mut audit,
        key.clone(),
        CandidateBudgetCharge::single(function, 1, 1),
        |trial| {
            trial.module_mut().functions[0].name = "accepted".to_string();
            Ok(())
        },
        |pre, trial| {
            (pre.module().functions[0].name == "answer"
                && trial.module().functions[0].name == "accepted")
                .then_some(())
                .ok_or_else(|| TransactionCheckError::compiler("wrong transaction snapshots"))
        },
    );
    assert_eq!(outcome, TransactionOutcome::Committed);
    assert_ne!(state, before);
    assert_eq!(state.module().functions[0].name, "accepted");
    assert_eq!(audit.attempts()[0].key, key);
    assert_eq!(
        audit.attempts()[0].disposition,
        CandidateDisposition::Accepted
    );
}

#[test]
fn transaction_post_verifier_failure_should_be_a_compiler_error_without_scalar_artifact() {
    let mut state = state("export fn answer() -> i32 { return 42; }");
    let before = state.clone();
    let function = state.module().functions[0].id;
    let mut audit = KirOptimizationAuditState::for_module(state.module());
    let outcome = execute_verified_transaction(
        &mut state,
        &mut audit,
        loop_key(function, 0),
        CandidateBudgetCharge::single(function, 1, 1),
        |trial| {
            trial.module_mut().functions[0].blocks[0].terminator =
                calckernel::KirTerminator::Jump {
                    edge: calckernel::KirEdge {
                        target: BlockId::from_index(999),
                        args: Vec::new(),
                        memory_args: Vec::new(),
                    },
                };
            Ok(())
        },
        |_pre, _trial| Ok(()),
    );
    assert!(matches!(outcome, TransactionOutcome::CompilerError(_)));
    assert_eq!(state, before);
    assert_eq!(
        audit.attempts()[0].disposition,
        CandidateDisposition::CompilerError
    );
}

#[test]
fn transaction_forged_proof_should_be_a_compiler_error_and_roll_back() {
    let mut state = state("export fn answer(n: i32) -> i32 { return n; }");
    let before = state.clone();
    let function = state.module().functions[0].id;
    let mut audit = KirOptimizationAuditState::for_module(state.module());
    let outcome = execute_verified_transaction(
        &mut state,
        &mut audit,
        loop_key(function, 0),
        CandidateBudgetCharge::single(function, 1, 1),
        |trial| {
            let block = trial.module().functions[0].blocks[0].id;
            let value = trial.module().functions[0].params[0].value;
            trial
                .proofs_mut()
                .try_insert(
                    FactUseSite {
                        function,
                        block,
                        instruction: None,
                        contract_instance: None,
                    },
                    vec![ProofStep::TypeBounds {
                        claim: calckernel::ScalarClaim::new(
                            value,
                            calckernel::ScalarInterval::new(0.into(), 0.into()).unwrap(),
                            calckernel::ScalarFailure::None,
                        ),
                    }],
                    ProofStepId::from_index(0),
                )
                .map(|_| ())
                .map_err(|error| error.to_string())
        },
        |_pre, _trial| Ok(()),
    );
    assert!(matches!(outcome, TransactionOutcome::CompilerError(_)));
    assert_eq!(state, before);
    assert_eq!(
        audit.attempts()[0].disposition,
        CandidateDisposition::CompilerError
    );
}

#[test]
fn audit_ledger_should_use_fixed_saturating_budgets_and_atomic_multi_function_debits() {
    let state = state(
        "fn helper(n: u32) -> u32 { return n + 1; } export fn caller(n: u32) -> u32 { return helper(n); }",
    );
    let caller = state.module().functions[1].id;
    let callee = state.module().functions[0].id;
    let mut audit = KirOptimizationAuditState::for_module(state.module());
    for function in [caller, callee] {
        let units = calckernel::kir_function_units(
            state
                .module()
                .functions
                .iter()
                .find(|item| item.id == function)
                .unwrap(),
        );
        let budget = audit.ledger().budget(function).unwrap();
        assert_eq!(
            budget.proposer_initial,
            units.saturating_mul(64).saturating_add(128)
        );
        assert_eq!(
            budget.checker_initial,
            units.saturating_mul(96).saturating_add(256)
        );
    }

    let before = audit.ledger().clone();
    let impossible = CandidateBudgetCharge {
        functions: vec![callee, caller],
        proposer_units: u32::MAX,
        checker_units: u32::MAX,
    };
    assert!(!audit.ledger_mut().try_debit(&impossible));
    assert_eq!(audit.ledger(), &before, "exhaustion must be atomic");

    let charge = CandidateBudgetCharge {
        functions: vec![callee, caller],
        proposer_units: 2,
        checker_units: 3,
    };
    assert!(audit.ledger_mut().try_debit(&charge));
    for function in [caller, callee] {
        let current = audit.ledger().budget(function).unwrap();
        let original = before.budget(function).unwrap();
        assert_eq!(current.proposer_remaining, original.proposer_remaining - 2);
        assert_eq!(current.checker_remaining, original.checker_remaining - 3);
    }
}

#[test]
fn candidate_order_should_be_total_deterministic_and_reject_duplicate_keys() {
    let f0 = FunctionId::from_index(0);
    let f1 = FunctionId::from_index(1);
    let keys = vec![
        CandidateKey::ResidualSlp {
            function: f0,
            block: BlockId::from_index(4),
            root: InstructionId::from_index(8),
            lanes: 4,
        },
        CandidateKey::LoopFrontier {
            function: f0,
            loop_id: LoopId::from_index(1),
            kind: LoopCandidateKind::PartialUnroll,
            variant: LoopCandidateVariant::Slp,
            vf: 1,
            uf: 4,
        },
        CandidateKey::Specialization {
            caller: f1,
            call: InstructionId::from_index(2),
            callee: f0,
            fact_set_digest: "11".repeat(32),
        },
        loop_key(f0, 0),
    ];
    let ordered = order_candidate_keys(keys.iter().rev().cloned()).expect("unique order");
    assert!(matches!(ordered[0], CandidateKey::Specialization { .. }));
    assert!(matches!(ordered[1], CandidateKey::LoopFrontier { .. }));
    assert!(matches!(
        ordered.last(),
        Some(CandidateKey::ResidualSlp { .. })
    ));
    let mut duplicate = keys;
    duplicate.push(loop_key(f0, 0));
    assert!(
        order_candidate_keys(duplicate)
            .unwrap_err()
            .contains("duplicate")
    );
}

#[test]
fn audit_ledger_rejected_reused_and_nonwinner_attempts_should_never_refund_budget() {
    let state = state("export fn answer() -> i32 { return 42; }");
    let function = state.module().functions[0].id;
    let mut audit = KirOptimizationAuditState::for_module(state.module());
    let initial = audit.ledger().budget(function).unwrap();
    for (ordinal, disposition) in [
        CandidateDisposition::Rejected,
        CandidateDisposition::Reused,
        CandidateDisposition::NonWinner,
    ]
    .into_iter()
    .enumerate()
    {
        audit
            .record_noncommitting_attempt(
                loop_key(function, ordinal as u32),
                CandidateBudgetCharge::single(function, 1, 1),
                disposition,
                disposition.stable_name(),
            )
            .expect("audit attempt");
    }
    let remaining = audit.ledger().budget(function).unwrap();
    assert_eq!(remaining.proposer_remaining, initial.proposer_remaining - 3);
    assert_eq!(remaining.checker_remaining, initial.checker_remaining - 3);
    assert_eq!(audit.attempts().len(), 3);
    let text = print_optimization_audit(&audit);
    assert_eq!(text, print_optimization_audit(&audit));
}

#[test]
fn verifier_cache_should_roll_back_on_rejection_and_refresh_only_after_commit() {
    let mut state = state("export fn answer() -> i32 { return 42; }");
    let function = state.module().functions[0].id;
    let original_cache = state.verification_cache().clone();
    let mut audit = KirOptimizationAuditState::for_module(state.module());
    let rejected = execute_verified_transaction(
        &mut state,
        &mut audit,
        loop_key(function, 0),
        CandidateBudgetCharge::single(function, 1, 1),
        |trial| {
            trial.module_mut().functions[0].name = "rejected".to_string();
            Ok(())
        },
        |_pre, _trial| Err(TransactionCheckError::reject("test-rejection")),
    );
    assert_eq!(rejected, TransactionOutcome::Rejected);
    assert_eq!(state.verification_cache(), &original_cache);

    let committed = execute_verified_transaction(
        &mut state,
        &mut audit,
        loop_key(function, 1),
        CandidateBudgetCharge::single(function, 1, 1),
        |trial| {
            trial.module_mut().functions[0].name = "committed".to_string();
            Ok(())
        },
        |_pre, _trial| Ok(()),
    );
    assert_eq!(committed, TransactionOutcome::Committed);
    assert_ne!(state.verification_cache(), &original_cache);
    assert_eq!(state.verification_cache().kir_digest, state.kir_digest());
}
