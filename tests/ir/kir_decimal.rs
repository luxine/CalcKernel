use super::*;

fn measured(write_output: impl FnOnce(&mut String)) -> (String, usize) {
    let mut output = String::from("前缀:");
    KIR_FORMAT_DISPATCHES.with(|counter| counter.set(0));
    write_output(&mut output);
    let dispatches = KIR_FORMAT_DISPATCHES.with(std::cell::Cell::get);
    (output, dispatches)
}

#[test]
fn value_ids_preserve_decimal_boundaries_without_generic_format_dispatch() {
    let values = [
        0,
        1,
        9,
        10,
        99,
        100,
        999,
        1000,
        999_999_999,
        1_000_000_000,
        u32::MAX,
    ]
    .map(ValueId::from_index);
    let (text, dispatches) = measured(|output| write_values(output, &values));
    assert_eq!(
        text,
        "前缀:v0, v1, v9, v10, v99, v100, v999, v1000, v999999999, v1000000000, v4294967295"
    );
    assert_eq!(
        dispatches, 0,
        "ID-only output should not enter general fmt machinery"
    );
}

#[test]
fn edges_preserve_ids_and_memory_without_generic_format_dispatch() {
    let edge = KirEdge {
        target: BlockId::from_index(u32::MAX),
        args: vec![ValueId::from_index(0), ValueId::from_index(1_000_000_000)],
        memory_args: vec![
            MemoryVersionId::from_index(0),
            MemoryVersionId::from_index(u32::MAX),
        ],
    };
    let (text, dispatches) = measured(|output| write_edge(output, &edge));
    assert_eq!(
        text,
        "前缀:b4294967295(v0, v1000000000; memory m0, m4294967295)"
    );
    assert_eq!(dispatches, 0);
}

#[test]
fn empty_edges_keep_parentheses_without_generic_format_dispatch() {
    let edge = KirEdge {
        target: BlockId::from_index(0),
        args: vec![],
        memory_args: vec![],
    };
    let (text, dispatches) = measured(|output| write_edge(output, &edge));
    assert_eq!(text, "前缀:b0()");
    assert_eq!(dispatches, 0);
}

#[test]
fn return_memory_keeps_region_version_pairs_without_generic_format_dispatch() {
    let pairs = [(0, u32::MAX), (u32::MAX, 0), (10, 100)].map(|(region, version)| {
        (
            MemoryRegionId::from_index(region),
            MemoryVersionId::from_index(version),
        )
    });
    let (text, dispatches) = measured(|output| write_return_memory(output, &pairs));
    assert_eq!(
        text,
        "前缀: [memory r0=m4294967295, r4294967295=m0, r10=m100]"
    );
    assert_eq!(dispatches, 0);
}

#[test]
fn optional_proof_ids_keep_some_and_none_without_generic_format_dispatch() {
    let (text, dispatches) = measured(|output| {
        write_optional_proof(output, Some(ProofId::from_index(u32::MAX)));
        output.push(',');
        write_optional_proof(output, None);
    });
    assert_eq!(text, "前缀:p4294967295,none");
    assert_eq!(dispatches, 0);
}

#[test]
fn empty_id_collections_do_not_append_bytes() {
    let (text, dispatches) = measured(|output| {
        write_values(output, &[]);
        write_return_memory(output, &[]);
    });
    assert_eq!(text, "前缀:");
    assert_eq!(dispatches, 0);
}

#[test]
fn decimal_writer_matches_standard_formatting_across_bounded_full_width_values() {
    let mut values = (0..1000).collect::<Vec<u32>>();
    let mut value = 0x4b49_525f_u32;
    for _ in 0..10_000 {
        value = value.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        values.push(value);
    }
    for power in [
        1_u32,
        10,
        100,
        1000,
        10_000,
        100_000,
        1_000_000,
        10_000_000,
        100_000_000,
        1_000_000_000,
    ] {
        values.extend([power - 1, power, power + 1]);
    }
    values.push(u32::MAX);
    for value in values {
        let mut output = String::from("保留:");
        write_index(&mut output, "标识v", value, ":尾部");
        assert_eq!(output, format!("保留:标识v{value}:尾部"));
    }
}
