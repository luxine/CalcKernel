use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use super::schema::{
    DECISION_DIGEST_DOMAIN, MAX_TUNE_DECISION_BYTES, PLAN_DIGEST_DOMAIN, POLICY_DIGEST_DOMAIN,
    TUNE_DECISION_MAGIC, TUNE_DECISION_SCHEMA, TuneBudget,
};

const HEADER_BYTES: usize = 12;
const DIGEST_BYTES: usize = 32;

/// One structurally validated CK 0.14 tuning decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuneDecision {
    bytes: Vec<u8>,
}

/// Source- and artifact-aware replay material extracted only after complete
/// self-contained validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuneReplayRequirements {
    pub compiler_version: String,
    pub compiler_source: [u8; 32],
    pub llvm_bridge: [u8; 32],
    pub source_digest: [u8; 32],
    pub semantic_contract_digest: [u8; 32],
    pub pre_tune_kir_digest: [u8; 32],
    pub compilation_mode_digest: [u8; 32],
    pub output_kind: u8,
    pub target_triple: String,
    pub target_cpu: String,
    pub target_features: Vec<String>,
    pub target_profile: String,
    pub profile_digest: Option<[u8; 32]>,
    pub budget: TuneBudget,
    pub session_digest: [u8; 32],
    pub selected_plan_digest: [u8; 32],
    pub frontier_digest: [u8; 32],
    pub selected_pre_state_digest: [u8; 32],
    pub selected_post_state_digest: [u8; 32],
    pub object_graph_digest: [u8; 32],
    pub link_recipe_digest: [u8; 32],
    pub outputs: Vec<TuneReplayOutput>,
}

/// One role-tagged recorded output required by replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuneReplayOutput {
    pub role: u8,
    pub logical_basename: String,
    pub content_digest: [u8; 32],
    pub content_bytes: u64,
}

impl TuneDecision {
    /// Returns the exact validated decision bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Rechecks every equality derivable without source or artifact access.
    ///
    /// # Errors
    ///
    /// Returns the same stable failure categories as initial decoding.
    pub fn validate_self_contained(&self) -> Result<(), TuneDecisionError> {
        decode_tune_decision(&self.bytes).map(|_| ())
    }

    /// Extracts the closed source-aware replay identity.
    pub fn replay_requirements(&self) -> Result<TuneReplayRequirements, TuneDecisionError> {
        self.validate_self_contained()?;
        let body_end = self.bytes.len() - DIGEST_BYTES;
        let top = parse_fields(
            &self.bytes[HEADER_BYTES..body_end],
            1..=8,
            "top-level records",
        )?;
        let identity = parse_record_fields(top[0], 1..=22, "Identity")?;
        let contract = parse_record_fields(top[1], 1..=32, "Contract")?;
        let environment = parse_record_fields(top[3], 1..=19, "Environment")?;
        let target_record = parse_record_envelope(identity[20], "TargetIdentity")?;
        let target = parse_record_fields(target_record, 1..=4, "TargetIdentity")?;
        let profile_digest = match identity[21].split_first() {
            Some((&0, [])) => None,
            Some((&1, record)) => {
                let profile = parse_record_fields(
                    parse_record_envelope(record, "ProfileIdentity")?,
                    1..=6,
                    "ProfileIdentity",
                )?;
                Some(copy_digest(profile[4], "ProfileIdentity.contentDigest")?)
            }
            _ => return Err(TuneDecisionError::InvalidValue("Identity.profile")),
        };
        let features = parse_text_values(target[2], 256, "TargetIdentity.features")?;
        let selection = parse_record_fields(top[6], 1..=5, "Selection")?;
        let replay = parse_record_fields(top[7], 1..=10, "Replay")?;
        let (output_count, mut offset) = parse_list_header(replay[5], 3, "Replay.outputs")?;
        let mut outputs = Vec::with_capacity(usize::try_from(output_count).unwrap_or(0));
        for _ in 0..output_count {
            let (record, consumed) = parse_record_prefix(&replay[5][offset..], "OutputIdentity")?;
            let fields = parse_record_fields(record, 1..=4, "OutputIdentity")?;
            outputs.push(TuneReplayOutput {
                role: parse_enum(fields[0], 1..=3, "OutputIdentity.role")?,
                logical_basename: parse_text(fields[1], "OutputIdentity.logicalBasename")?
                    .to_string(),
                content_digest: copy_digest(fields[2], "OutputIdentity.contentDigest")?,
                content_bytes: parse_u64(fields[3], "OutputIdentity.contentBytes")?,
            });
            offset += consumed;
        }
        require_exact_end(replay[5], offset, "Replay.outputs")?;
        Ok(TuneReplayRequirements {
            compiler_version: parse_text(identity[0], "Identity.ckVersion")?.to_string(),
            compiler_source: copy_digest(identity[1], "Identity.compilerSource")?,
            llvm_bridge: copy_digest(identity[4], "Identity.llvmBridge")?,
            source_digest: copy_digest(identity[15], "Identity.sourceDigest")?,
            semantic_contract_digest: copy_digest(identity[16], "Identity.semanticContractDigest")?,
            pre_tune_kir_digest: copy_digest(identity[17], "Identity.preTuneKirDigest")?,
            compilation_mode_digest: copy_digest(identity[18], "Identity.compilationModeDigest")?,
            output_kind: parse_enum(identity[19], 1..=2, "Identity.outputKind")?,
            target_triple: parse_text(target[0], "TargetIdentity.triple")?.to_string(),
            target_cpu: parse_text(target[1], "TargetIdentity.cpu")?.to_string(),
            target_features: features,
            target_profile: parse_text(target[3], "TargetIdentity.targetProfile")?.to_string(),
            profile_digest,
            budget: match parse_enum(contract[5], 1..=3, "Contract.preset")? {
                1 => TuneBudget::Quick,
                2 => TuneBudget::Standard,
                3 => TuneBudget::Thorough,
                _ => unreachable!("closed preset parser"),
            },
            session_digest: copy_digest(environment[17], "Environment.sessionDigest")?,
            selected_plan_digest: copy_digest(selection[2], "Selection.selectedPlanDigest")?,
            frontier_digest: copy_digest(replay[0], "Replay.frontierDigest")?,
            selected_pre_state_digest: copy_digest(replay[1], "Replay.selectedPreState")?,
            selected_post_state_digest: copy_digest(replay[2], "Replay.selectedPostState")?,
            object_graph_digest: copy_digest(replay[3], "Replay.objectGraphDigest")?,
            link_recipe_digest: copy_digest(replay[4], "Replay.linkRecipeDigest")?,
            outputs,
        })
    }

    /// Reports whether any recorded candidate terminated with a canonical
    /// timeout. Completed baseline decisions with such a timeout are not warm
    /// cache hits because their interrupted search must be measured afresh.
    pub fn has_candidate_timeout(&self) -> Result<bool, TuneDecisionError> {
        self.validate_self_contained()?;
        let body_end = self.bytes.len() - DIGEST_BYTES;
        let top = parse_fields(
            &self.bytes[HEADER_BYTES..body_end],
            1..=8,
            "top-level records",
        )?;
        let candidates = parse_record_fields(top[5], 1..=2, "Candidates")?;
        let baseline = parse_record_fields(
            parse_record_envelope(candidates[0], "Candidate")?,
            1..=12,
            "Candidate",
        )?;
        if optional_is_present(baseline[10], "Candidate.timeout")? {
            return Ok(true);
        }
        let (count, mut offset) = parse_list_header(candidates[1], 32, "Candidates.trials")?;
        for _ in 0..count {
            let (record, consumed) = parse_record_prefix(&candidates[1][offset..], "Candidate")?;
            let fields = parse_record_fields(record, 1..=12, "Candidate")?;
            if optional_is_present(fields[10], "Candidate.timeout")? {
                return Ok(true);
            }
            offset = offset
                .checked_add(consumed)
                .ok_or(TuneDecisionError::ResourceLimit("Candidates.trials"))?;
        }
        require_exact_end(candidates[1], offset, "Candidates.trials")?;
        Ok(false)
    }
}

fn optional_is_present(bytes: &[u8], context: &'static str) -> Result<bool, TuneDecisionError> {
    match bytes.split_first() {
        Some((&0, [])) => Ok(false),
        Some((&1, value)) if !value.is_empty() => Ok(true),
        _ => Err(TuneDecisionError::InvalidValue(context)),
    }
}

fn copy_digest(bytes: &[u8], context: &'static str) -> Result<[u8; 32], TuneDecisionError> {
    bytes
        .try_into()
        .map_err(|_| TuneDecisionError::InvalidValue(context))
}

fn parse_text_values(
    bytes: &[u8],
    limit: u32,
    context: &'static str,
) -> Result<Vec<String>, TuneDecisionError> {
    let (count, mut offset) = parse_list_header(bytes, limit, context)?;
    let mut values = Vec::with_capacity(usize::try_from(count).unwrap_or(0));
    for _ in 0..count {
        let (value, consumed) = parse_text_prefix(&bytes[offset..], context)?;
        values.push(value.to_string());
        offset += consumed;
    }
    require_exact_end(bytes, offset, context)?;
    Ok(values)
}

/// Encodes a previously validated decision using its unique schema-1 bytes.
#[must_use]
pub fn encode_tune_decision(decision: &TuneDecision) -> Vec<u8> {
    decision.bytes.clone()
}

/// Stable failure categories for bounded tuning-decision parsing.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TuneDecisionError {
    #[error("truncated {0}")]
    Truncated(&'static str),
    #[error("unexpected tuning-decision magic")]
    UnexpectedMagic,
    #[error("unsupported tuning-decision schema")]
    UnsupportedSchema,
    #[error("tuning-decision digest mismatch")]
    DigestMismatch,
    #[error("non-canonical {0}")]
    NonCanonicalOrder(&'static str),
    #[error("missing tag {tag} in {record}")]
    MissingField { record: &'static str, tag: u16 },
    #[error("resource limit exceeded for {0}")]
    ResourceLimit(&'static str),
    #[error("invalid value for {0}")]
    InvalidValue(&'static str),
    #[error("invalid UTF-8")]
    InvalidUtf8,
}

/// Parses one canonical `CKTUNE01` decision.
///
/// # Errors
///
/// Returns a stable validation error for truncated, foreign, or unsupported input.
pub fn decode_tune_decision(bytes: &[u8]) -> Result<TuneDecision, TuneDecisionError> {
    if bytes.len() < HEADER_BYTES {
        return Err(TuneDecisionError::Truncated("decision header"));
    }
    if bytes.len() > MAX_TUNE_DECISION_BYTES {
        return Err(TuneDecisionError::ResourceLimit("decision bytes"));
    }
    if &bytes[..TUNE_DECISION_MAGIC.len()] != TUNE_DECISION_MAGIC {
        return Err(TuneDecisionError::UnexpectedMagic);
    }
    if u32::from_be_bytes(
        bytes[8..12]
            .try_into()
            .map_err(|_| TuneDecisionError::Truncated("decision schema"))?,
    ) != TUNE_DECISION_SCHEMA
    {
        return Err(TuneDecisionError::UnsupportedSchema);
    }
    if bytes.len() < HEADER_BYTES + DIGEST_BYTES {
        return Err(TuneDecisionError::Truncated("decision digest"));
    }
    let body_end = bytes.len() - DIGEST_BYTES;
    let mut hasher = Sha256::new();
    hasher.update(DECISION_DIGEST_DOMAIN);
    hasher.update(&bytes[..body_end]);
    if hasher.finalize().as_slice() != &bytes[body_end..] {
        return Err(TuneDecisionError::DigestMismatch);
    }
    let fields = parse_fields(&bytes[HEADER_BYTES..body_end], 1..=8, "top-level records")?;
    validate_identity(fields[0])?;
    validate_contract(fields[1])?;
    validate_workload(fields[2])?;
    validate_environment(fields[3])?;
    validate_frontier(fields[4])?;
    validate_candidates(fields[5])?;
    validate_selection(fields[6])?;
    validate_replay(fields[7])?;
    validate_cross_record_equalities(&fields)?;
    Ok(TuneDecision {
        bytes: bytes.to_vec(),
    })
}

fn validate_cross_record_equalities(fields: &[&[u8]]) -> Result<(), TuneDecisionError> {
    validate_frontier_equalities(fields)?;
    let candidates = parse_record_fields(fields[5], 1..=2, "Candidates")?;
    let baseline = parse_record_envelope(candidates[0], "Candidate")?;
    let baseline_fields = parse_record_fields(baseline, 1..=12, "Candidate")?;
    validate_plan_digest(baseline_fields[0], baseline_fields[1])?;
    let empty_plan = domain_hash(PLAN_DIGEST_DOMAIN, &0u32.to_be_bytes());
    if baseline_fields[0] != empty_plan {
        return Err(TuneDecisionError::InvalidValue("Candidate.planDigest"));
    }

    let mut all_candidates = vec![baseline_fields];
    let (trial_count, mut offset) = parse_list_header(candidates[1], 32, "Candidates.trials")?;
    for _ in 0..trial_count {
        let (trial, consumed) = parse_record_prefix(
            candidates[1]
                .get(offset..)
                .ok_or(TuneDecisionError::Truncated("Candidates.trials"))?,
            "Candidate",
        )?;
        let trial_fields = parse_record_fields(trial, 1..=12, "Candidate")?;
        validate_plan_digest(trial_fields[0], trial_fields[1])?;
        all_candidates.push(trial_fields);
        offset = offset
            .checked_add(consumed)
            .ok_or(TuneDecisionError::ResourceLimit("Candidates.trials"))?;
    }
    require_exact_end(candidates[1], offset, "Candidates.trials")?;

    let selection = parse_record_fields(fields[6], 1..=5, "Selection")?;
    let reason = parse_enum(selection[3], 1..=4, "Selection.reason")?;
    let selected = selection[2];
    let selected_candidate = all_candidates
        .iter()
        .find(|candidate| candidate[0] == selected)
        .ok_or(TuneDecisionError::InvalidValue(
            "Selection.selectedPlanDigest",
        ))?;
    if reason != 1 && selected != empty_plan {
        return Err(TuneDecisionError::InvalidValue(
            "Selection.selectedPlanDigest",
        ));
    }

    for candidate in &all_candidates {
        validate_compile_cache_origin(fields[0], candidate)?;
    }

    let replay = parse_record_fields(fields[7], 1..=10, "Replay")?;
    if replay[3] != selected_candidate[2] || replay[4] != selected_candidate[3] {
        return Err(TuneDecisionError::InvalidValue("Replay artifact identity"));
    }
    if replay[6] != selected_candidate[9] {
        return Err(TuneDecisionError::InvalidValue("Replay.compileOrigin"));
    }
    validate_measurement_cache_origin(fields, replay[7])?;
    validate_replay_primary(replay[5], selected_candidate[11], selected_candidate[4])?;
    Ok(())
}

fn validate_compile_cache_origin(
    identity: &[u8],
    candidate: &[&[u8]],
) -> Result<(), TuneDecisionError> {
    let origin = parse_record_fields(
        parse_record_envelope(candidate[9], "CacheOrigin")?,
        1..=3,
        "CacheOrigin",
    )?;
    let mut key_material = Vec::new();
    append_field(&mut key_material, 1, &1u32.to_be_bytes());
    append_field(&mut key_material, 2, &record_envelope(identity));
    append_field(&mut key_material, 3, candidate[0]);
    let key_digest = record_domain_hash(b"CK-TUNE-COMPILE-KEY\0", &key_material);
    if origin[1] != key_digest {
        return Err(TuneDecisionError::InvalidValue(
            "CacheOrigin.compileKeyDigest",
        ));
    }
    let mut entry_material = Vec::new();
    append_field(&mut entry_material, 1, &key_digest);
    append_field(&mut entry_material, 2, candidate[11]);
    append_field(&mut entry_material, 3, candidate[4]);
    append_field(&mut entry_material, 4, candidate[2]);
    append_field(&mut entry_material, 5, candidate[3]);
    if origin[2] != record_domain_hash(b"CK-TUNE-COMPILE-ENTRY\0", &entry_material) {
        return Err(TuneDecisionError::InvalidValue(
            "CacheOrigin.compileEntryDigest",
        ));
    }
    Ok(())
}

fn validate_measurement_cache_origin(
    records: &[&[u8]],
    encoded_origin: &[u8],
) -> Result<(), TuneDecisionError> {
    let environment = parse_record_fields(records[3], 1..=19, "Environment")?;
    let origin = parse_record_fields(
        parse_record_envelope(encoded_origin, "CacheOrigin")?,
        1..=3,
        "CacheOrigin",
    )?;
    let mut key_material = Vec::new();
    append_field(&mut key_material, 1, &1u32.to_be_bytes());
    append_field(&mut key_material, 2, environment[17]);
    append_field(&mut key_material, 3, environment[18]);
    let key_digest = record_domain_hash(b"CK-TUNE-MEASUREMENT-KEY\0", &key_material);
    if origin[1] != key_digest {
        return Err(TuneDecisionError::InvalidValue(
            "CacheOrigin.measurementKeyDigest",
        ));
    }
    let mut entry_material = Vec::new();
    append_field(&mut entry_material, 1, &key_digest);
    append_field(&mut entry_material, 2, &record_envelope(records[5]));
    append_field(&mut entry_material, 3, &record_envelope(records[6]));
    if origin[2] != record_domain_hash(b"CK-TUNE-MEASUREMENT-ENTRY\0", &entry_material) {
        return Err(TuneDecisionError::InvalidValue(
            "CacheOrigin.measurementEntryDigest",
        ));
    }
    Ok(())
}

fn validate_frontier_equalities(fields: &[&[u8]]) -> Result<(), TuneDecisionError> {
    let identity = parse_record_fields(fields[0], 1..=22, "Identity")?;
    let pre_tune = identity[17];
    let frontier = parse_record_fields(fields[4], 1..=4, "Frontier")?;
    let mut site_classes = BTreeMap::<Vec<u8>, (u8, [u8; 32])>::new();
    let mut prior_site = None::<[u8; 32]>;
    let (site_count, mut site_offset) = parse_list_header(frontier[1], 4_096, "Frontier.sites")?;
    for _ in 0..site_count {
        let (site, consumed) = parse_record_prefix(&frontier[1][site_offset..], "Frontier.sites")?;
        let site_fields = parse_record_fields(site, 1..=6, "Site")?;
        let site_id = copy_digest(site_fields[0], "Site.siteId")?;
        if prior_site.is_some_and(|prior| prior >= site_id) {
            return Err(TuneDecisionError::InvalidValue("Frontier.sites order"));
        }
        prior_site = Some(site_id);
        let class = parse_enum(site_fields[1], 1..=7, "Site.class")?;
        let mut root_material = Vec::new();
        append_field(&mut root_material, 1, pre_tune);
        append_field(&mut root_material, 2, site_fields[5]);
        if site_fields[2] != record_domain_hash(b"CK-TUNE-ROOT\0", &root_material) {
            return Err(TuneDecisionError::InvalidValue("Site.rootId"));
        }
        let mut site_material = Vec::new();
        append_field(&mut site_material, 1, site_fields[2]);
        append_field(&mut site_material, 2, site_fields[1]);
        append_field(&mut site_material, 3, site_fields[4]);
        append_field(&mut site_material, 4, site_fields[3]);
        if site_fields[0] != record_domain_hash(b"CK-TUNE-SITE\0", &site_material) {
            return Err(TuneDecisionError::InvalidValue("Site.siteId"));
        }
        if site_classes
            .insert(
                site_id.to_vec(),
                (class, copy_digest(site_fields[3], "Site.preStateDigest")?),
            )
            .is_some()
        {
            return Err(TuneDecisionError::InvalidValue("Frontier.sites duplicate"));
        }
        site_offset = site_offset
            .checked_add(consumed)
            .ok_or(TuneDecisionError::ResourceLimit("Frontier.sites"))?;
    }
    require_exact_end(frontier[1], site_offset, "Frontier.sites")?;

    let mut prior_unit = None::<(u8, [u8; 32])>;
    let mut unit_ids = BTreeSet::new();
    let (unit_count, mut unit_offset) = parse_list_header(frontier[2], 64, "Frontier.units")?;
    for _ in 0..unit_count {
        let (unit, consumed) = parse_record_prefix(&frontier[2][unit_offset..], "Frontier.units")?;
        let unit_fields = parse_record_fields(unit, 1..=4, "Unit")?;
        let unit_id = copy_digest(unit_fields[0], "Unit.unitId")?;
        let mut unit_material = Vec::new();
        append_field(&mut unit_material, 1, unit_fields[1]);
        append_field(&mut unit_material, 2, unit_fields[2]);
        if unit_fields[0] != record_domain_hash(b"CK-TUNE-UNIT\0", &unit_material) {
            return Err(TuneDecisionError::InvalidValue("Unit.unitId"));
        }
        let site_ids = parse_digest_values(unit_fields[1], 4_096, "Unit.siteIds")?;
        if site_ids.is_empty() || site_ids.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(TuneDecisionError::InvalidValue("Unit.siteIds"));
        }
        let mut unit_class = None;
        for site_id in &site_ids {
            let (class, site_pre) = site_classes
                .get(site_id.as_slice())
                .copied()
                .ok_or(TuneDecisionError::InvalidValue("Unit.siteIds"))?;
            if unit_fields[2] != site_pre {
                return Err(TuneDecisionError::InvalidValue("Unit.baselineStateDigest"));
            }
            if unit_class
                .replace(class)
                .is_some_and(|prior| prior != class)
            {
                return Err(TuneDecisionError::InvalidValue("Unit site class"));
            }
        }
        let class = unit_class.ok_or(TuneDecisionError::InvalidValue("Unit.siteIds"))?;
        let phase = replay_phase(class);
        if prior_unit.is_some_and(|prior| prior >= (phase, unit_id)) {
            return Err(TuneDecisionError::InvalidValue("Frontier.units order"));
        }
        prior_unit = Some((phase, unit_id));
        if !unit_ids.insert(unit_id) {
            return Err(TuneDecisionError::InvalidValue("Frontier.units duplicate"));
        }

        let (variant_count, mut variant_offset) =
            parse_list_header(unit_fields[3], 4, "Unit.variants")?;
        let mut prior_variant = None::<[u8; 32]>;
        for _ in 0..variant_count {
            let (variant, variant_consumed) =
                parse_record_prefix(&unit_fields[3][variant_offset..], "Unit.variants")?;
            let variant_fields = parse_record_fields(variant, 1..=7, "UnitVariant")?;
            let variant_id = copy_digest(variant_fields[0], "UnitVariant.variantId")?;
            if prior_variant.is_some_and(|prior| prior >= variant_id)
                || variant_fields[1] != [class]
            {
                return Err(TuneDecisionError::InvalidValue(
                    "Unit.variants order or class",
                ));
            }
            prior_variant = Some(variant_id);
            validate_site_alternative_equalities(
                variant_fields[2],
                class,
                &site_ids,
                &site_classes,
                variant_fields[6],
            )?;
            let mut variant_material = Vec::new();
            append_field(&mut variant_material, 1, unit_fields[0]);
            for (tag, value) in (2u16..=7).zip(&variant_fields[1..7]) {
                append_field(&mut variant_material, tag, value);
            }
            if variant_fields[0] != record_domain_hash(b"CK-TUNE-UNIT-VARIANT\0", &variant_material)
            {
                return Err(TuneDecisionError::InvalidValue("UnitVariant.variantId"));
            }
            variant_offset = variant_offset
                .checked_add(variant_consumed)
                .ok_or(TuneDecisionError::ResourceLimit("Unit.variants"))?;
        }
        require_exact_end(unit_fields[3], variant_offset, "Unit.variants")?;
        unit_offset = unit_offset
            .checked_add(consumed)
            .ok_or(TuneDecisionError::ResourceLimit("Frontier.units"))?;
    }
    require_exact_end(frontier[2], unit_offset, "Frontier.units")?;

    let mut space_material = Vec::new();
    append_field(&mut space_material, 1, frontier[1]);
    append_field(&mut space_material, 2, frontier[2]);
    if frontier[0] != record_domain_hash(b"CK-TUNE-CANDIDATE-SPACE\0", &space_material) {
        return Err(TuneDecisionError::InvalidValue(
            "Frontier.candidateSpaceDigest",
        ));
    }
    let mut frontier_hasher = Sha256::new();
    frontier_hasher.update(b"CK-TUNE-FRONTIER\0");
    frontier_hasher.update(frontier[0]);
    frontier_hasher.update(frontier[3]);
    let replay = parse_record_fields(fields[7], 1..=10, "Replay")?;
    if replay[0] != frontier_hasher.finalize().as_slice() {
        return Err(TuneDecisionError::InvalidValue("Replay.frontierDigest"));
    }
    Ok(())
}

fn validate_site_alternative_equalities(
    bytes: &[u8],
    class: u8,
    unit_site_ids: &[[u8; 32]],
    site_classes: &BTreeMap<Vec<u8>, (u8, [u8; 32])>,
    variant_post: &[u8],
) -> Result<(), TuneDecisionError> {
    let (count, mut offset) = parse_list_header(bytes, 4_096, "UnitVariant.siteAlternatives")?;
    if usize::try_from(count).ok() != Some(unit_site_ids.len()) {
        return Err(TuneDecisionError::InvalidValue(
            "UnitVariant.siteAlternatives",
        ));
    }
    let mut seen = Vec::new();
    let mut prior = None::<([u8; 32], [u8; 32])>;
    for _ in 0..count {
        let (alternative, consumed) =
            parse_record_prefix(&bytes[offset..], "UnitVariant.siteAlternatives")?;
        let fields = parse_record_fields(alternative, 1..=5, "SiteAlternative")?;
        let site_id = copy_digest(fields[0], "SiteAlternative.siteId")?;
        let alternative_id = copy_digest(fields[1], "SiteAlternative.alternativeId")?;
        let key = (site_id, alternative_id);
        let Some((site_class, site_pre)) = site_classes.get(site_id.as_slice()).copied() else {
            return Err(TuneDecisionError::InvalidValue(
                "UnitVariant.siteAlternatives site",
            ));
        };
        if prior.is_some_and(|prior| prior >= key) || site_class != class || fields[2] != site_pre {
            return Err(TuneDecisionError::InvalidValue(
                "UnitVariant.siteAlternatives order or class",
            ));
        }
        prior = Some(key);
        seen.push(site_id);
        let payload = parse_record_fields(
            parse_record_envelope(fields[4], "AlternativePayload")?,
            1..=2,
            "AlternativePayload",
        )?;
        if payload[0] != [class] || fields[3] != variant_post {
            return Err(TuneDecisionError::InvalidValue(
                "SiteAlternative class or post-state",
            ));
        }
        let mut material = Vec::new();
        append_field(&mut material, 1, fields[0]);
        append_field(&mut material, 2, fields[4]);
        append_field(&mut material, 3, fields[3]);
        if fields[1] != record_domain_hash(b"CK-TUNE-ALTERNATIVE\0", &material) {
            return Err(TuneDecisionError::InvalidValue(
                "SiteAlternative.alternativeId",
            ));
        }
        offset = offset
            .checked_add(consumed)
            .ok_or(TuneDecisionError::ResourceLimit(
                "UnitVariant.siteAlternatives",
            ))?;
    }
    require_exact_end(bytes, offset, "UnitVariant.siteAlternatives")?;
    if seen != unit_site_ids {
        return Err(TuneDecisionError::InvalidValue(
            "UnitVariant.siteAlternatives site set",
        ));
    }
    Ok(())
}

fn parse_digest_values(
    bytes: &[u8],
    maximum: u32,
    name: &'static str,
) -> Result<Vec<[u8; 32]>, TuneDecisionError> {
    let (count, mut offset) = parse_list_header(bytes, maximum, name)?;
    let mut values = Vec::with_capacity(usize::try_from(count).unwrap_or(0));
    for _ in 0..count {
        let end = offset
            .checked_add(32)
            .ok_or(TuneDecisionError::ResourceLimit(name))?;
        values.push(copy_digest(
            bytes
                .get(offset..end)
                .ok_or(TuneDecisionError::Truncated(name))?,
            name,
        )?);
        offset = end;
    }
    require_exact_end(bytes, offset, name)?;
    Ok(values)
}

fn replay_phase(class: u8) -> u8 {
    match class {
        2 => 1,
        1 => 2,
        6 => 3,
        4 => 4,
        3 => 5,
        5 => 6,
        7 => 7,
        _ => unreachable!("AlternativeClass was already validated"),
    }
}

fn append_field(output: &mut Vec<u8>, tag: u16, payload: &[u8]) {
    output.extend_from_slice(&tag.to_be_bytes());
    output.extend_from_slice(
        &u32::try_from(payload.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );
    output.extend_from_slice(payload);
}

fn record_envelope(fields: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(4 + fields.len());
    output.extend_from_slice(
        &u32::try_from(fields.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );
    output.extend_from_slice(fields);
    output
}

fn record_domain_hash(domain: &[u8], fields: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(
        u32::try_from(fields.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );
    hasher.update(fields);
    hasher.finalize().into()
}

fn validate_plan_digest(stored: &[u8], choices: &[u8]) -> Result<(), TuneDecisionError> {
    let derived = domain_hash(PLAN_DIGEST_DOMAIN, choices);
    if stored != derived {
        return Err(TuneDecisionError::InvalidValue("Candidate.planDigest"));
    }
    Ok(())
}

fn validate_replay_primary(
    outputs: &[u8],
    primary_digest: &[u8],
    primary_bytes: &[u8],
) -> Result<(), TuneDecisionError> {
    let (count, offset) = parse_list_header(outputs, 3, "Replay.outputs")?;
    if count == 0 {
        return Err(TuneDecisionError::InvalidValue("Replay.outputs"));
    }
    let (primary, _) = parse_record_prefix(&outputs[offset..], "OutputIdentity")?;
    let fields = parse_record_fields(primary, 1..=4, "OutputIdentity")?;
    if fields[0] != [1] || fields[2] != primary_digest || fields[3] != primary_bytes {
        return Err(TuneDecisionError::InvalidValue("Replay.outputs.primary"));
    }
    Ok(())
}

fn domain_hash(domain: &[u8], bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    hasher.finalize().into()
}

fn validate_workload(bytes: &[u8]) -> Result<(), TuneDecisionError> {
    let fields = parse_record_fields(bytes, 1..=8, "Workload")?;
    require_length(fields[0], 32, "Workload.manifestDigest")?;
    require_length(fields[1], 32, "Workload.runnerSnapshotDigest")?;
    parse_u64(fields[2], "Workload.runnerSnapshotBytes")?;
    validate_text_list(fields[3], 64, "Workload.argv")?;
    validate_record_list(fields[4], 16, "Workload.environment", |record| {
        let entry = parse_record_fields(record, 1..=3, "EnvironmentEntry")?;
        parse_text(entry[0], "EnvironmentEntry.name")?;
        parse_u64(entry[1], "EnvironmentEntry.valueBytes")?;
        require_length(entry[2], 32, "EnvironmentEntry.valueDigest")
    })?;
    let timeout = parse_u32(fields[5], "Workload.timeoutMs")?;
    if !(100..=120_000).contains(&timeout) {
        return Err(TuneDecisionError::InvalidValue("Workload.timeoutMs"));
    }
    validate_record_list(fields[6], 64, "Workload.inputs", validate_input_identity)?;
    let mut roles = 0u8;
    validate_record_list(fields[7], 16, "Workload.cases", |record| {
        let fields = parse_record_fields(record, 1..=5, "CaseIdentity")?;
        parse_text(fields[0], "CaseIdentity.id")?;
        let role = parse_enum(fields[1], 1..=2, "CaseIdentity.role")?;
        roles |= 1 << (role - 1);
        parse_u64(fields[2], "CaseIdentity.seed")?;
        if parse_u32(fields[3], "CaseIdentity.weight")? == 0 {
            return Err(TuneDecisionError::InvalidValue("CaseIdentity.weight"));
        }
        require_length(fields[4], 32, "CaseIdentity.expectedDigest")
    })?;
    if roles != 0b11 {
        return Err(TuneDecisionError::InvalidValue("Workload.cases roles"));
    }
    Ok(())
}

fn validate_input_identity(bytes: &[u8]) -> Result<(), TuneDecisionError> {
    let fields = parse_record_fields(bytes, 1..=3, "InputIdentity")?;
    let path = parse_text(fields[0], "InputIdentity.logicalPath")?;
    if path.is_empty()
        || path
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(TuneDecisionError::InvalidValue("InputIdentity.logicalPath"));
    }
    require_length(fields[1], 32, "InputIdentity.digest")?;
    if parse_u64(fields[2], "InputIdentity.bytes")? > 1 << 30 {
        return Err(TuneDecisionError::ResourceLimit("InputIdentity.bytes"));
    }
    Ok(())
}

fn validate_environment(bytes: &[u8]) -> Result<(), TuneDecisionError> {
    let fields = parse_record_fields(bytes, 1..=19, "Environment")?;
    for field in &fields[..9] {
        parse_text(field, "Environment text")?;
    }
    validate_text_list(fields[9], 256, "Environment.cpuFeatures")?;
    validate_optional(fields[10], "Environment.physicalCores", |value| {
        parse_u32(value, "Environment.physicalCores").map(|_| ())
    })?;
    if parse_u32(fields[11], "Environment.logicalCores")? == 0 {
        return Err(TuneDecisionError::InvalidValue("Environment.logicalCores"));
    }
    validate_optional(fields[12], "Environment.numaNodes", |value| {
        parse_u32(value, "Environment.numaNodes").map(|_| ())
    })?;
    parse_text(fields[13], "Environment.timerKind")?;
    if parse_u64(fields[14], "Environment.timerResolutionNs")? == 0 {
        return Err(TuneDecisionError::InvalidValue(
            "Environment.timerResolutionNs",
        ));
    }
    parse_text(fields[15], "Environment.schedulingPolicy")?;
    validate_record_list(fields[16], 16, "Environment.calibrations", |record| {
        let fields = parse_record_fields(record, 1..=6, "Calibration")?;
        parse_text(fields[0], "Calibration.caseId")?;
        if parse_u64(fields[1], "Calibration.iterations")? == 0
            || parse_u32(fields[2], "Calibration.attempts")? == 0
            || parse_u64(fields[3], "Calibration.acceptedElapsedNs")? == 0
            || parse_u64(fields[4], "Calibration.confirmationElapsedNs")? == 0
        {
            return Err(TuneDecisionError::InvalidValue(
                "Calibration positive value",
            ));
        }
        parse_bool(fields[5], "Calibration.overshoot").map(|_| ())
    })?;
    require_length(fields[17], 32, "Environment.sessionDigest")?;
    require_length(fields[18], 32, "Environment.measurementCacheSaltDigest")?;
    Ok(())
}

fn validate_frontier(bytes: &[u8]) -> Result<(), TuneDecisionError> {
    let fields = parse_record_fields(bytes, 1..=4, "Frontier")?;
    require_length(fields[0], 32, "Frontier.candidateSpaceDigest")?;
    validate_record_list(fields[1], 4_096, "Frontier.sites", validate_site)?;
    validate_record_list(fields[2], 64, "Frontier.units", validate_unit)?;
    validate_record_list(fields[3], 16_384, "Frontier.expansions", validate_expansion)?;
    Ok(())
}

fn validate_site(bytes: &[u8]) -> Result<(), TuneDecisionError> {
    let fields = parse_record_fields(bytes, 1..=6, "Site")?;
    require_length(fields[0], 32, "Site.siteId")?;
    parse_enum(fields[1], 1..=7, "Site.class")?;
    require_length(fields[2], 32, "Site.rootId")?;
    require_length(fields[3], 32, "Site.preStateDigest")?;
    parse_u32(fields[4], "Site.canonicalRank")?;
    validate_record_value(fields[5], "RootAnchor", validate_root_anchor)
}

fn validate_root_anchor(bytes: &[u8]) -> Result<(), TuneDecisionError> {
    let fields = parse_record_fields(bytes, 1..=3, "RootAnchor")?;
    parse_text(fields[0], "RootAnchor.functionSymbol")?;
    parse_enum(fields[1], 1..=6, "RootAnchor.kind")?;
    parse_u32(fields[2], "RootAnchor.preorderOrdinal")?;
    Ok(())
}

fn validate_unit(bytes: &[u8]) -> Result<(), TuneDecisionError> {
    let fields = parse_record_fields(bytes, 1..=4, "Unit")?;
    require_length(fields[0], 32, "Unit.unitId")?;
    validate_fixed_list(fields[1], 4_096, 32, "Unit.siteIds")?;
    require_length(fields[2], 32, "Unit.baselineStateDigest")?;
    let count = validate_record_list(fields[3], 4, "Unit.variants", validate_unit_variant)?;
    if count == 0 {
        return Err(TuneDecisionError::InvalidValue("Unit.variants"));
    }
    Ok(())
}

fn validate_unit_variant(bytes: &[u8]) -> Result<(), TuneDecisionError> {
    let fields = parse_record_fields(bytes, 1..=7, "UnitVariant")?;
    require_length(fields[0], 32, "UnitVariant.variantId")?;
    parse_enum(fields[1], 1..=7, "UnitVariant.class")?;
    validate_record_list(
        fields[2],
        4_096,
        "UnitVariant.siteAlternatives",
        validate_site_alternative,
    )?;
    for field in &fields[3..6] {
        parse_u64(field, "UnitVariant estimate")?;
    }
    require_length(fields[6], 32, "UnitVariant.postStateDigest")
}

fn validate_site_alternative(bytes: &[u8]) -> Result<(), TuneDecisionError> {
    let fields = parse_record_fields(bytes, 1..=5, "SiteAlternative")?;
    for field in &fields[..4] {
        require_length(field, 32, "SiteAlternative digest")?;
    }
    validate_record_value(
        fields[4],
        "AlternativePayload",
        validate_alternative_payload,
    )
}

fn validate_alternative_payload(bytes: &[u8]) -> Result<(), TuneDecisionError> {
    let fields = parse_record_fields(bytes, 1..=2, "AlternativePayload")?;
    let class = parse_enum(fields[0], 1..=7, "AlternativePayload.class")?;
    let record = parse_record_envelope(fields[1], "AlternativePayload.value")?;
    match class {
        1 => {
            let fields = parse_record_fields(record, 1..=2, "InliningPayload")?;
            parse_text(fields[0], "InliningPayload.calleeSymbol")?;
            parse_enum(fields[1], 1..=2, "InliningPayload.action")?;
        }
        2 => {
            let fields = parse_record_fields(record, 1..=2, "SpecializationPayload")?;
            validate_record_list(fields[0], 16, "SpecializationPayload.bindings", |record| {
                let fields = parse_record_fields(record, 1..=3, "SpecializationBinding")?;
                parse_u32(fields[0], "SpecializationBinding.argumentOrdinal")?;
                parse_enum(fields[1], 1..=7, "SpecializationBinding.kind")?;
                parse_u128(fields[2], "SpecializationBinding.bits").map(|_| ())
            })?;
            parse_bool(fields[1], "SpecializationPayload.guarded")?;
        }
        3 => {
            let fields = parse_record_fields(record, 1..=1, "UnrollingPayload")?;
            let factor = parse_u32(fields[0], "UnrollingPayload.factor")?;
            if !(2..=64).contains(&factor) || !factor.is_power_of_two() {
                return Err(TuneDecisionError::InvalidValue("UnrollingPayload.factor"));
            }
        }
        4 => validate_simd_payload(record, "LoopSimdPayload", false)?,
        5 => {
            let fields = parse_record_fields(record, 1..=2, "SlpPayload")?;
            let width = parse_u32(fields[0], "SlpPayload.packWidth")?;
            if !(2..=64).contains(&width) {
                return Err(TuneDecisionError::InvalidValue("SlpPayload.packWidth"));
            }
            validate_record_list(
                fields[1],
                64,
                "SlpPayload.operandAnchors",
                validate_root_anchor,
            )?;
        }
        6 => validate_simd_payload(record, "ShortSliceVersioningPayload", true)?,
        7 => {
            let fields = parse_record_fields(record, 1..=2, "LayoutPayload")?;
            parse_enum(fields[0], 1..=3, "LayoutPayload.scope")?;
            validate_fixed_list(fields[1], 4_096, 32, "LayoutPayload.rootOrder")?;
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn validate_simd_payload(
    bytes: &[u8],
    record: &'static str,
    short_slice: bool,
) -> Result<(), TuneDecisionError> {
    let fields = parse_record_fields(bytes, 1..=3, record)?;
    let vector_index = usize::from(short_slice);
    parse_u32(fields[0], record)?;
    let vector_bits = parse_u32(fields[vector_index], record)?;
    let interleave = parse_u32(fields[vector_index + 1], record)?;
    if !(64..=2_048).contains(&vector_bits)
        || !vector_bits.is_power_of_two()
        || !(1..=8).contains(&interleave)
    {
        return Err(TuneDecisionError::InvalidValue(record));
    }
    Ok(())
}

fn validate_expansion(bytes: &[u8]) -> Result<(), TuneDecisionError> {
    let fields = parse_record_fields(bytes, 1..=10, "Expansion")?;
    parse_u32(fields[0], "Expansion.ordinal")?;
    for field in &fields[1..4] {
        require_length(field, 32, "Expansion digest")?;
    }
    parse_enum(fields[4], 1..=4, "Expansion.disposition")?;
    validate_optional(fields[5], "Expansion.resultPlanDigest", |value| {
        require_length(value, 32, "Expansion.resultPlanDigest")
    })?;
    parse_u16(fields[6], "Expansion.diagnosticCode")?;
    for field in &fields[7..10] {
        validate_optional(field, "Expansion metric", |value| {
            parse_u64(value, "Expansion metric").map(|_| ())
        })?;
    }
    Ok(())
}

fn validate_candidates(bytes: &[u8]) -> Result<(), TuneDecisionError> {
    let fields = parse_record_fields(bytes, 1..=2, "Candidates")?;
    validate_record_value(fields[0], "Candidate", validate_candidate)?;
    validate_record_list(fields[1], 32, "Candidates.trials", validate_candidate)?;
    Ok(())
}

fn validate_candidate(bytes: &[u8]) -> Result<(), TuneDecisionError> {
    let fields = parse_record_fields(bytes, 1..=12, "Candidate")?;
    require_length(fields[0], 32, "Candidate.planDigest")?;
    validate_record_list(fields[1], 64, "Candidate.choices", |record| {
        let fields = parse_record_fields(record, 1..=5, "PlanChoice")?;
        require_length(fields[0], 32, "PlanChoice.unitId")?;
        require_length(fields[1], 32, "PlanChoice.variantId")?;
        parse_enum(fields[2], 1..=7, "PlanChoice.class")?;
        require_length(fields[3], 32, "PlanChoice.preStateDigest")?;
        require_length(fields[4], 32, "PlanChoice.postStateDigest")
    })?;
    require_length(fields[2], 32, "Candidate.objectGraphDigest")?;
    require_length(fields[3], 32, "Candidate.linkRecipeDigest")?;
    parse_u64(fields[4], "Candidate.primaryArtifactBytes")?;
    parse_enum(fields[5], 1..=8, "Candidate.outcome")?;
    parse_u16(fields[6], "Candidate.diagnosticCode")?;
    validate_optional(fields[7], "Candidate.correctnessDigest", |value| {
        require_length(value, 32, "Candidate.correctnessDigest")
    })?;
    validate_record_list(
        fields[8],
        48,
        "Candidate.streams",
        validate_measurement_stream,
    )?;
    validate_record_value(fields[9], "CacheOrigin", validate_cache_origin)?;
    validate_optional(fields[10], "Candidate.timeout", |value| {
        validate_record_value(value, "TimeoutRecord", validate_timeout)
    })?;
    require_length(fields[11], 32, "Candidate.primaryArtifactDigest")?;
    Ok(())
}

fn validate_measurement_stream(bytes: &[u8]) -> Result<(), TuneDecisionError> {
    let fields = parse_record_fields(bytes, 1..=7, "MeasurementStream")?;
    let phase = parse_enum(fields[0], 1..=7, "MeasurementStream.phase")?;
    if !matches!(phase, 3 | 5 | 7) {
        return Err(TuneDecisionError::InvalidValue("MeasurementStream.phase"));
    }
    parse_enum(fields[1], 0..=2, "MeasurementStream.round")?;
    parse_text(fields[2], "MeasurementStream.caseId")?;
    require_length(fields[3], 32, "MeasurementStream.planDigest")?;
    if parse_u64(fields[4], "MeasurementStream.iterations")? == 0 {
        return Err(TuneDecisionError::InvalidValue(
            "MeasurementStream.iterations",
        ));
    }
    let count = validate_record_list(
        fields[5],
        20,
        "MeasurementStream.rows",
        validate_measurement_row,
    )?;
    if count != 20 {
        return Err(TuneDecisionError::InvalidValue("MeasurementStream.rows"));
    }
    require_length(fields[6], 32, "MeasurementStream.correctnessDigest")?;
    Ok(())
}

fn validate_measurement_row(bytes: &[u8]) -> Result<(), TuneDecisionError> {
    let fields = parse_record_fields(bytes, 1..=4, "MeasurementRow")?;
    let ordinal = parse_u32(fields[0], "MeasurementRow.ordinal")?;
    if ordinal >= 20 {
        return Err(TuneDecisionError::InvalidValue("MeasurementRow.ordinal"));
    }
    require_length(fields[1], 32, "MeasurementRow.permutationKey")?;
    let calls = parse_u64_list(fields[2], 3, "MeasurementRow.callsNs")?;
    if calls.len() != 3 || calls.contains(&0) {
        return Err(TuneDecisionError::InvalidValue("MeasurementRow.callsNs"));
    }
    if parse_u64(fields[3], "MeasurementRow.storedMinimumNs")?
        != *calls
            .iter()
            .min()
            .ok_or(TuneDecisionError::InvalidValue("MeasurementRow.callsNs"))?
    {
        return Err(TuneDecisionError::InvalidValue(
            "MeasurementRow.storedMinimumNs",
        ));
    }
    Ok(())
}

fn validate_cache_origin(bytes: &[u8]) -> Result<(), TuneDecisionError> {
    let fields = parse_record_fields(bytes, 1..=3, "CacheOrigin")?;
    parse_enum(fields[0], 1..=2, "CacheOrigin.kind")?;
    require_length(fields[1], 32, "CacheOrigin.keyDigest")?;
    require_length(fields[2], 32, "CacheOrigin.entryDigest")
}

fn validate_timeout(bytes: &[u8]) -> Result<(), TuneDecisionError> {
    let fields = parse_record_fields(bytes, 1..=6, "TimeoutRecord")?;
    parse_enum(fields[0], 1..=7, "TimeoutRecord.phase")?;
    parse_enum(fields[1], 0..=2, "TimeoutRecord.round")?;
    parse_u32(fields[2], "TimeoutRecord.row")?;
    parse_text(fields[3], "TimeoutRecord.caseId")?;
    parse_enum(fields[4], 1..=3, "TimeoutRecord.call")?;
    parse_u64(fields[5], "TimeoutRecord.elapsedNs")?;
    Ok(())
}

fn validate_selection(bytes: &[u8]) -> Result<(), TuneDecisionError> {
    let fields = parse_record_fields(bytes, 1..=5, "Selection")?;
    validate_record_value(fields[0], "RoundSummary", validate_round_summary)?;
    validate_record_value(fields[1], "RoundSummary", validate_round_summary)?;
    require_length(fields[2], 32, "Selection.selectedPlanDigest")?;
    let reason = parse_enum(fields[3], 1..=4, "Selection.reason")?;
    let present = validate_optional(fields[4], "Selection.certificate", |value| {
        validate_record_value(value, "Certificate", validate_certificate)
    })?;
    if present != (reason == 1) {
        return Err(TuneDecisionError::InvalidValue("Selection.certificate"));
    }
    Ok(())
}

fn validate_round_summary(bytes: &[u8]) -> Result<(), TuneDecisionError> {
    let fields = parse_record_fields(bytes, 1..=3, "RoundSummary")?;
    parse_enum(fields[0], 1..=2, "RoundSummary.round")?;
    validate_record_list(fields[1], 4, "RoundSummary.plans", validate_round_plan)?;
    validate_fixed_list(fields[2], 4, 32, "RoundSummary.rankedPlanDigests")?;
    Ok(())
}

fn validate_round_plan(bytes: &[u8]) -> Result<(), TuneDecisionError> {
    let fields = parse_record_fields(bytes, 1..=6, "RoundPlan")?;
    require_length(fields[0], 32, "RoundPlan.planDigest")?;
    validate_record_list(fields[1], 16, "RoundPlan.caseMedians", |record| {
        let fields = parse_record_fields(record, 1..=4, "CaseMedian")?;
        parse_text(fields[0], "CaseMedian.caseId")?;
        parse_u64(fields[1], "CaseMedian.baselineNs")?;
        parse_u64(fields[2], "CaseMedian.candidateNs")?;
        parse_u64(fields[3], "CaseMedian.ratioQ32").map(|_| ())
    })?;
    parse_u64(fields[2], "RoundPlan.aggregateRatioQ32")?;
    parse_bool(fields[3], "RoundPlan.stable")?;
    parse_bool(fields[4], "RoundPlan.thresholdPassed")?;
    parse_u32(fields[5], "RoundPlan.pairedWins")?;
    Ok(())
}

fn validate_certificate(bytes: &[u8]) -> Result<(), TuneDecisionError> {
    let fields = parse_record_fields(bytes, 1..=8, "Certificate")?;
    for field in fields {
        require_length(field, 32, "Certificate digest")?;
    }
    Ok(())
}

fn validate_replay(bytes: &[u8]) -> Result<(), TuneDecisionError> {
    let fields = parse_record_fields(bytes, 1..=10, "Replay")?;
    for field in &fields[..5] {
        require_length(field, 32, "Replay digest")?;
    }
    let outputs = validate_record_list(fields[5], 3, "Replay.outputs", |record| {
        let fields = parse_record_fields(record, 1..=4, "OutputIdentity")?;
        parse_enum(fields[0], 1..=3, "OutputIdentity.role")?;
        parse_text(fields[1], "OutputIdentity.logicalBasename")?;
        require_length(fields[2], 32, "OutputIdentity.contentDigest")?;
        parse_u64(fields[3], "OutputIdentity.contentBytes").map(|_| ())
    })?;
    if outputs == 0 {
        return Err(TuneDecisionError::InvalidValue("Replay.outputs"));
    }
    validate_record_value(fields[6], "CacheOrigin", validate_cache_origin)?;
    validate_record_value(fields[7], "CacheOrigin", validate_cache_origin)?;
    require_length(fields[8], 32, "Replay.replayResultDigest")?;
    require_length(fields[9], 32, "Replay.choiceIdentityDigest")?;
    Ok(())
}

fn validate_contract(bytes: &[u8]) -> Result<(), TuneDecisionError> {
    let fields = parse_record_fields(bytes, 1..=32, "Contract")?;
    for (index, field) in fields.iter().take(5).enumerate() {
        if parse_u32(field, "Contract schema")? != 1 {
            return Err(TuneDecisionError::InvalidValue(match index {
                0 => "Contract.formatSchema",
                1 => "Contract.contractSchema",
                2 => "Contract.measurementSchema",
                3 => "Contract.inspectionSchema",
                _ => "Contract.planSchema",
            }));
        }
    }
    let budget = *fields[5]
        .first()
        .filter(|_| fields[5].len() == 1)
        .ok_or(TuneDecisionError::InvalidValue("Contract.budget"))?;
    let expected = match budget {
        1 => (4, 1_024, 8, 4, 2, 600_000),
        2 => (8, 4_096, 16, 8, 3, 1_800_000),
        3 => (16, 16_384, 32, 16, 4, 7_200_000),
        _ => return Err(TuneDecisionError::InvalidValue("Contract.budget")),
    };
    let actual = (
        parse_u32(fields[6], "Contract.beamWidth")?,
        parse_u32(fields[7], "Contract.expansionLimit")?,
        parse_u32(fields[8], "Contract.compileAttemptLimit")?,
        parse_u32(fields[9], "Contract.measuredFinalistLimit")?,
        parse_u32(fields[10], "Contract.validationEntrantLimit")?,
        parse_u64(fields[11], "Contract.wallClockMs")?,
    );
    if actual != expected {
        return Err(TuneDecisionError::InvalidValue("Contract budget preset"));
    }
    let fixed_u32 = [
        11, 10, 32, 3, 20, 3, 2_250, 4, 5, 6, 5, 16, 97, 100, 102, 100, 16,
    ];
    for ((index, field), expected) in fields[12..14]
        .iter()
        .chain(fields[16..31].iter())
        .enumerate()
        .zip(fixed_u32)
    {
        if parse_u32(field, "Contract fixed value")? != expected {
            return Err(TuneDecisionError::InvalidValue(match index {
                0 => "Contract.artifactRatioNumerator",
                1 => "Contract.artifactRatioDenominator",
                _ => "Contract fixed value",
            }));
        }
    }
    if parse_u64(fields[14], "Contract.calibrationMinimumNs")? != 50_000_000
        || parse_u64(fields[15], "Contract.calibrationPreferredMaximumNs")? != 250_000_000
    {
        return Err(TuneDecisionError::InvalidValue(
            "Contract calibration bounds",
        ));
    }
    let policy_prefix_len = bytes
        .len()
        .checked_sub(6 + 32)
        .ok_or(TuneDecisionError::Truncated("Contract.policyDigest"))?;
    let mut hasher = Sha256::new();
    hasher.update(POLICY_DIGEST_DOMAIN);
    hasher.update(
        u32::try_from(policy_prefix_len)
            .map_err(|_| TuneDecisionError::ResourceLimit("Contract"))?
            .to_be_bytes(),
    );
    hasher.update(&bytes[..policy_prefix_len]);
    if hasher.finalize().as_slice() != fields[31] {
        return Err(TuneDecisionError::DigestMismatch);
    }
    Ok(())
}

fn validate_identity(bytes: &[u8]) -> Result<(), TuneDecisionError> {
    let fields = parse_record_fields(bytes, 1..=22, "Identity")?;
    for (index, field) in fields.iter().enumerate() {
        match index + 1 {
            1 | 3 | 4 => {
                parse_text(field, "Identity text")?;
            }
            2 | 5 | 16..=19 => require_length(field, 32, "Identity digest")?,
            6..=15 => {
                parse_u32(field, "Identity schema")?;
            }
            20 => match field {
                [1 | 2] => {}
                _ => return Err(TuneDecisionError::InvalidValue("Identity.outputKind")),
            },
            21 => validate_target_identity(field)?,
            22 => validate_optional_profile(field)?,
            _ => return Err(TuneDecisionError::InvalidValue("Identity tag")),
        }
    }
    Ok(())
}

fn validate_target_identity(bytes: &[u8]) -> Result<(), TuneDecisionError> {
    let record = parse_record_envelope(bytes, "TargetIdentity")?;
    let fields = parse_record_fields(record, 1..=4, "TargetIdentity")?;
    parse_text(fields[0], "TargetIdentity.triple")?;
    parse_text(fields[1], "TargetIdentity.cpu")?;
    validate_text_list(fields[2], 256, "TargetIdentity.features")?;
    parse_text(fields[3], "TargetIdentity.targetProfile")?;
    Ok(())
}

fn validate_optional_profile(bytes: &[u8]) -> Result<(), TuneDecisionError> {
    let Some((&present, payload)) = bytes.split_first() else {
        return Err(TuneDecisionError::Truncated("Identity.profile"));
    };
    match present {
        0 if payload.is_empty() => Ok(()),
        1 => {
            let record = parse_record_envelope(payload, "ProfileIdentity")?;
            let fields = parse_record_fields(record, 1..=6, "ProfileIdentity")?;
            parse_u32(fields[0], "ProfileIdentity.formatSchema")?;
            require_length(fields[1], 32, "ProfileIdentity.compilerSource")?;
            require_length(fields[2], 32, "ProfileIdentity.sourceDigest")?;
            require_length(fields[3], 32, "ProfileIdentity.topologyDigest")?;
            require_length(fields[4], 32, "ProfileIdentity.contentDigest")?;
            parse_u64(fields[5], "ProfileIdentity.contentBytes")?;
            Ok(())
        }
        _ => Err(TuneDecisionError::InvalidValue("Identity.profile")),
    }
}

fn parse_record_fields<'a>(
    bytes: &'a [u8],
    expected: std::ops::RangeInclusive<u16>,
    record: &'static str,
) -> Result<Vec<&'a [u8]>, TuneDecisionError> {
    require_first_field(bytes, record, *expected.start())?;
    parse_fields(bytes, expected, record)
}

fn parse_record_envelope<'a>(
    bytes: &'a [u8],
    context: &'static str,
) -> Result<&'a [u8], TuneDecisionError> {
    let header = bytes
        .get(..4)
        .ok_or(TuneDecisionError::Truncated(context))?;
    let length = usize::try_from(u32::from_be_bytes([
        header[0], header[1], header[2], header[3],
    ]))
    .map_err(|_| TuneDecisionError::ResourceLimit(context))?;
    let end = 4usize
        .checked_add(length)
        .ok_or(TuneDecisionError::ResourceLimit(context))?;
    if end != bytes.len() {
        return Err(TuneDecisionError::InvalidValue(context));
    }
    bytes
        .get(4..end)
        .ok_or(TuneDecisionError::Truncated(context))
}

fn validate_text_list(
    bytes: &[u8],
    limit: u32,
    context: &'static str,
) -> Result<(), TuneDecisionError> {
    let header = bytes
        .get(..4)
        .ok_or(TuneDecisionError::Truncated(context))?;
    let count = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
    if count > limit {
        return Err(TuneDecisionError::ResourceLimit(context));
    }
    let mut offset = 4usize;
    let mut previous: Option<&str> = None;
    for _ in 0..count {
        let (value, consumed) = parse_text_prefix(&bytes[offset..], context)?;
        if previous.is_some_and(|prior| prior.as_bytes() >= value.as_bytes()) {
            return Err(TuneDecisionError::NonCanonicalOrder(context));
        }
        previous = Some(value);
        offset = offset
            .checked_add(consumed)
            .ok_or(TuneDecisionError::ResourceLimit(context))?;
    }
    if offset != bytes.len() {
        return Err(TuneDecisionError::InvalidValue(context));
    }
    Ok(())
}

fn validate_record_value(
    bytes: &[u8],
    context: &'static str,
    validate: impl FnOnce(&[u8]) -> Result<(), TuneDecisionError>,
) -> Result<(), TuneDecisionError> {
    validate(parse_record_envelope(bytes, context)?)
}

fn validate_record_list(
    bytes: &[u8],
    limit: u32,
    context: &'static str,
    mut validate: impl FnMut(&[u8]) -> Result<(), TuneDecisionError>,
) -> Result<u32, TuneDecisionError> {
    let (count, mut offset) = parse_list_header(bytes, limit, context)?;
    for _ in 0..count {
        let (record, consumed) = parse_record_prefix(
            bytes
                .get(offset..)
                .ok_or(TuneDecisionError::Truncated(context))?,
            context,
        )?;
        validate(record)?;
        offset = offset
            .checked_add(consumed)
            .ok_or(TuneDecisionError::ResourceLimit(context))?;
    }
    require_exact_end(bytes, offset, context)?;
    Ok(count)
}

fn validate_fixed_list(
    bytes: &[u8],
    limit: u32,
    item_length: usize,
    context: &'static str,
) -> Result<u32, TuneDecisionError> {
    let (count, offset) = parse_list_header(bytes, limit, context)?;
    let items_length = usize::try_from(count)
        .map_err(|_| TuneDecisionError::ResourceLimit(context))?
        .checked_mul(item_length)
        .ok_or(TuneDecisionError::ResourceLimit(context))?;
    let expected = offset
        .checked_add(items_length)
        .ok_or(TuneDecisionError::ResourceLimit(context))?;
    require_exact_end(bytes, expected, context)?;
    Ok(count)
}

fn parse_u64_list(
    bytes: &[u8],
    limit: u32,
    context: &'static str,
) -> Result<Vec<u64>, TuneDecisionError> {
    let (count, offset) = parse_list_header(bytes, limit, context)?;
    let items_length = usize::try_from(count)
        .map_err(|_| TuneDecisionError::ResourceLimit(context))?
        .checked_mul(8)
        .ok_or(TuneDecisionError::ResourceLimit(context))?;
    let expected = offset
        .checked_add(items_length)
        .ok_or(TuneDecisionError::ResourceLimit(context))?;
    require_exact_end(bytes, expected, context)?;
    let mut values = Vec::with_capacity(usize::try_from(count).unwrap_or(0));
    for item in bytes[offset..].chunks_exact(8) {
        values.push(parse_u64(item, context)?);
    }
    Ok(values)
}

fn validate_optional(
    bytes: &[u8],
    context: &'static str,
    validate: impl FnOnce(&[u8]) -> Result<(), TuneDecisionError>,
) -> Result<bool, TuneDecisionError> {
    let Some((&present, payload)) = bytes.split_first() else {
        return Err(TuneDecisionError::Truncated(context));
    };
    match present {
        0 if payload.is_empty() => Ok(false),
        1 => {
            validate(payload)?;
            Ok(true)
        }
        _ => Err(TuneDecisionError::InvalidValue(context)),
    }
}

fn parse_list_header(
    bytes: &[u8],
    limit: u32,
    context: &'static str,
) -> Result<(u32, usize), TuneDecisionError> {
    let header = bytes
        .get(..4)
        .ok_or(TuneDecisionError::Truncated(context))?;
    let count = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
    if count > limit {
        return Err(TuneDecisionError::ResourceLimit(context));
    }
    Ok((count, 4))
}

fn parse_record_prefix<'a>(
    bytes: &'a [u8],
    context: &'static str,
) -> Result<(&'a [u8], usize), TuneDecisionError> {
    let header = bytes
        .get(..4)
        .ok_or(TuneDecisionError::Truncated(context))?;
    let length = usize::try_from(u32::from_be_bytes([
        header[0], header[1], header[2], header[3],
    ]))
    .map_err(|_| TuneDecisionError::ResourceLimit(context))?;
    let end = 4usize
        .checked_add(length)
        .ok_or(TuneDecisionError::ResourceLimit(context))?;
    let record = bytes
        .get(4..end)
        .ok_or(TuneDecisionError::Truncated(context))?;
    Ok((record, end))
}

fn require_exact_end(
    bytes: &[u8],
    offset: usize,
    context: &'static str,
) -> Result<(), TuneDecisionError> {
    if offset != bytes.len() {
        return Err(TuneDecisionError::InvalidValue(context));
    }
    Ok(())
}

fn parse_text<'a>(bytes: &'a [u8], context: &'static str) -> Result<&'a str, TuneDecisionError> {
    let (value, consumed) = parse_text_prefix(bytes, context)?;
    if consumed != bytes.len() {
        return Err(TuneDecisionError::InvalidValue(context));
    }
    Ok(value)
}

fn parse_text_prefix<'a>(
    bytes: &'a [u8],
    context: &'static str,
) -> Result<(&'a str, usize), TuneDecisionError> {
    let header = bytes
        .get(..4)
        .ok_or(TuneDecisionError::Truncated(context))?;
    let length = usize::try_from(u32::from_be_bytes([
        header[0], header[1], header[2], header[3],
    ]))
    .map_err(|_| TuneDecisionError::ResourceLimit(context))?;
    if length > 4_096 {
        return Err(TuneDecisionError::ResourceLimit(context));
    }
    let end = 4usize
        .checked_add(length)
        .ok_or(TuneDecisionError::ResourceLimit(context))?;
    let raw = bytes
        .get(4..end)
        .ok_or(TuneDecisionError::Truncated(context))?;
    if raw.contains(&0) {
        return Err(TuneDecisionError::InvalidValue(context));
    }
    let value = std::str::from_utf8(raw).map_err(|_| TuneDecisionError::InvalidUtf8)?;
    if value.nfc().ne(value.chars()) || is_absolute_text(value) {
        return Err(TuneDecisionError::InvalidValue(context));
    }
    Ok((value, end))
}

fn is_absolute_text(value: &str) -> bool {
    value.starts_with('/')
        || value.starts_with("\\\\")
        || value.as_bytes().get(..3).is_some_and(|prefix| {
            prefix[0].is_ascii_alphabetic()
                && prefix[1] == b':'
                && matches!(prefix[2], b'/' | b'\\')
        })
}

fn parse_u32(bytes: &[u8], context: &'static str) -> Result<u32, TuneDecisionError> {
    let value: [u8; 4] = bytes
        .try_into()
        .map_err(|_| TuneDecisionError::InvalidValue(context))?;
    Ok(u32::from_be_bytes(value))
}

fn parse_u16(bytes: &[u8], context: &'static str) -> Result<u16, TuneDecisionError> {
    let value: [u8; 2] = bytes
        .try_into()
        .map_err(|_| TuneDecisionError::InvalidValue(context))?;
    Ok(u16::from_be_bytes(value))
}

fn parse_u128(bytes: &[u8], context: &'static str) -> Result<u128, TuneDecisionError> {
    let value: [u8; 16] = bytes
        .try_into()
        .map_err(|_| TuneDecisionError::InvalidValue(context))?;
    Ok(u128::from_be_bytes(value))
}

fn parse_u64(bytes: &[u8], context: &'static str) -> Result<u64, TuneDecisionError> {
    let value: [u8; 8] = bytes
        .try_into()
        .map_err(|_| TuneDecisionError::InvalidValue(context))?;
    Ok(u64::from_be_bytes(value))
}

fn parse_bool(bytes: &[u8], context: &'static str) -> Result<bool, TuneDecisionError> {
    match bytes {
        [0] => Ok(false),
        [1] => Ok(true),
        _ => Err(TuneDecisionError::InvalidValue(context)),
    }
}

fn parse_enum(
    bytes: &[u8],
    range: std::ops::RangeInclusive<u8>,
    context: &'static str,
) -> Result<u8, TuneDecisionError> {
    match bytes {
        [value] if range.contains(value) => Ok(*value),
        _ => Err(TuneDecisionError::InvalidValue(context)),
    }
}

fn require_length(
    bytes: &[u8],
    length: usize,
    context: &'static str,
) -> Result<(), TuneDecisionError> {
    if bytes.len() != length {
        return Err(TuneDecisionError::InvalidValue(context));
    }
    Ok(())
}

fn parse_fields<'a>(
    bytes: &'a [u8],
    expected: std::ops::RangeInclusive<u16>,
    context: &'static str,
) -> Result<Vec<&'a [u8]>, TuneDecisionError> {
    let expected_count = usize::from(*expected.end() - *expected.start() + 1);
    let mut fields = Vec::with_capacity(expected_count);
    let mut offset = 0usize;
    for tag in expected {
        let header_end = offset
            .checked_add(6)
            .ok_or(TuneDecisionError::ResourceLimit(context))?;
        let header = bytes
            .get(offset..header_end)
            .ok_or(TuneDecisionError::MissingField {
                record: context,
                tag,
            })?;
        let actual_tag = u16::from_be_bytes([header[0], header[1]]);
        if actual_tag != tag {
            return Err(TuneDecisionError::NonCanonicalOrder(context));
        }
        let length = usize::try_from(u32::from_be_bytes([
            header[2], header[3], header[4], header[5],
        ]))
        .map_err(|_| TuneDecisionError::ResourceLimit(context))?;
        let payload_end = header_end
            .checked_add(length)
            .ok_or(TuneDecisionError::ResourceLimit(context))?;
        fields.push(
            bytes
                .get(header_end..payload_end)
                .ok_or(TuneDecisionError::Truncated(context))?,
        );
        offset = payload_end;
    }
    if offset != bytes.len() {
        return Err(TuneDecisionError::NonCanonicalOrder(context));
    }
    Ok(fields)
}

fn require_first_field(
    bytes: &[u8],
    record: &'static str,
    tag: u16,
) -> Result<(), TuneDecisionError> {
    if bytes.len() < 6 {
        return Err(TuneDecisionError::MissingField { record, tag });
    }
    Ok(())
}
