use calckernel::{
    InstructionId, KirBoundsMode, KirBuildConfig, KirConsumer, KirInstruction, KirInstructionKind,
    KirLegalCost, KirNativeCpuPolicy, KirOptimizationLevel, KirOverflowMode, KirResult,
    KirSanitizerMode, KirTargetProfile, KirTargetProfileBuilder, KirVerifiedProgramState,
    SourceFile, TuneAlternativeClass, TuneAlternativePayload, TuneBudget, TuneVariantAction,
    TuningPlan, ValueId, apply_tuning_plan, build_kir_module, build_kir_module_with_profile, check,
    check_tuning_plan, enumerate_tuning_space, import_contract_facts, lower_to_mir,
    prepare_kir_pre_tune_state, print_kir_module, run_deterministic_search, run_kir_pass_pipeline,
};

const TUNABLE_SOURCE: &str = "export fn kernel() -> u32 { let i: u32 = 0; let total: u32 = 0; while i < 12 { total = total + i; i = i + 1; } return total; }";
const INLINE_SOURCE: &str = "fn add_one(value: u32) -> u32 { return value + 1; } export fn kernel(value: u32) -> u32 { return add_one(value); }";
const TWO_INLINE_SOURCE: &str = "fn add_one(value: u32) -> u32 { return value + 1; } fn times_two(value: u32) -> u32 { return value * 2; } export fn kernel(value: u32) -> u32 { return add_one(value) + times_two(value); }";
const VECTOR_SOURCE: &str = include_str!("../../benches/oracles/fixtures/map_u32.ck");
const LAYOUT_SOURCE: &str = include_str!("../../benches/fixtures/pgo/branch_layout.ck");
const SPECIALIZATION_SOURCE: &str =
    include_str!("../../benches/fixtures/pgo/call_constant_length.ck");
const SLP_SOURCE: &str = r#"
export fn lanes(a0: i32, a1: i32, a2: i32, a3: i32, b0: i32, b1: i32, b2: i32, b3: i32) -> i32 {
  let p0: i32 = a0 * b0;
  let p1: i32 = a1 * b1;
  let p2: i32 = a2 * b2;
  let p3: i32 = a3 * b3;
  return p0 + p1 + p2 + p3;
}
"#;
const PREDICATED_UPDATE_FLOYD: &str = r#"
export unsafe fn floyd(distance: slice<f64>, n: u32) -> void
contract {
  requires n <= 65535;
  effects readwrite(distance);
}
{
  let k: u32 = 0;
  while k < n {
    let k_row: u32 = k * n;
    let i: u32 = 0;
    while i < n {
      let i_row: u32 = i * n;
      let dik: f64 = distance[i_row + k];
      let j: u32 = 0;
      while j < n {
        let index: u32 = i_row + j;
        let candidate: f64 = dik + distance[k_row + j];
        let old: f64 = distance[index];
        if candidate < old {
          distance[index] = candidate;
        }
        j = j + 1;
      }
      i = i + 1;
    }
    k = k + 1;
  }
}
"#;

#[test]
fn predicated_tuning_should_keep_distinct_floyd_variants() {
    let state = vector_state(PREDICATED_UPDATE_FLOYD);
    let space = enumerate_tuning_space(&state).expect("Floyd tuning space");
    let (unit_index, unit) = space
        .units
        .iter()
        .enumerate()
        .find(|(_, unit)| {
            unit.variants.iter().any(|variant| {
                matches!(
                    &variant.action,
                    TuneVariantAction::LoopSimd(candidate)
                        if candidate.predicated_update.is_some()
                )
            })
        })
        .expect("predicated Floyd tuning unit");
    assert!(unit.variants.len() >= 2, "{unit:#?}");

    let variant_ids = unit
        .variants
        .iter()
        .map(|variant| variant.variant_id)
        .collect::<BTreeSet<_>>();
    let payloads = unit
        .variants
        .iter()
        .map(|variant| match variant.site_alternatives[0].payload {
            TuneAlternativePayload::LoopSimd {
                vector_bits,
                interleave,
                break_even_iterations,
            } => (vector_bits, interleave, break_even_iterations),
            ref other => panic!("unexpected Floyd payload: {other:?}"),
        })
        .collect::<BTreeSet<_>>();
    let post_states = unit
        .variants
        .iter()
        .map(|variant| variant.isolated_post_state_digest)
        .collect::<BTreeSet<_>>();
    assert_eq!(variant_ids.len(), unit.variants.len());
    assert_eq!(payloads.len(), unit.variants.len());
    assert_eq!(post_states.len(), unit.variants.len());
    for variant_index in 0..unit.variants.len() {
        let plan = space
            .plan_for_variant(&state, unit_index, variant_index)
            .expect("derive Floyd plan")
            .expect("Floyd variant plan");
        apply_tuning_plan(&state, &space, &plan).expect("replay Floyd variant");
    }
}

#[test]
fn tuning_site_unit_and_variant_ids_are_stable() {
    let state = state(TUNABLE_SOURCE);
    let left = enumerate_tuning_space(&state).expect("space");
    let right = enumerate_tuning_space(&state).expect("space");

    assert_eq!(left, right);
    assert_ne!(left.digest, [0; 32]);
    assert!(!left.units.is_empty());
    assert!(left.units.len() <= 64);
    assert!(left.units.iter().all(|unit| unit.variants.len() <= 4));
}

#[test]
fn tuning_space_materializes_a_checked_direct_call_inline_alternative() {
    let state = state(INLINE_SOURCE);
    let space = enumerate_tuning_space(&state).expect("space");
    let (unit_index, variant_index) = space
        .units
        .iter()
        .enumerate()
        .find_map(|(unit_index, unit)| {
            unit.variants
                .iter()
                .position(|variant| variant.class == TuneAlternativeClass::Inlining)
                .map(|variant_index| (unit_index, variant_index))
        })
        .expect("direct-call inline unit");
    let plan = space
        .plan_for_variant(&state, unit_index, variant_index)
        .expect("derive inline plan")
        .expect("inline plan");
    let replayed = apply_tuning_plan(&state, &space, &plan).expect("checked inline replay");

    assert_ne!(
        print_kir_module(replayed.module()),
        print_kir_module(state.module())
    );
}

#[test]
fn early_tuning_replay_does_not_reenter_ordinary_tunable_phases() {
    let state = state(TWO_INLINE_SOURCE);
    let space = enumerate_tuning_space(&state).expect("space");
    let (unit_index, variant_index) = space
        .units
        .iter()
        .enumerate()
        .find_map(|(unit_index, unit)| {
            unit.variants
                .iter()
                .position(|variant| variant.class == TuneAlternativeClass::Inlining)
                .map(|variant_index| (unit_index, variant_index))
        })
        .expect("one direct-call inline unit");
    let plan = space
        .plan_for_variant(&state, unit_index, variant_index)
        .expect("derive exact inline plan")
        .expect("inline plan");
    let replayed = apply_tuning_plan(&state, &space, &plan).expect("exact replay");
    let remaining_calls = replayed
        .module()
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.instructions)
        .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Call { .. }))
        .count();

    assert_eq!(
        remaining_calls, 1,
        "the plan selected exactly one of two inline sites"
    );
}

#[test]
fn tuning_space_materializes_a_checked_short_slice_versioning_alternative() {
    let state = vector_state(VECTOR_SOURCE);
    let space = enumerate_tuning_space(&state).expect("space");
    let (unit_index, variant_index) = space
        .units
        .iter()
        .enumerate()
        .find_map(|(unit_index, unit)| {
            unit.variants
                .iter()
                .position(|variant| variant.class == TuneAlternativeClass::ShortSliceVersioning)
                .map(|variant_index| (unit_index, variant_index))
        })
        .expect("short-slice/versioning unit");
    let plan = space
        .plan_for_variant(&state, unit_index, variant_index)
        .expect("derive short-slice plan")
        .expect("short-slice plan");
    let replayed = apply_tuning_plan(&state, &space, &plan).expect("checked short-slice replay");

    assert_ne!(
        print_kir_module(replayed.module()),
        print_kir_module(state.module())
    );
}

#[test]
fn loop_tuning_replay_runs_the_fixed_dead_code_suffix() {
    let base = vector_state(VECTOR_SOURCE);
    let mut module = base.module().clone();
    let template = module
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.instructions)
        .find(|instruction| {
            matches!(instruction.kind, KirInstructionKind::ConstInt { .. })
                && instruction.results.len() == 1
        })
        .expect("integer constant template")
        .clone();
    let instruction_id = InstructionId::from_index(
        module
            .functions
            .iter()
            .flat_map(|function| &function.blocks)
            .flat_map(|block| &block.instructions)
            .map(|instruction| instruction.id.index())
            .max()
            .expect("instruction ids")
            + 1,
    );
    let value_id = ValueId::from_index(
        module
            .functions
            .iter()
            .flat_map(|function| &function.blocks)
            .flat_map(|block| &block.instructions)
            .flat_map(|instruction| &instruction.results)
            .map(|result| result.value.index())
            .max()
            .expect("value ids")
            + 1,
    );
    module.functions[0].blocks[0]
        .instructions
        .push(KirInstruction {
            id: instruction_id,
            results: vec![KirResult {
                value: value_id,
                type_node: template.results[0].type_node.clone(),
            }],
            kind: KirInstructionKind::ConstInt {
                value: "424242".to_string(),
            },
            memory: None,
            effect: None,
        });
    let state = KirVerifiedProgramState::from_parts(
        module,
        base.contract_facts().cloned(),
        base.proofs().clone(),
        base.eliminated_guards().to_vec(),
        base.evidence_generation(),
    )
    .expect("verified state with a deliberately dead pure instruction");
    let space = enumerate_tuning_space(&state).expect("space");
    let (unit_index, variant_index) = space
        .units
        .iter()
        .enumerate()
        .find_map(|(unit_index, unit)| {
            unit.variants
                .iter()
                .position(|variant| variant.class == TuneAlternativeClass::LoopSimd)
                .map(|variant_index| (unit_index, variant_index))
        })
        .expect("Loop SIMD unit");
    let plan = space
        .plan_for_variant(&state, unit_index, variant_index)
        .expect("derive Loop SIMD plan")
        .expect("Loop SIMD plan");
    let replayed = apply_tuning_plan(&state, &space, &plan).expect("checked Loop SIMD replay");

    assert!(replayed.module().functions.iter().all(|function| {
        function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .all(|instruction| instruction.id != instruction_id)
    }));
}

#[test]
fn loop_tuning_bypasses_only_the_ordinary_static_profitability_threshold() {
    let state = vector_state_with_costs(VECTOR_SOURCE, 1, 100);
    assert!(
        calckernel::discover_vectorization_candidates(&state)
            .candidates
            .is_empty(),
        "the ordinary O3 proposer must retain its 20% static-profitability threshold",
    );
    let space = enumerate_tuning_space(&state).expect("tuning space");
    let (unit_index, variant_index) = space
        .units
        .iter()
        .enumerate()
        .find_map(|(unit_index, unit)| {
            unit.variants
                .iter()
                .position(|variant| variant.class == TuneAlternativeClass::LoopSimd)
                .map(|variant_index| (unit_index, variant_index))
        })
        .expect("measurement-owned Loop SIMD alternative");
    let plan = space
        .plan_for_variant(&state, unit_index, variant_index)
        .expect("derive measurement-owned plan")
        .expect("Loop SIMD plan");

    apply_tuning_plan(&state, &space, &plan)
        .expect("legality, proof, transaction, and growth checks still accept the trial");
}

#[test]
fn tuning_space_materializes_checked_canonical_layout_metadata() {
    let state = state(LAYOUT_SOURCE);
    let space = enumerate_tuning_space(&state).expect("space");
    let (unit_index, variant_index) = space
        .units
        .iter()
        .enumerate()
        .find_map(|(unit_index, unit)| {
            unit.variants
                .iter()
                .position(|variant| variant.class == TuneAlternativeClass::Layout)
                .map(|variant_index| (unit_index, variant_index))
        })
        .expect("layout unit");
    let plan = space
        .plan_for_variant(&state, unit_index, variant_index)
        .expect("derive layout plan")
        .expect("layout plan");
    let replayed = apply_tuning_plan(&state, &space, &plan).expect("checked layout replay");

    assert!(replayed.module().tune_layout.is_some());
    assert!(print_kir_module(replayed.module()).contains("tune-layout"));
    assert!(state.module().tune_layout.is_none());
}

#[test]
fn layout_only_replay_preserves_the_fixed_ordinary_o3_suffix() {
    let state = state(LAYOUT_SOURCE);
    let space = enumerate_tuning_space(&state).expect("space");
    let (unit_index, variant_index) = space
        .units
        .iter()
        .enumerate()
        .find_map(|(unit_index, unit)| {
            unit.variants
                .iter()
                .position(|variant| variant.class == TuneAlternativeClass::Layout)
                .map(|variant_index| (unit_index, variant_index))
        })
        .expect("layout unit");
    let plan = space
        .plan_for_variant(&state, unit_index, variant_index)
        .expect("derive layout plan")
        .expect("layout plan");
    let baseline = apply_tuning_plan(&state, &space, &TuningPlan::baseline())
        .expect("ordinary O3 baseline replay");
    let replayed = apply_tuning_plan(&state, &space, &plan).expect("checked layout replay");
    let mut without_layout = replayed.module().clone();
    without_layout.tune_layout = None;

    assert_eq!(
        print_kir_module(&without_layout),
        print_kir_module(baseline.module()),
        "layout metadata must not suppress the fixed ordinary O3 suffix",
    );
}

#[test]
fn tuning_corpus_materializes_all_seven_closed_alternative_classes() {
    let states = [
        state(TUNABLE_SOURCE),
        state(INLINE_SOURCE),
        state(LAYOUT_SOURCE),
        vector_state(VECTOR_SOURCE),
        vector_state(SPECIALIZATION_SOURCE),
        vector_state(SLP_SOURCE),
    ];
    assert!(
        states.iter().any(|state| {
            !calckernel::discover_specialization_candidates(state.module(), state.contract_facts())
                .candidates
                .is_empty()
        }),
        "the frozen tuning corpus must expose a specialization proposal"
    );
    let actual = states
        .iter()
        .flat_map(|state| enumerate_tuning_space(state).expect("space").units)
        .flat_map(|unit| unit.variants)
        .map(|variant| variant.class)
        .collect::<BTreeSet<_>>();
    let expected = BTreeSet::from([
        TuneAlternativeClass::Inlining,
        TuneAlternativeClass::Specialization,
        TuneAlternativeClass::Unrolling,
        TuneAlternativeClass::LoopSimd,
        TuneAlternativeClass::Slp,
        TuneAlternativeClass::ShortSliceVersioning,
        TuneAlternativeClass::Layout,
    ]);

    assert_eq!(actual, expected);
}

#[test]
fn tuning_space_site_and_alternative_state_identities_are_wire_consistent() {
    for state in [state(INLINE_SOURCE), vector_state(VECTOR_SOURCE)] {
        let space = enumerate_tuning_space(&state).expect("space");
        for unit in &space.units {
            for variant in &unit.variants {
                for alternative in &variant.site_alternatives {
                    let site = space
                        .sites
                        .iter()
                        .find(|site| site.site_id == alternative.site_id)
                        .expect("alternative site");
                    assert_eq!(site.pre_state_digest, space.pre_state_digest);
                    assert_eq!(alternative.pre_state_digest, site.pre_state_digest);
                    assert_eq!(alternative.pre_state_digest, space.pre_state_digest);
                    assert_eq!(
                        alternative.post_state_digest,
                        variant.isolated_post_state_digest
                    );
                }
            }
        }
    }
}

#[test]
fn overlapping_loop_alternatives_are_recorded_without_aborting_the_search() {
    let state = vector_state(VECTOR_SOURCE);
    let space = enumerate_tuning_space(&state).expect("space");
    let frontier = run_deterministic_search(&state, &space, TuneBudget::Quick)
        .expect("an incompatible whole-plan expansion must not abort the search");
    assert!(!frontier.expansions.is_empty());
    assert!(frontier.expansions.iter().any(|expansion| {
        expansion.disposition == calckernel::ExpansionDisposition::Illegal
            && expansion.result_plan_digest.is_none()
            && expansion.diagnostic_code != 0
    }));
}

fn vector_state(source: &str) -> KirVerifiedProgramState {
    vector_state_with_costs(source, 20, 1)
}

fn vector_state_with_costs(
    source: &str,
    scalar_cost: u32,
    vector_cost: u32,
) -> KirVerifiedProgramState {
    let checked = check(&SourceFile::new("tuning-vector.ck", source));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let mut builder = KirTargetProfileBuilder::native(
        KirConsumer::NativeLibrary,
        "x86_64-unknown-linux-gnu",
        64,
        true,
        KirNativeCpuPolicy::Native,
        "x86-64-v4",
        vec!["+avx2".to_string()],
    )
    .expect("native tuning profile");
    for key in KirTargetProfile::fixed_query_universe()
        .into_iter()
        .filter(|key| {
            ((key.lanes == 2 || key.lanes == 4)
                && (key.lane == calckernel::KirLaneType::U32
                    || key.lane == calckernel::KirLaneType::F64)
                && matches!(
                    key.operation,
                    calckernel::KirProfileOperation::Splat
                        | calckernel::KirProfileOperation::Add
                        | calckernel::KirProfileOperation::Subtract
                        | calckernel::KirProfileOperation::Multiply
                        | calckernel::KirProfileOperation::Load
                        | calckernel::KirProfileOperation::Store
                        | calckernel::KirProfileOperation::Compare
                        | calckernel::KirProfileOperation::Select
                        | calckernel::KirProfileOperation::Cast
                        | calckernel::KirProfileOperation::Insert
                        | calckernel::KirProfileOperation::Extract
                        | calckernel::KirProfileOperation::RuntimePredicate
                ))
                || (key.lanes == 4
                    && key.lane == calckernel::KirLaneType::I32
                    && matches!(
                        key.operation,
                        calckernel::KirProfileOperation::Splat
                            | calckernel::KirProfileOperation::Insert
                            | calckernel::KirProfileOperation::Multiply
                            | calckernel::KirProfileOperation::Extract
                    ))
        })
    {
        let legalized_type = format!("test-{:?}-{}", key.lane, key.lanes);
        builder
            .set_legal(
                key,
                KirLegalCost {
                    cost: vector_cost,
                    legalization_parts: 1,
                    legalized_type,
                },
            )
            .expect("legal tuning query");
    }
    for key in KirTargetProfile::fixed_query_universe()
        .into_iter()
        .filter(|key| {
            key.lanes == 1
                && matches!(
                    (key.lane, key.operation),
                    (
                        calckernel::KirLaneType::U32,
                        calckernel::KirProfileOperation::Add
                    ) | (
                        calckernel::KirLaneType::F64,
                        calckernel::KirProfileOperation::Add
                    ) | (
                        calckernel::KirLaneType::I32,
                        calckernel::KirProfileOperation::Multiply
                    )
                )
        })
    {
        let legalized_type = if key.lane == calckernel::KirLaneType::F64 {
            "double".to_string()
        } else {
            "i32".to_string()
        };
        builder
            .set_legal(
                key,
                KirLegalCost {
                    cost: scalar_cost,
                    legalization_parts: 1,
                    legalized_type,
                },
            )
            .expect("legal scalar tuning query");
    }
    builder.set_maximum_interleave_factor(4);
    let module = build_kir_module_with_profile(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::NativeLibrary,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        builder.build().expect("tuning profile"),
    )
    .expect("KIR");
    let contracts = import_contract_facts(&module, &checked.checked_program, 0)
        .expect("contract facts for tuning");
    prepare_kir_pre_tune_state(module, Some(&contracts)).expect("verified vector pre-tune")
}

#[test]
fn tuning_plan_checker_rejects_forged_variant_and_preserves_prestate() {
    let state = state(TUNABLE_SOURCE);
    let space = enumerate_tuning_space(&state).expect("space");
    let plan = space
        .plan_for_variant(&state, 0, 0)
        .expect("derive plan")
        .expect("plan");
    check_tuning_plan(&state, &space, &plan).expect("checked plan");
    let replayed = apply_tuning_plan(&state, &space, &plan).expect("replay");
    assert_ne!(replayed.kir_digest(), state.kir_digest());

    let mut forged = plan;
    forged.choices[0].variant_id[0] ^= 1;
    assert!(check_tuning_plan(&state, &space, &forged).is_err());
}

#[test]
fn tuning_plan_checker_recomputes_the_candidate_space_from_the_prestate() {
    let state = state(TUNABLE_SOURCE);
    let mut space = enumerate_tuning_space(&state).expect("space");
    let plan = space
        .plan_for_variant(&state, 0, 0)
        .expect("derive plan")
        .expect("plan");
    space.digest[0] ^= 1;

    assert!(check_tuning_plan(&state, &space, &plan).is_err());
}

#[test]
fn tuning_empty_plan_preserves_canonical_kir_bytes() {
    let raw = raw_module(TUNABLE_SOURCE);
    let state = prepare_kir_pre_tune_state(raw.clone(), None).expect("pre-tune");
    let space = enumerate_tuning_space(&state).expect("space");
    let replayed = apply_tuning_plan(&state, &space, &calckernel::TuningPlan::baseline())
        .expect("empty replay");
    let ordinary = run_kir_pass_pipeline(raw, KirOptimizationLevel::O3, None);
    assert!(ordinary.errors.is_empty(), "{:?}", ordinary.errors);

    assert_eq!(
        print_kir_module(replayed.module()),
        print_kir_module(ordinary.artifact.as_ref().expect("ordinary O3 artifact"))
    );
}

fn state(source: &str) -> KirVerifiedProgramState {
    prepare_kir_pre_tune_state(raw_module(source), None).expect("verified pre-tune")
}

fn raw_module(source: &str) -> calckernel::KirModule {
    let checked = check(&SourceFile::new("tuning.ck", source));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::Inspection,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .expect("KIR")
}
use std::collections::BTreeSet;
