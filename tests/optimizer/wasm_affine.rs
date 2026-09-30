use std::collections::BTreeSet;

use calckernel::{
    KirAlignmentClass, KirBoundsMode, KirBuildConfig, KirConsumer, KirCostKey, KirCostSemantics,
    KirLaneType, KirOperationAvailability, KirOptimizationLevel, KirOverflowMode,
    KirProfileOperation, KirSanitizerMode, KirTargetProfile, KirVerifiedProgramState,
    KirWasmFeatures, SourceFile, WasmAffineShape, WasmRangeCount, check,
    discover_vectorization_candidates, import_contract_facts, lower_to_mir, run_kir_pass_pipeline,
};

const AFFINE_BROADCAST: &str = r#"
export unsafe fn affine(a: slice<f64>, b: slice<f64>, out: slice<f64>, n: u32, offset: u32, column: u32) -> void
contract {
  requires offset + n <= b.len && offset + n <= out.len && column < a.len;
  requires noalias(a, out) && noalias(b, out);
  effects read(a), read(b), readwrite(out);
}
{
  let i: u32 = 0;
  while i < n {
    let index: u32 = offset + i;
    let previous: f64 = out[index];
    let scalar: f64 = a[column];
    let varying: f64 = b[index];
    out[index] = previous + scalar * varying;
    i = i + 1;
  }
}
"#;

fn wasm_state(source: &str) -> KirVerifiedProgramState {
    wasm_state_at_level(source, KirOptimizationLevel::O2)
}

fn wasm_state_at_level(source: &str, level: KirOptimizationLevel) -> KirVerifiedProgramState {
    let checked = check(&SourceFile::new("wasm-affine.ck", source));
    assert_eq!(checked.diagnostics, [], "invalid test fixture:\n{source}");
    let mir = lower_to_mir(&checked.checked_program).expect("MIR lowering");
    let profile = KirTargetProfile::webassembly_with_features(KirWasmFeatures::Simd128);
    let module = calckernel::build_kir_module_with_profile(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::WebAssembly,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        profile,
    )
    .expect("WASM KIR");
    let contracts =
        import_contract_facts(&module, &checked.checked_program, 0).expect("contract facts");
    let optimized = run_kir_pass_pipeline(module, level, Some(&contracts));
    assert!(optimized.errors.is_empty(), "{:?}", optimized.errors);
    KirVerifiedProgramState::from_parts(
        optimized.artifact.expect("O2 KIR"),
        optimized.contract_facts,
        optimized.proofs,
        optimized.eliminated_guards,
        0,
    )
    .expect("verified O2 state")
}

fn wasm_scalar_o3_with_late_contracts(source: &str) -> KirVerifiedProgramState {
    // O3 now commits this candidate in production. Build the scalar O3
    // pre-state without alias facts, then attach the same source facts to
    // exercise proposal on the nested shape independently.
    let checked = check(&SourceFile::new("wasm-affine-nested.ck", source));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("nested MIR");
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
    .expect("nested KIR");
    let optimized = run_kir_pass_pipeline(module, KirOptimizationLevel::O3, None);
    assert!(optimized.errors.is_empty(), "{:?}", optimized.errors);
    let scalar = optimized.artifact.expect("scalar O3 KIR");
    let facts =
        import_contract_facts(&scalar, &checked.checked_program, 0).expect("late source facts");
    KirVerifiedProgramState::from_parts(
        scalar,
        Some(facts),
        optimized.proofs,
        optimized.eliminated_guards,
        0,
    )
    .expect("nested scalar pre-state")
}

fn affine_candidates(
    source: &str,
) -> (
    KirVerifiedProgramState,
    Vec<calckernel::VectorizationCandidate>,
) {
    let state = wasm_state(source);
    let candidates = discover_vectorization_candidates(&state)
        .candidates
        .into_iter()
        .filter(|candidate| candidate.wasm_affine.is_some())
        .collect();
    (state, candidates)
}

#[test]
fn wasm_affine_discovery_records_broadcast_contiguous_ranges_and_address_setup() {
    let (state, candidates) = affine_candidates(AFFINE_BROADCAST);
    assert_eq!(candidates.len(), 1, "{:?}", candidates);
    let candidate = &candidates[0];
    assert_eq!((candidate.vf, candidate.uf), (2, 1));
    assert!(candidate.version_predicate.is_none());

    let affine = candidate.wasm_affine.as_ref().expect("affine proof");
    let profile = &state.module().profile;
    let cost = |operation, lanes| match profile.operation_availability(&KirCostKey {
        operation,
        lane: KirLaneType::U32,
        lanes,
        semantics: KirCostSemantics::NotApplicable,
        alignment: KirAlignmentClass::NotApplicable,
    }) {
        Some(KirOperationAvailability::Legal(cost)) => cost.cost,
        Some(KirOperationAvailability::Unavailable) if operation == KirProfileOperation::Branch => {
            1
        }
        unavailable => panic!("expected cost entry for {operation:?}: {unavailable:?}"),
    };
    let expected_predicate_cost = cost(KirProfileOperation::Compare, 1)
        + cost(KirProfileOperation::Branch, 1)
        + 3 * cost(KirProfileOperation::RuntimePredicate, 2);
    assert_eq!(
        candidate.predicted_cost.predicates, expected_predicate_cost,
        "cost includes the trip threshold and every affine range guard"
    );
    let function = state
        .module()
        .functions
        .iter()
        .find(|function| function.id == candidate.function)
        .expect("candidate function");
    let parameter = |name: &str| {
        function
            .params
            .iter()
            .find(|parameter| parameter.name == name)
            .expect("named source parameter")
            .value
    };
    let a = parameter("a");
    let b = parameter("b");
    let out = parameter("out");
    let offset = parameter("offset");
    let column = parameter("column");
    let n = parameter("n");

    let a_accesses = affine
        .accesses
        .iter()
        .filter(|access| access.slice == a)
        .collect::<Vec<_>>();
    let b_accesses = affine
        .accesses
        .iter()
        .filter(|access| access.slice == b)
        .collect::<Vec<_>>();
    let out_accesses = affine
        .accesses
        .iter()
        .filter(|access| access.slice == out)
        .collect::<Vec<_>>();
    assert_eq!(a_accesses.len(), 1);
    assert!(matches!(
        a_accesses[0].shape,
        WasmAffineShape::Broadcast { index } if index == column
    ));
    assert_eq!(b_accesses.len(), 1);
    assert!(matches!(
        b_accesses[0].shape,
        WasmAffineShape::Contiguous { offset: Some(value) } if value == offset
    ));
    assert_eq!(
        out_accesses.len(),
        2,
        "out load/store share one complete address"
    );
    assert!(out_accesses.iter().all(|access| matches!(
        access.shape,
        WasmAffineShape::Contiguous { offset: Some(value) } if value == offset
    )));

    let setup = affine
        .scalar_address_setup
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    assert!(!setup.is_empty());
    assert_eq!(setup.len(), affine.scalar_address_setup.len());

    assert_eq!(affine.range_requirements.len(), 3);
    let ranges = &affine.range_requirements;
    assert!(ranges.iter().any(|range| {
        range.slice == a
            && range.start == Some(column)
            && range.count == WasmRangeCount::One
            && range.element_bytes == 8
    }));
    for slice in [b, out] {
        assert!(ranges.iter().any(|range| {
            range.slice == slice
                && range.start == Some(offset)
                && range.count == WasmRangeCount::TripBound(n)
                && range.element_bytes == 8
        }));
    }
}

#[test]
fn wasm_affine_discovery_handles_zero_offset_contiguous_sources() {
    let source = AFFINE_BROADCAST
        .replace(
            "offset + n <= b.len && offset + n <= out.len",
            "n <= b.len && n <= out.len",
        )
        .replace("let index: u32 = offset + i;", "let index: u32 = i;");
    let (state, candidates) = affine_candidates(&source);
    assert_eq!(candidates.len(), 1);
    let candidate = &candidates[0];
    let affine = candidate.wasm_affine.as_ref().expect("affine candidate");
    assert!(affine.scalar_address_setup.is_empty());
    for name in ["b", "out"] {
        let slice = state
            .module()
            .functions
            .iter()
            .find(|function| function.id == candidate.function)
            .and_then(|function| {
                function
                    .params
                    .iter()
                    .find(|parameter| parameter.name == name)
            })
            .expect("named slice parameter")
            .value;
        assert!(affine.range_requirements.iter().any(|range| {
            range.slice == slice
                && range.start.is_none()
                && range.count == WasmRangeCount::TripBound(candidate.bound)
        }));
    }
}

#[test]
fn wasm_affine_discovery_keeps_the_existing_direct_map_path() {
    let source = r#"
export unsafe fn map(a: slice<f64>, b: slice<f64>, out: slice<f64>, n: u32) -> void
contract {
  requires n <= a.len && n <= b.len && n <= out.len;
  requires noalias(a, b) && noalias(a, out) && noalias(b, out);
  effects read(a), read(b), write(out);
}
{
  let i: u32 = 0;
  while i < n { out[i] = a[i] + b[i]; i = i + 1; }
}
"#;
    let state = wasm_state(source);
    let candidates = discover_vectorization_candidates(&state).candidates;
    assert!(
        !candidates.is_empty(),
        "existing direct map path remains available"
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.wasm_affine.is_none())
    );
}

#[test]
fn wasm_affine_discovery_requires_noalias_and_matching_output_addresses() {
    let writable_broadcast = AFFINE_BROADCAST
        .replace(
            "effects read(a), read(b), readwrite(out);",
            "effects readwrite(a), read(b), readwrite(out);",
        )
        .replace(
            "let varying: f64 = b[index];",
            "let varying: f64 = b[index];\n    a[column] = scalar;",
        );
    for (case, source) in [
        AFFINE_BROADCAST.replace("noalias(a, out) && ", ""),
        AFFINE_BROADCAST.replace(" && noalias(b, out)", ""),
        AFFINE_BROADCAST.replace("out[index];", "out[index + 1];"),
        AFFINE_BROADCAST.replace("a[column];", "a[column + i];"),
        writable_broadcast,
        AFFINE_BROADCAST.replace(
            "previous + scalar * varying",
            "previous + scalar * varying + u32_to_f64(index)",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let (_, candidates) = affine_candidates(&source);
        assert!(
            candidates.is_empty(),
            "unexpected affine candidate for case {case}:\n{source}"
        );
    }
}

#[test]
fn wasm_affine_discovery_accepts_nested_column_update_with_hoisted_preheader_bases() {
    let source = r#"
export unsafe fn matmul_column(a: slice<f64>, b: slice<f64>, out: slice<f64>, n: u32) -> void
contract {
  requires n != 0 && n <= a.len && n <= b.len && n <= out.len;
  requires noalias(a, b) && noalias(a, out) && noalias(b, out);
  effects read(a), read(b), readwrite(out);
}
{
  let row: u32 = 0;
  while row < n {
    let inner: u32 = 0;
    while inner < n {
      let col: u32 = 0;
      while col < n {
        let out_index: u32 = row * n + col;
        let a_index: u32 = row * n + inner;
        let b_index: u32 = inner * n + col;
        let previous: f64 = out[out_index];
        let scalar: f64 = a[a_index];
        let varying: f64 = b[b_index];
        out[out_index] = previous + scalar * varying;
        col = col + 1;
      }
      inner = inner + 1;
    }
    row = row + 1;
  }
}
"#;
    let state = wasm_scalar_o3_with_late_contracts(source);
    let discovery = discover_vectorization_candidates(&state);
    let candidates = discovery
        .candidates
        .iter()
        .filter(|candidate| candidate.wasm_affine.is_some())
        .collect::<Vec<_>>();
    let mut variants = candidates
        .iter()
        .map(|candidate| (candidate.vf, candidate.uf))
        .collect::<Vec<_>>();
    variants.sort_unstable();
    assert_eq!(
        variants,
        [(2, 1), (2, 2), (2, 4)],
        "nested affine UF variants should be found; fallbacks: {:?}",
        discovery.fallbacks
    );
    let affine = candidates
        .iter()
        .find(|candidate| candidate.uf == 1)
        .expect("original UF1 affine candidate")
        .wasm_affine
        .as_ref()
        .expect("affine candidate");
    assert_eq!(affine.scalar_address_setup.len(), 2);
    assert_eq!(affine.range_requirements.len(), 3);
    assert!(candidates.iter().all(|candidate| {
        candidate
            .wasm_affine
            .as_ref()
            .is_some_and(|affine| affine.range_requirements.len() == 3)
    }));
}
