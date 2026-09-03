use calckernel::{
    KirAlignmentClass, KirBoundsMode, KirBuildConfig, KirConsumer, KirLegalCost,
    KirNativeCpuPolicy, KirOverflowMode, KirProfileOperation, KirSanitizerMode, KirTargetProfile,
    KirTargetProfileBuilder, SourceFile, TuneAlternativeClass, TuneBudget, TuneVariantAction,
    TuningPlan, attest_selected_predicated_update, build_kir_module_with_profile, check,
    enumerate_tuning_space, import_contract_facts, lower_to_mir, prepare_kir_pre_tune_state,
    run_deterministic_search,
};

const FLOYD: &str = r#"
export unsafe fn floyd(distance: slice<f64>, n: u32) -> void
contract { requires n <= 65535; effects readwrite(distance); }
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
        if candidate < old { distance[index] = candidate; }
        j = j + 1;
      }
      i = i + 1;
    }
    k = k + 1;
  }
}
"#;

#[test]
fn predicated_attestation_should_require_exact_single_choice() {
    let state = floyd_state(FLOYD);
    let space = enumerate_tuning_space(&state).expect("Floyd space");
    let (unit_index, variant_index) = target_variant(&space);
    let plan = space
        .plan_for_variant(&state, unit_index, variant_index)
        .expect("derive target plan")
        .expect("target plan");
    let attestation = attest_selected_predicated_update(&state, &space, &plan)
        .expect("source-aware Floyd attestation");
    let candidate = match &space.units[unit_index].variants[variant_index].action {
        TuneVariantAction::LoopSimd(candidate) => candidate,
        _ => unreachable!("target helper returned a non-vector variant"),
    };
    let update = candidate
        .predicated_update
        .as_ref()
        .expect("predicated shape");
    assert_eq!(attestation.function, "floyd");
    assert_eq!(attestation.header, candidate.header);
    assert_eq!(attestation.compare, update.condition_instruction);
    assert_eq!(attestation.load, update.old_load_instruction);
    assert_eq!(attestation.store, update.store_instruction);
    assert!(attestation.minimum <= 128);

    assert!(attest_selected_predicated_update(&state, &space, &TuningPlan::baseline()).is_err());
    for mutate in [
        |plan: &mut TuningPlan| plan.choices[0].unit_id[0] ^= 1,
        |plan: &mut TuningPlan| plan.choices[0].variant_id[0] ^= 1,
        |plan: &mut TuningPlan| plan.choices[0].pre_state_digest[0] ^= 1,
        |plan: &mut TuningPlan| plan.choices[0].post_state_digest[0] ^= 1,
    ] {
        let mut forged = plan.clone();
        mutate(&mut forged);
        assert!(attest_selected_predicated_update(&state, &space, &forged).is_err());
    }

    let layout = space
        .units
        .iter()
        .enumerate()
        .find_map(|(unit, candidate)| {
            candidate
                .variants
                .iter()
                .position(|variant| variant.class == TuneAlternativeClass::Layout)
                .map(|variant| (unit, variant))
        });
    if let Some((unit, variant)) = layout {
        let layout_plan = space
            .plan_for_variant(&state, unit, variant)
            .expect("derive layout plan")
            .expect("layout plan");
        assert!(attest_selected_predicated_update(&state, &space, &layout_plan).is_err());
    }

    if let Some(compound) = run_deterministic_search(&state, &space, TuneBudget::Quick)
        .expect("bounded search")
        .compile_selection
        .into_iter()
        .find(|candidate| candidate.choices.len() > 1)
    {
        assert!(attest_selected_predicated_update(&state, &space, &compound).is_err());
    }

    let wrong_source = FLOYD.replacen("fn floyd", "fn not_floyd", 1);
    let wrong_state = floyd_state(&wrong_source);
    let wrong_space = enumerate_tuning_space(&wrong_state).expect("wrong-site space");
    let (wrong_unit, wrong_variant) = target_variant(&wrong_space);
    let wrong_plan = wrong_space
        .plan_for_variant(&wrong_state, wrong_unit, wrong_variant)
        .expect("derive wrong-site plan")
        .expect("wrong-site plan");
    assert!(attest_selected_predicated_update(&wrong_state, &wrong_space, &wrong_plan).is_err());
}

fn target_variant(space: &calckernel::TuningSpace) -> (usize, usize) {
    space
        .units
        .iter()
        .enumerate()
        .find_map(|(unit_index, unit)| {
            unit.variants
                .iter()
                .position(|variant| {
                    matches!(
                        &variant.action,
                        TuneVariantAction::LoopSimd(candidate)
                            if candidate.predicated_update.is_some()
                    )
                })
                .map(|variant_index| (unit_index, variant_index))
        })
        .expect("predicated Floyd target")
}

fn floyd_state(source: &str) -> calckernel::KirVerifiedProgramState {
    let checked = check(&SourceFile::new("floyd.ck", source));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("Floyd MIR");
    let mut builder = KirTargetProfileBuilder::native(
        KirConsumer::NativeLibrary,
        "x86_64-unknown-linux-gnu",
        64,
        true,
        KirNativeCpuPolicy::Native,
        "x86-64-v4",
        vec!["+avx2".to_string()],
    )
    .expect("native profile");
    for key in KirTargetProfile::fixed_query_universe()
        .into_iter()
        .filter(|key| {
            (key.lanes == 2 || key.lanes == 4)
                && (key.lane == calckernel::KirLaneType::U32
                    || key.lane == calckernel::KirLaneType::F64)
                && matches!(
                    key.operation,
                    KirProfileOperation::Splat
                        | KirProfileOperation::Add
                        | KirProfileOperation::Subtract
                        | KirProfileOperation::Multiply
                        | KirProfileOperation::Load
                        | KirProfileOperation::Store
                        | KirProfileOperation::Compare
                        | KirProfileOperation::Select
                        | KirProfileOperation::Cast
                        | KirProfileOperation::Insert
                        | KirProfileOperation::Extract
                        | KirProfileOperation::RuntimePredicate
                )
                && (!matches!(
                    key.operation,
                    KirProfileOperation::Load | KirProfileOperation::Store
                ) || key.alignment
                    == KirAlignmentClass::Bytes(if key.lane == calckernel::KirLaneType::F64 {
                        8
                    } else {
                        4
                    }))
        })
    {
        let legalized_type = format!("test-{:?}-{}", key.lane, key.lanes);
        builder
            .set_legal(
                key,
                KirLegalCost {
                    cost: 1,
                    legalization_parts: 1,
                    legalized_type,
                },
            )
            .expect("legal vector query");
    }
    for key in KirTargetProfile::fixed_query_universe()
        .into_iter()
        .filter(|key| {
            key.lanes == 1
                && matches!(
                    (key.lane, key.operation),
                    (calckernel::KirLaneType::U32, KirProfileOperation::Add)
                        | (calckernel::KirLaneType::F64, KirProfileOperation::Add)
                )
        })
    {
        let legalized_type = if key.lane == calckernel::KirLaneType::F64 {
            "double"
        } else {
            "i32"
        }
        .to_string();
        builder
            .set_legal(
                key,
                KirLegalCost {
                    cost: 20,
                    legalization_parts: 1,
                    legalized_type,
                },
            )
            .expect("legal scalar query");
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
        builder.build().expect("profile"),
    )
    .expect("Floyd KIR");
    let contracts =
        import_contract_facts(&module, &checked.checked_program, 0).expect("Floyd contract facts");
    prepare_kir_pre_tune_state(module, Some(&contracts)).expect("verified Floyd pre-state")
}
