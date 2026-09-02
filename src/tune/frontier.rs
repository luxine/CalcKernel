use sha2::{Digest, Sha256};

use super::{ExpansionDisposition, ExpansionRecord, SearchFrontier};
use crate::TuningSpace;

/// Computes the schema-1 identity of the complete encoded candidate space and
/// ordinal-ordered expansion trace. The derived beam and compile selection are
/// intentionally excluded because the checker reconstructs them from the trace.
#[must_use]
pub fn canonical_frontier_digest(space: &TuningSpace, frontier: &SearchFrontier) -> [u8; 32] {
    let expansions = frontier
        .expansions
        .iter()
        .map(|expansion| record(&canonical_expansion(expansion)))
        .collect::<Vec<_>>();
    let mut hasher = Sha256::new();
    hasher.update(b"CK-TUNE-FRONTIER\0");
    hasher.update(space.digest);
    hasher.update(list(&expansions));
    hasher.finalize().into()
}

pub(crate) fn canonical_expansion(expansion: &ExpansionRecord) -> Vec<u8> {
    let mut out = Vec::new();
    field(&mut out, 1, &expansion.ordinal.to_be_bytes());
    field(&mut out, 2, &expansion.parent_plan_digest);
    field(&mut out, 3, &expansion.unit_id);
    field(&mut out, 4, &expansion.variant_id);
    let legal = expansion.disposition == ExpansionDisposition::Legal;
    field(
        &mut out,
        5,
        &[match expansion.disposition {
            ExpansionDisposition::Legal => 1,
            ExpansionDisposition::Duplicate => 3,
        }],
    );
    field(&mut out, 6, &optional(Some(&expansion.result_plan_digest)));
    field(&mut out, 7, &0u16.to_be_bytes());
    for (tag, metric) in (8u16..=10).zip([
        expansion.whole_plan_dynamic,
        expansion.whole_plan_static,
        expansion.whole_plan_kir_bytes,
    ]) {
        let metric = metric.to_be_bytes();
        field(&mut out, tag, &optional(legal.then_some(metric.as_slice())));
    }
    out
}

fn field(output: &mut Vec<u8>, tag: u16, payload: &[u8]) {
    output.extend_from_slice(&tag.to_be_bytes());
    output.extend_from_slice(
        &u32::try_from(payload.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );
    output.extend_from_slice(payload);
}

fn record(fields: &[u8]) -> Vec<u8> {
    let mut out = u32::try_from(fields.len())
        .unwrap_or(u32::MAX)
        .to_be_bytes()
        .to_vec();
    out.extend_from_slice(fields);
    out
}

fn list(items: &[Vec<u8>]) -> Vec<u8> {
    let mut out = u32::try_from(items.len())
        .unwrap_or(u32::MAX)
        .to_be_bytes()
        .to_vec();
    for item in items {
        out.extend_from_slice(item);
    }
    out
}

fn optional(value: Option<&[u8]>) -> Vec<u8> {
    let mut out = Vec::new();
    match value {
        Some(value) => {
            out.push(1);
            out.extend_from_slice(value);
        }
        None => out.push(0),
    }
    out
}
