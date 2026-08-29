use calckernel::{
    AliasKind, KirBoundsMode, KirBuildConfig, KirConsumer, KirMemoryRegionOrigin, KirOverflowMode,
    KirSanitizerMode, MemoryRegionId, SourceFile, analyze_regions, build_kir_module, check,
    import_contract_facts, lower_to_mir, query_alias, refine_memory_ssa, validate_kir_module,
};

fn build(source_text: &str) -> (calckernel::CheckedProgram, calckernel::KirModule) {
    let checked = check(&SourceFile::new("memory.ck", source_text));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR lowering");
    let kir = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::Inspection,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .expect("KIR construction");
    (checked.checked_program, kir)
}

#[test]
fn memory_region_analysis_should_preserve_roots_copies_and_symbolic_subslice_bytes() {
    let (_, kir) = build(
        r#"
        export fn select(items: slice<i32>, start: u32, end: u32) -> i32 {
          let copy: slice<i32> = items;
          let part: slice<i32> = copy[start..end];
          return part[0];
        }
        "#,
    );
    let function = &kir.functions[0];
    let regions = analyze_regions(function, None).expect("region analysis");
    let parameter_regions = function
        .regions
        .iter()
        .filter(|region| matches!(region.origin, KirMemoryRegionOrigin::Parameter(_)))
        .count();
    let subslice = function
        .regions
        .iter()
        .find(|region| matches!(region.origin, KirMemoryRegionOrigin::Subslice(_)))
        .expect("subslice region");
    let interval = regions
        .descriptor(subslice.id)
        .and_then(|descriptor| descriptor.byte_interval.as_ref())
        .expect("symbolic byte interval");

    assert_eq!(
        parameter_regions, 1,
        "descriptor copy is not a new allocation"
    );
    assert_eq!(
        subslice.parent,
        Some(regions.root(subslice.id).expect("root"))
    );
    assert_eq!(
        interval.element_type,
        calckernel::MirType::Primitive(calckernel::MirPrimitiveTypeName::I32)
    );
    assert_eq!(interval.scale_description(), "sizeof(i32)");
}

#[test]
fn memory_alias_query_should_prove_empty_and_disjoint_sibling_subslices() {
    let (_, kir) = build(
        r#"
        export fn split(items: slice<i32>) -> i32 {
          let empty: slice<i32> = items[0..0];
          let left: slice<i32> = items[0..2];
          let right: slice<i32> = items[2..4];
          return items[0];
        }
        "#,
    );
    let function = &kir.functions[0];
    let analysis = analyze_regions(function, None).expect("regions");
    let root = function
        .regions
        .iter()
        .find(|region| matches!(region.origin, KirMemoryRegionOrigin::Parameter(_)))
        .expect("root")
        .id;
    let children = function
        .regions
        .iter()
        .filter(|region| matches!(region.origin, KirMemoryRegionOrigin::Subslice(_)))
        .map(|region| region.id)
        .collect::<Vec<_>>();

    assert_eq!(
        query_alias(&analysis, children[0], root).kind,
        AliasKind::NoAlias
    );
    assert_eq!(
        query_alias(&analysis, children[1], children[2]).kind,
        AliasKind::NoAlias
    );
    assert_eq!(
        query_alias(&analysis, root, root).kind,
        AliasKind::MustAlias
    );
}

#[test]
fn memory_alias_query_should_keep_pairwise_noalias_from_becoming_parameter_wide() {
    let (checked, kir) = build(
        r#"
        export unsafe fn kernel(a: slice<i32>, b: slice<i32>, c: slice<i32>) -> i32
        contract { requires noalias(a, b); }
        { return a[0] + b[0] + c[0]; }
        "#,
    );
    let imported = import_contract_facts(&kir, &checked, 0).expect("contract facts");
    let function = &kir.functions[0];
    let analysis = analyze_regions(function, Some(imported.facts())).expect("regions");
    let roots = function
        .regions
        .iter()
        .filter_map(|region| match region.origin {
            KirMemoryRegionOrigin::Parameter(_) => Some(region.id),
            _ => None,
        })
        .collect::<Vec<MemoryRegionId>>();

    let ab = query_alias(&analysis, roots[0], roots[1]);
    assert_eq!(ab.kind, AliasKind::NoAlias);
    assert!(ab.fact.is_some());
    assert_eq!(
        query_alias(&analysis, roots[0], roots[2]).kind,
        AliasKind::MayAlias
    );
    assert_eq!(
        query_alias(&analysis, roots[1], roots[2]).kind,
        AliasKind::MayAlias
    );
    assert_eq!(
        analysis.partition(roots[0]),
        Some(MemoryRegionId::from_index(0))
    );
    assert_eq!(
        analysis.partition(roots[1]),
        Some(MemoryRegionId::from_index(0))
    );
}

#[test]
fn memory_ssa_refinement_should_thread_partitions_through_join_and_loop_phis() {
    let (checked, mut kir) = build(
        r#"
        export unsafe fn accumulate(a: slice<i32>, b: slice<i32>, n: u32) -> void
        contract { requires noalias(a, b); effects read(a), readwrite(b); }
        {
          let i: u32 = 0;
          while i < n {
            if i == 1 { b[i] = a[i]; } else { b[i] = b[i] + a[i]; }
            i = i + 1;
          }
        }
        "#,
    );
    let contracts = import_contract_facts(&kir, &checked, 0).expect("contracts");
    refine_memory_ssa(&mut kir, Some(contracts.facts())).expect("Memory SSA");
    let function = &kir.functions[0];

    assert_eq!(validate_kir_module(&kir).errors, []);
    assert!(
        function.blocks[1..]
            .iter()
            .all(|block| block.memory_params.len() == 2)
    );
    for block in &function.blocks {
        for edge in match &block.terminator {
            calckernel::KirTerminator::Return { .. } => Vec::new(),
            calckernel::KirTerminator::Jump { edge } => vec![edge],
            calckernel::KirTerminator::Branch {
                then_edge,
                else_edge,
                ..
            } => vec![then_edge, else_edge],
        } {
            assert_eq!(edge.memory_args.len(), 2);
        }
    }
}

#[test]
fn mutation_wrong_memory_partition_phi_argument_should_be_rejected() {
    let (checked, mut kir) = build(
        r#"
        export unsafe fn choose(a: slice<i32>, b: slice<i32>, flag: bool) -> i32
        contract { requires noalias(a, b); effects read(a), read(b); }
        { if flag { return a[0]; } return b[0]; }
        "#,
    );
    let contracts = import_contract_facts(&kir, &checked, 0).expect("contracts");
    refine_memory_ssa(&mut kir, Some(contracts.facts())).expect("Memory SSA");
    let edge = kir.functions[0]
        .blocks
        .iter_mut()
        .find_map(|block| match &mut block.terminator {
            calckernel::KirTerminator::Jump { edge } if edge.memory_args.len() == 2 => Some(edge),
            calckernel::KirTerminator::Branch { then_edge, .. }
                if then_edge.memory_args.len() == 2 =>
            {
                Some(then_edge)
            }
            _ => None,
        })
        .expect("two-partition edge");
    edge.memory_args.swap(0, 1);

    assert!(validate_kir_module(&kir).errors.iter().any(|error| {
        error
            .message
            .contains("memory edge argument partition does not match target phi")
    }));
}
