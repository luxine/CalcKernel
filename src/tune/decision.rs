use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use super::schema::{
    DECISION_DIGEST_DOMAIN, MAX_TUNE_DECISION_BYTES, PLAN_DIGEST_DOMAIN, POLICY_DIGEST_DOMAIN,
    TUNE_DECISION_MAGIC, TUNE_DECISION_SCHEMA,
};

const HEADER_BYTES: usize = 12;
const DIGEST_BYTES: usize = 32;

/// One structurally validated CK 0.14 tuning decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuneDecision {
    bytes: Vec<u8>,
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

    let replay = parse_record_fields(fields[7], 1..=10, "Replay")?;
    if replay[3] != selected_candidate[2] || replay[4] != selected_candidate[3] {
        return Err(TuneDecisionError::InvalidValue("Replay artifact identity"));
    }
    if replay[6] != selected_candidate[9] {
        return Err(TuneDecisionError::InvalidValue("Replay.compileOrigin"));
    }
    validate_replay_primary(replay[5], selected_candidate[11], selected_candidate[4])?;
    Ok(())
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
