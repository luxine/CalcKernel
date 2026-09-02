use sha2::{Digest, Sha256};

const SESSION_DIGEST_DOMAIN: &[u8] = b"CK-TUNE-SESSION\0";

/// Measurement-independent identity of the baseline artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BaselineSessionSeed {
    pub plan_digest: [u8; 32],
    pub object_graph_digest: [u8; 32],
    pub link_recipe_digest: [u8; 32],
    pub primary_artifact_bytes: u64,
}

/// The six canonical records from which a schema-1 measurement session is derived.
///
/// The first five byte slices are complete canonical record envelopes, not record
/// payloads. Keeping calibration, measurements, paths, cache origin, and publication
/// destinations out of this type makes accidental order-seed contamination impossible.
#[derive(Debug, Clone, Copy)]
pub struct SessionDigestMaterial<'a> {
    pub identity_record: &'a [u8],
    pub contract_record: &'a [u8],
    pub workload_record: &'a [u8],
    pub environment_seed_record: &'a [u8],
    pub frontier_record: &'a [u8],
    pub baseline: BaselineSessionSeed,
}

/// Derives the canonical schema-1 measurement-order seed.
pub fn derive_session_digest(material: &SessionDigestMaterial<'_>) -> Result<[u8; 32], String> {
    for value in [
        material.identity_record,
        material.contract_record,
        material.workload_record,
        material.environment_seed_record,
        material.frontier_record,
    ] {
        validate_record_envelope(value)?;
    }
    let mut baseline_fields = Vec::new();
    push_field(&mut baseline_fields, 1, &material.baseline.plan_digest)?;
    push_field(
        &mut baseline_fields,
        2,
        &material.baseline.object_graph_digest,
    )?;
    push_field(
        &mut baseline_fields,
        3,
        &material.baseline.link_recipe_digest,
    )?;
    push_field(
        &mut baseline_fields,
        4,
        &material.baseline.primary_artifact_bytes.to_be_bytes(),
    )?;
    let baseline_record = record(&baseline_fields)?;

    let mut fields = Vec::new();
    for (tag, value) in [
        (1, material.identity_record),
        (2, material.contract_record),
        (3, material.workload_record),
        (4, material.environment_seed_record),
        (5, material.frontier_record),
        (6, baseline_record.as_slice()),
    ] {
        push_field(&mut fields, tag, value)?;
    }
    let session_record = record(&fields)?;
    let mut hasher = Sha256::new();
    hasher.update(SESSION_DIGEST_DOMAIN);
    hasher.update(session_record);
    Ok(hasher.finalize().into())
}

fn validate_record_envelope(value: &[u8]) -> Result<(), String> {
    let length = value
        .get(..4)
        .ok_or_else(|| "session component is not a canonical record".to_string())?;
    let declared = u32::from_be_bytes(
        length
            .try_into()
            .map_err(|_| "session component length is invalid".to_string())?,
    );
    if usize::try_from(declared).map_err(|_| "session component length overflow".to_string())?
        != value.len() - 4
    {
        return Err("session component is not a canonical record".to_string());
    }
    Ok(())
}

fn push_field(output: &mut Vec<u8>, tag: u16, value: &[u8]) -> Result<(), String> {
    output.extend_from_slice(&tag.to_be_bytes());
    output.extend_from_slice(
        &u32::try_from(value.len())
            .map_err(|_| "session field exceeds schema-1 length".to_string())?
            .to_be_bytes(),
    );
    output.extend_from_slice(value);
    Ok(())
}

fn record(fields: &[u8]) -> Result<Vec<u8>, String> {
    let mut output = u32::try_from(fields.len())
        .map_err(|_| "session record exceeds schema-1 length".to_string())?
        .to_be_bytes()
        .to_vec();
    output.extend_from_slice(fields);
    Ok(output)
}
