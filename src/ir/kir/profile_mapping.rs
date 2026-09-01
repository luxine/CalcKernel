use crate::{CkProfileSiteKind, profile_site_table_digest};

use super::{
    CkProfileEffectDomain, CkProfileKirError, CkProfileKirMode, CkProfileKirPlan,
    CkProfileMappingEntry, independently_rebuild_annotations, kir_digest, validate_kir_module,
};

/// Independently checks the closed site/annotation/operation mapping.
///
/// # Errors
///
/// Rejects stale KIR, forged identities/descriptors, missing, extra, duplicate,
/// or reordered records, and profile operations in off/use mode.
pub fn validate_ck_profile_kir_plan(plan: &CkProfileKirPlan) -> Result<(), CkProfileKirError> {
    let validation = validate_kir_module(&plan.module);
    if let Some(first) = validation.errors.first() {
        return Err(CkProfileKirError::InvalidKir(first.message.clone()));
    }
    if kir_digest(&plan.module) != plan.pre_profile_kir_digest {
        return Err(CkProfileKirError::Mapping(
            "pre-profile KIR digest mismatch",
        ));
    }
    if plan.mode == CkProfileKirMode::Off {
        if !plan.sites.is_empty()
            || !plan.annotations.is_empty()
            || !plan.operations.is_empty()
            || plan.mapping.is_some()
        {
            return Err(CkProfileKirError::Mapping(
                "off mode must not materialize profile records",
            ));
        }
        if profile_site_table_digest(&[])? != plan.site_table_digest {
            return Err(CkProfileKirError::Mapping("empty site digest mismatch"));
        }
        return Ok(());
    }

    let expected = independently_rebuild_annotations(&plan.module)?;
    if expected != plan.annotations {
        return Err(CkProfileKirError::Mapping(
            "annotation table does not match canonical events",
        ));
    }
    let sites = expected
        .iter()
        .map(|annotation| annotation.descriptor.clone())
        .collect::<Vec<_>>();
    if sites != plan.sites {
        return Err(CkProfileKirError::Mapping(
            "site descriptors do not match annotations",
        ));
    }
    if profile_site_table_digest(&sites)? != plan.site_table_digest {
        return Err(CkProfileKirError::Mapping("site table digest mismatch"));
    }
    let Some(mapping) = &plan.mapping else {
        return Err(CkProfileKirError::Mapping("mapping record is missing"));
    };
    if mapping.pre_profile_kir_digest != plan.pre_profile_kir_digest
        || mapping.site_table_digest != plan.site_table_digest
        || mapping.entries.len() != expected.len()
    {
        return Err(CkProfileKirError::Mapping("mapping identity mismatch"));
    }
    for (index, (entry, annotation)) in mapping.entries.iter().zip(&expected).enumerate() {
        let expected_index =
            u32::try_from(index).map_err(|_| CkProfileKirError::IdentityExhausted)?;
        let operation_index = (plan.mode == CkProfileKirMode::Generate).then_some(expected_index);
        let expected_entry = CkProfileMappingEntry {
            site_id: annotation.site_id,
            source: annotation.event.clone(),
            target: annotation.event.clone(),
            operation_index,
        };
        if entry != &expected_entry {
            return Err(CkProfileKirError::Mapping(
                "mapping is not canonical one-to-one transfer",
            ));
        }
    }
    match plan.mode {
        CkProfileKirMode::Generate => {
            if plan.operations.len() != expected.len() {
                return Err(CkProfileKirError::Mapping(
                    "generation operation count mismatch",
                ));
            }
            for (index, (operation, annotation)) in
                plan.operations.iter().zip(&expected).enumerate()
            {
                if operation.site_id != annotation.site_id
                    || operation.event != annotation.event
                    || operation.effect.domain != CkProfileEffectDomain::WorkloadProfile
                    || operation.effect.sequence
                        != u32::try_from(index).map_err(|_| CkProfileKirError::IdentityExhausted)?
                {
                    return Err(CkProfileKirError::Mapping(
                        "generation operation is missing, duplicated, or reordered",
                    ));
                }
            }
        }
        CkProfileKirMode::Use if !plan.operations.is_empty() => {
            return Err(CkProfileKirError::Mapping(
                "profile use must not contain counter operations",
            ));
        }
        CkProfileKirMode::Off | CkProfileKirMode::Use => {}
    }
    Ok(())
}

/// Prints a deterministic closed representation of the profile KIR sidecar.
#[must_use]
pub fn print_ck_profile_kir_plan(plan: &CkProfileKirPlan) -> String {
    let mut output = format!(
        "ck-profile-kir-v1 mode={} kir={} sites={}\n",
        plan.mode.stable_name(),
        hex(&plan.pre_profile_kir_digest),
        hex(&plan.site_table_digest),
    );
    for annotation in &plan.annotations {
        output.push_str(&format!(
            "site {} function={} location={} kind={} event={}\n",
            hex(&annotation.site_id.0),
            hex(&annotation.descriptor.function_digest),
            annotation.descriptor.location,
            print_kind(&annotation.descriptor.kind),
            print_event(&annotation.event),
        ));
    }
    for operation in &plan.operations {
        output.push_str(&format!(
            "operation {} effect=workload-profile:{} event={}\n",
            hex(&operation.site_id.0),
            operation.effect.sequence,
            print_event(&operation.event),
        ));
    }
    if let Some(mapping) = &plan.mapping {
        for entry in &mapping.entries {
            output.push_str(&format!(
                "mapping one-to-one site={} source={} target={} operation={}\n",
                hex(&entry.site_id.0),
                print_event(&entry.source),
                print_event(&entry.target),
                entry
                    .operation_index
                    .map_or_else(|| "none".to_string(), |index| index.to_string()),
            ));
        }
    }
    output
}

fn print_kind(kind: &CkProfileSiteKind) -> String {
    match kind {
        CkProfileSiteKind::FunctionEntry => "function-entry".to_string(),
        CkProfileSiteKind::Edge {
            from_block,
            to_block,
            reconstructed,
        } => format!("edge:{from_block}->{to_block}:reconstructed={reconstructed}"),
        CkProfileSiteKind::LoopTripHistogram { loop_identity } => {
            format!("loop-trip:{loop_identity}")
        }
        CkProfileSiteKind::SliceLengthHistogram { decision_identity } => {
            format!("slice-length:{decision_identity}")
        }
        CkProfileSiteKind::CandidateConstant {
            decision_identity,
            candidates,
        } => format!(
            "candidate-constant:{decision_identity}:[{}]",
            candidates
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}

fn print_event(event: &super::CkProfileEvent) -> String {
    match event {
        super::CkProfileEvent::FunctionEntry { function, block } => {
            format!("entry:f{}:b{}", function.index(), block.index())
        }
        super::CkProfileEvent::Edge { function, from, to } => format!(
            "edge:f{}:b{}->b{}",
            function.index(),
            from.index(),
            to.index()
        ),
        super::CkProfileEvent::LoopTrip {
            function,
            header,
            latches,
            exits,
        } => format!(
            "loop:f{}:h{}:latches=[{}]:exits=[{}]",
            function.index(),
            header.index(),
            latches
                .iter()
                .map(|block| format!("b{}", block.index()))
                .collect::<Vec<_>>()
                .join(","),
            exits
                .iter()
                .map(|(from, to)| format!("b{}->b{}", from.index(), to.index()))
                .collect::<Vec<_>>()
                .join(","),
        ),
        super::CkProfileEvent::SliceLength {
            function,
            block,
            instruction,
            value,
        } => format!(
            "slice-length:f{}:b{}:i{}:v{}",
            function.index(),
            block.index(),
            instruction.index(),
            value.index()
        ),
        super::CkProfileEvent::CandidateConstant {
            function,
            block,
            instruction,
            observed,
        } => format!(
            "candidate:f{}:b{}:i{}:v{}",
            function.index(),
            block.index(),
            instruction.index(),
            observed.index()
        ),
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}
