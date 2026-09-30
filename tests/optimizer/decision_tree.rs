use calckernel::{
    KirBoundsMode, KirBuildConfig, KirConsumer, KirCostEstimate, KirOptimizationLevel,
    KirOverflowMode, KirSanitizerMode, KirTargetProfile, KirVerifiedProgramState, KirWasmFeatures,
    SourceFile, check, discover_wasm_decision_tree_candidates, import_contract_facts, lower_to_mir,
    run_kir_pass_pipeline,
};

const FOUR_WAY_PIECEWISE: &str = r#"
export unsafe fn piecewise(input: slice<f64>, out: slice<f64>) -> void contract {
  requires input.len == out.len;
  requires noalias(input, out);
  effects read(input), write(out);
} {
  let i: u32 = 0;
  while i < input.len {
    let x: f64 = input[i];
    if x < -0.25 {
      out[i] = x * x + 0.5;
      i = i + 1;
      continue;
    }
    if x < 0.0 {
      out[i] = x * 0.75 - 0.125;
      i = i + 1;
      continue;
    }
    if x < 0.25 {
      out[i] = x * x * x + 0.25;
      i = i + 1;
      continue;
    }
    out[i] = (x - 0.25) * 1.5;
    i = i + 1;
  }
}
"#;

const FOUR_WAY_WITH_EXPLICIT_BOUND: &str = r#"
export unsafe fn piecewise(input: slice<f64>, out: slice<f64>, n: u32) -> void contract {
  requires noalias(input, out);
  effects read(input), write(out);
} {
  let i: u32 = 0;
  while i < n {
    let x: f64 = input[i];
    if x < -0.25 {
      out[i] = x * x + 0.5;
      i = i + 1;
      continue;
    }
    if x < 0.0 {
      out[i] = x * 0.75 - 0.125;
      i = i + 1;
      continue;
    }
    if x < 0.25 {
      out[i] = x * x * x + 0.25;
      i = i + 1;
      continue;
    }
    out[i] = (x - 0.25) * 1.5;
    i = i + 1;
  }
}
"#;

fn wasm_state(source: &str) -> KirVerifiedProgramState {
    let checked = check(&SourceFile::new("decision-tree.ck", source));
    assert_eq!(checked.diagnostics, [], "invalid test fixture:\n{source}");
    let mir = lower_to_mir(&checked.checked_program).expect("MIR lowering");
    let module = calckernel::build_kir_module_with_profile(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::WebAssembly,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        KirTargetProfile::webassembly_with_features(KirWasmFeatures::Simd128),
    )
    .expect("WASM KIR");
    let optimized = run_kir_pass_pipeline(module, KirOptimizationLevel::O3, None);
    assert!(optimized.errors.is_empty(), "{:?}", optimized.errors);
    let scalar = optimized.artifact.expect("scalar O3 KIR");
    let contracts =
        import_contract_facts(&scalar, &checked.checked_program, 0).expect("source contract facts");
    KirVerifiedProgramState::from_parts(
        scalar,
        Some(contracts),
        optimized.proofs,
        optimized.eliminated_guards,
        0,
    )
    .expect("verified O3 state")
}

#[test]
fn discovers_four_way_strict_piecewise_store_tree_with_contract_noalias() {
    let state = wasm_state(FOUR_WAY_PIECEWISE);
    let discovery = discover_wasm_decision_tree_candidates(&state);
    let mut variants = discovery
        .candidates
        .iter()
        .map(|candidate| (candidate.vf, candidate.uf))
        .collect::<Vec<_>>();
    variants.sort_unstable();
    assert_eq!(variants, [(2, 1), (2, 4)], "{discovery:?}");
    let candidate = discovery
        .candidates
        .iter()
        .find(|candidate| candidate.uf == 1)
        .expect("UF1 cost baseline");
    assert_eq!((candidate.vf, candidate.uf), (2, 1));
    assert_eq!(candidate.blocks.len(), 7, "three branches and four leaves");
    assert_eq!(candidate.range_requirements.len(), 2);
    assert_eq!(candidate.minimum_trip, 12);
    assert_eq!(
        candidate.predicted_cost,
        KirCostEstimate::new(299, 174, 34, 24)
    );
    assert!(
        u64::from(candidate.predicted_cost.total) * 100
            <= u64::from(candidate.predicted_cost.scalar) * 80
    );
    assert!(
        state
            .contract_facts()
            .and_then(|contracts| contracts.facts().get(candidate.noalias_fact))
            .is_some()
    );
}

#[test]
fn rejects_all_arm_cost_when_function_has_observable_work_before_tree() {
    let source =
        FOUR_WAY_PIECEWISE.replacen("  let i: u32 = 0;", "  out[0] = 0.0;\n  let i: u32 = 0;", 1);
    let state = wasm_state(&source);
    let discovery = discover_wasm_decision_tree_candidates(&state);
    assert!(
        discovery.candidates.is_empty(),
        "all-arm scalar pricing requires the proven whole-function lowering envelope: {discovery:?}"
    );
}

#[test]
fn resolves_loop_bound_through_header_phi_to_entry_argument() {
    let state = wasm_state(FOUR_WAY_WITH_EXPLICIT_BOUND);
    let discovery = discover_wasm_decision_tree_candidates(&state);
    let mut variants = discovery
        .candidates
        .iter()
        .map(|candidate| (candidate.vf, candidate.uf))
        .collect::<Vec<_>>();
    variants.sort_unstable();
    assert_eq!(variants, [(2, 1), (2, 4)], "{discovery:?}");
    let expected_bound = state.module().functions[0]
        .params
        .iter()
        .find(|parameter| parameter.name == "n")
        .expect("bound parameter")
        .value;
    assert!(
        discovery
            .candidates
            .iter()
            .all(|candidate| candidate.bound == expected_bound),
        "every UF variant must use the same explicit source bound"
    );
}

#[test]
fn rejects_piecewise_source_without_source_noalias_fact() {
    let source = FOUR_WAY_PIECEWISE.replace("  requires noalias(input, out);\n", "");
    let state = wasm_state(&source);
    let discovery = discover_wasm_decision_tree_candidates(&state);
    assert!(discovery.candidates.is_empty(), "{discovery:?}");
    assert!(
        discovery
            .fallbacks
            .iter()
            .any(|fallback| fallback.reason == "decision-tree-source-noalias-contract-not-proven")
    );
}

#[test]
fn rejects_floating_division_in_a_speculated_branch() {
    let source = FOUR_WAY_PIECEWISE.replace("x * x + 0.5", "x / (x - x)");
    let state = wasm_state(&source);
    let discovery = discover_wasm_decision_tree_candidates(&state);
    assert!(discovery.candidates.is_empty(), "{discovery:?}");
    assert!(discovery.fallbacks.iter().any(|fallback| {
        fallback.reason == "decision-tree-source-tree-shape-or-effects-not-proven"
    }));
}

#[test]
fn rejects_unbounded_speculative_work_from_one_large_leaf() {
    let expensive_product = format!("x{}", " * x".repeat(64));
    let source = FOUR_WAY_PIECEWISE.replace("x * x + 0.5", &expensive_product);
    let state = wasm_state(&source);
    let discovery = discover_wasm_decision_tree_candidates(&state);
    assert!(discovery.candidates.is_empty(), "{discovery:?}");
    assert!(
        discovery
            .fallbacks
            .iter()
            .any(|fallback| fallback.reason == "decision-tree-speculative-work-budget-exceeded")
    );
}
