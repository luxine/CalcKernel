use std::collections::{BTreeMap, BTreeSet};

use super::{
    CkProfile, CkProfileAnalysis, CkProfileCounter, CkProfileCounterRecord, CkProfileError,
    CkProfileIdentity, CkProfileSiteDescriptor, CkProfileSiteId, CkProfileWorkTerm,
    profile_site_table_digest,
};

/// One target site and the complete set of old sites summed into it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkProfileTransferEntry {
    pub target: CkProfileSiteId,
    pub sources: Vec<CkProfileSiteId>,
}

/// Closed count-transfer proof record for a CFG-changing rewrite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkProfileMappingTransfer {
    pub source_site_table_digest: [u8; 32],
    pub target_site_table_digest: [u8; 32],
    pub entries: Vec<CkProfileTransferEntry>,
}

/// A transferred counter or an explicitly unavailable affected site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkTransferredProfileCounter {
    pub site_id: CkProfileSiteId,
    pub counter: Option<CkProfileCounter>,
}

/// Applies one terminal profile to an independently recreated canonical topology.
///
/// # Errors
///
/// Reports the first identity mismatch, any descriptor/counter mismatch, or
/// checked analysis overflow. Low confidence remains a successful analysis.
pub fn apply_profile(
    profile: &CkProfile,
    expected_identity: &CkProfileIdentity,
    expected_sites: &[CkProfileSiteDescriptor],
    work_terms: &[CkProfileWorkTerm],
) -> Result<CkProfileAnalysis, CkProfileError> {
    if let Some((field, expected, observed)) = expected_identity.first_mismatch(&profile.identity) {
        return Err(CkProfileError::IdentityMismatch {
            field,
            expected,
            observed,
        });
    }
    if profile.sites != expected_sites
        || profile.identity.module.site_table_digest != profile_site_table_digest(expected_sites)?
    {
        return Err(CkProfileError::SiteTableMismatch);
    }
    if profile.counters.len() != expected_sites.len()
        || profile
            .counters
            .iter()
            .zip(expected_sites)
            .any(|(counter, site)| counter.site_id != site.id)
    {
        return Err(CkProfileError::CounterTableMismatch);
    }
    super::analysis::analyze_profile(profile, work_terms)
}

/// Independently verifies and applies a closed one-to-one/sum count transfer.
///
/// A missing record after a changed site-table identity makes every target
/// unavailable. A forged or incomplete record is a compiler error.
///
/// # Errors
///
/// Rejects digest mismatch, missing/duplicate/extra sources or targets, counter
/// shape changes, and checked accumulation failures.
pub fn transfer_profile_counts(
    source_sites: &[CkProfileSiteDescriptor],
    source_counters: &[CkProfileCounterRecord],
    target_sites: &[CkProfileSiteDescriptor],
    transfer: Option<&CkProfileMappingTransfer>,
) -> Result<Vec<CkTransferredProfileCounter>, CkProfileError> {
    if source_sites.len() != source_counters.len()
        || source_sites
            .iter()
            .zip(source_counters)
            .any(|(site, counter)| site.id != counter.site_id)
    {
        return Err(CkProfileError::CounterTableMismatch);
    }
    let source_digest = profile_site_table_digest(source_sites)?;
    let target_digest = profile_site_table_digest(target_sites)?;
    if source_digest == target_digest && source_sites == target_sites && transfer.is_none() {
        return Ok(source_counters
            .iter()
            .map(|record| CkTransferredProfileCounter {
                site_id: record.site_id,
                counter: Some(record.counter.clone()),
            })
            .collect());
    }
    let Some(transfer) = transfer else {
        return Ok(target_sites
            .iter()
            .map(|site| CkTransferredProfileCounter {
                site_id: site.id,
                counter: None,
            })
            .collect());
    };
    if transfer.source_site_table_digest != source_digest
        || transfer.target_site_table_digest != target_digest
    {
        return Err(CkProfileError::MappingTransfer(
            "site-table digest mismatch",
        ));
    }
    let source = source_counters
        .iter()
        .map(|record| (record.site_id, &record.counter))
        .collect::<BTreeMap<_, _>>();
    let targets = target_sites
        .iter()
        .map(|site| site.id)
        .collect::<BTreeSet<_>>();
    let mut seen_sources = BTreeSet::new();
    let mut seen_targets = BTreeSet::new();
    let mut output = Vec::with_capacity(target_sites.len());
    for entry in &transfer.entries {
        if entry.sources.is_empty()
            || !targets.contains(&entry.target)
            || !seen_targets.insert(entry.target)
        {
            return Err(CkProfileError::MappingTransfer(
                "target mapping is not closed",
            ));
        }
        let mut aggregate = None;
        for source_id in &entry.sources {
            if !seen_sources.insert(*source_id) {
                return Err(CkProfileError::MappingTransfer(
                    "source mapping is not one-use",
                ));
            }
            let counter = source
                .get(source_id)
                .ok_or(CkProfileError::MappingTransfer(
                    "mapping names an unknown source",
                ))?;
            aggregate = Some(match aggregate {
                None => (*counter).clone(),
                Some(mut value) => {
                    add_counter(&mut value, counter)?;
                    value
                }
            });
        }
        output.push(CkTransferredProfileCounter {
            site_id: entry.target,
            counter: aggregate,
        });
    }
    if seen_sources.len() != source.len() || seen_targets != targets {
        return Err(CkProfileError::MappingTransfer("mapping is incomplete"));
    }
    output.sort_by_key(|record| record.site_id);
    Ok(output)
}

fn add_counter(
    target: &mut CkProfileCounter,
    source: &CkProfileCounter,
) -> Result<(), CkProfileError> {
    match (target, source) {
        (CkProfileCounter::Scalar(left), CkProfileCounter::Scalar(right)) => {
            *left = left.saturating_add(*right);
        }
        (
            CkProfileCounter::Histogram {
                buckets: left,
                saturated: left_saturated,
            },
            CkProfileCounter::Histogram {
                buckets: right,
                saturated: right_saturated,
            },
        ) => {
            for (left, right) in left.iter_mut().zip(right) {
                let previous = *left;
                *left = left.saturating_add(*right);
                *left_saturated |= *left == u64::MAX && previous != u64::MAX;
            }
            *left_saturated |= *right_saturated;
        }
        (
            CkProfileCounter::CandidateConstant {
                candidates: left,
                other: left_other,
                saturated: left_saturated,
            },
            CkProfileCounter::CandidateConstant {
                candidates: right,
                other: right_other,
                saturated: right_saturated,
            },
        ) if left.len() == right.len() => {
            for (left, right) in left.iter_mut().zip(right) {
                let previous = *left;
                *left = left.saturating_add(*right);
                *left_saturated |= *left == u64::MAX && previous != u64::MAX;
            }
            let previous = *left_other;
            *left_other = left_other.saturating_add(*right_other);
            *left_saturated |= *left_other == u64::MAX && previous != u64::MAX;
            *left_saturated |= *right_saturated;
        }
        _ => return Err(CkProfileError::MappingTransfer("counter shape changed")),
    }
    Ok(())
}
