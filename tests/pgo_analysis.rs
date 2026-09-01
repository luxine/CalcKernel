use calckernel::{
    CkAffineCostFormula, CkProfileCostClass, CkProfileCostDecision, CkProfileCostDomain,
    CkProfileCostProposal, CkProfileCounter, CkProfileCounterRecord, CkProfileMappingTransfer,
    CkProfileSiteDescriptor, CkProfileSiteId, CkProfileSiteKind, CkProfileTransferEntry,
    CkSignedMagnitude, profile_histogram_bucket_range, profile_ratio_at_least,
    profile_site_dominant_outcome, profile_site_table_digest, transfer_profile_counts,
    verify_profile_cost_proposal,
};

#[test]
fn confidence_boundaries_should_use_checked_integer_cross_multiplication() {
    assert!(profile_ratio_at_least(90, 100, 9_000));
    assert!(!profile_ratio_at_least(89, 100, 9_000));
    assert!(!profile_ratio_at_least(0, 0, 0));
    assert_eq!(profile_site_dominant_outcome(&[115, 13], 128, 9_000), None);
    assert_eq!(
        profile_site_dominant_outcome(&[116, 12], 128, 9_000),
        Some(0)
    );
    assert_eq!(
        profile_site_dominant_outcome(&[1088, 192], 128, 8_500),
        Some(0)
    );
    assert_eq!(
        profile_site_dominant_outcome(&[1087, 193], 128, 8_500),
        None
    );
}

#[test]
fn histogram_cost_checker_should_prove_every_bucket_lower_bound_and_recompute_total() {
    let expected = [
        (0, 0),
        (1, 1),
        (2, 2),
        (3, 4),
        (5, 8),
        (9, 16),
        (17, 32),
        (33, 64),
        (65, 128),
        (129, 256),
        (257, 512),
        (513, 1024),
        (1025, 2048),
        (2049, 4096),
        (4097, 65536),
        (65537, u32::MAX),
    ];
    for (bucket, range) in expected.into_iter().enumerate() {
        assert_eq!(profile_histogram_bucket_range(bucket as u8), Some(range));
    }
    assert_eq!(profile_histogram_bucket_range(16), None);

    let proposal = CkProfileCostProposal {
        classes: vec![CkProfileCostClass {
            count: 10,
            domain: CkProfileCostDomain::HistogramBucket(3),
            baseline: CkAffineCostFormula {
                fixed: 0,
                per_unit: 10,
            },
            selected: CkAffineCostFormula {
                fixed: 5,
                per_unit: 2,
            },
        }],
        guard_cost: 1,
        reported_net: CkSignedMagnitude {
            negative: false,
            magnitude: 180,
        },
    };
    assert_eq!(
        verify_profile_cost_proposal(&proposal),
        Ok(CkProfileCostDecision::Select { net_benefit: 180 })
    );
    let mut forged = proposal;
    forged.reported_net.magnitude += 1;
    assert!(verify_profile_cost_proposal(&forged).is_err());
}

#[test]
fn mapping_transfer_should_require_a_closed_checked_sum_or_make_targets_unknown() {
    let source_sites = vec![site(1), site(2)];
    let source_counters = vec![counter(1, 40), counter(2, 2)];
    let target_sites = vec![site(3)];
    let missing = transfer_profile_counts(&source_sites, &source_counters, &target_sites, None)
        .expect("missing transfer is conservative");
    assert_eq!(missing[0].counter, None);

    let transfer = CkProfileMappingTransfer {
        source_site_table_digest: profile_site_table_digest(&source_sites).expect("source digest"),
        target_site_table_digest: profile_site_table_digest(&target_sites).expect("target digest"),
        entries: vec![CkProfileTransferEntry {
            target: target_sites[0].id,
            sources: source_sites.iter().map(|site| site.id).collect(),
        }],
    };
    let mapped = transfer_profile_counts(
        &source_sites,
        &source_counters,
        &target_sites,
        Some(&transfer),
    )
    .expect("closed sum mapping");
    assert_eq!(mapped[0].counter, Some(CkProfileCounter::Scalar(42)));

    let mut forged = transfer;
    forged.entries[0].sources.pop();
    assert!(
        transfer_profile_counts(
            &source_sites,
            &source_counters,
            &target_sites,
            Some(&forged)
        )
        .is_err()
    );
}

fn site(byte: u8) -> CkProfileSiteDescriptor {
    CkProfileSiteDescriptor {
        id: CkProfileSiteId([byte; 16]),
        function_digest: [9; 32],
        location: u32::from(byte),
        kind: CkProfileSiteKind::FunctionEntry,
    }
}

fn counter(byte: u8, value: u64) -> CkProfileCounterRecord {
    CkProfileCounterRecord {
        site_id: CkProfileSiteId([byte; 16]),
        counter: CkProfileCounter::Scalar(value),
    }
}
