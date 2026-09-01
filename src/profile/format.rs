use std::cmp::Ordering;

use sha2::{Digest, Sha256};

use super::CkProfileError;
use super::identity::{
    CK_PROFILE_FORMAT_SCHEMA, CK_PROFILE_MAX_BYTES, CK_PROFILE_MAX_CANDIDATES,
    CK_PROFILE_MAX_SHARDS, CK_PROFILE_MAX_SITES, CkProfileIdentity, decode_identity,
};

const SHARD_MAGIC: &[u8; 8] = b"CKPART01";
const PROFILE_MAGIC: &[u8; 8] = b"CKPROF01";
const OUTER_HEADER_BYTES: usize = 12;
const DIGEST_BYTES: usize = 32;

/// Stable 128-bit index for one full canonical profile site descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CkProfileSiteId(pub [u8; 16]);

/// Closed CK 0.13 profile site families.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CkProfileSiteKind {
    FunctionEntry,
    Edge {
        from_block: u32,
        to_block: u32,
        reconstructed: bool,
    },
    LoopTripHistogram {
        loop_identity: u32,
    },
    SliceLengthHistogram {
        decision_identity: u32,
    },
    CandidateConstant {
        decision_identity: u32,
        candidates: Vec<i64>,
    },
}

/// Full collision-authoritative description of an instrumentation site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkProfileSiteDescriptor {
    pub id: CkProfileSiteId,
    pub function_digest: [u8; 32],
    pub location: u32,
    pub kind: CkProfileSiteKind,
}

/// Counter payload associated with one site descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CkProfileCounter {
    Scalar(u64),
    Histogram {
        buckets: [u64; 16],
        saturated: bool,
    },
    CandidateConstant {
        candidates: Vec<u64>,
        other: u64,
        saturated: bool,
    },
}

impl CkProfileCounter {
    pub(crate) fn is_saturated(&self) -> bool {
        match self {
            Self::Scalar(value) => *value == u64::MAX,
            Self::Histogram { saturated, .. } | Self::CandidateConstant { saturated, .. } => {
                *saturated
            }
        }
    }

    pub(crate) fn is_observed(&self) -> bool {
        match self {
            Self::Scalar(value) => *value != 0,
            Self::Histogram { buckets, .. } => buckets.iter().any(|value| *value != 0),
            Self::CandidateConstant {
                candidates, other, ..
            } => *other != 0 || candidates.iter().any(|value| *value != 0),
        }
    }
}

/// One canonical site/counter association.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkProfileCounterRecord {
    pub site_id: CkProfileSiteId,
    pub counter: CkProfileCounter,
}

/// One completed raw run. Shards are the only schema-1 merge inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkProfileShard {
    pub identity: CkProfileIdentity,
    pub sites: Vec<CkProfileSiteDescriptor>,
    pub counters: Vec<CkProfileCounterRecord>,
    pub run_id: [u8; 16],
    pub overflowed: bool,
    pub incomplete_observations: bool,
}

/// Terminal aggregate profile consumed by Native O2/O3 builds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkProfile {
    pub identity: CkProfileIdentity,
    pub sites: Vec<CkProfileSiteDescriptor>,
    pub counters: Vec<CkProfileCounterRecord>,
    pub completed_runs: u64,
    pub merged_shards: u32,
    pub overflowed: bool,
    pub incomplete_observations: bool,
}

/// Computes the authoritative SHA-256 for a canonical descriptor table.
///
/// # Errors
///
/// Returns an error when descriptors are out of order or exceed schema limits.
pub fn profile_site_table_digest(
    sites: &[CkProfileSiteDescriptor],
) -> Result<[u8; 32], CkProfileError> {
    let bytes = encode_sites(sites)?;
    let mut hasher = Sha256::new();
    hasher.update(b"CK-PROFILE-SITE-TABLE\0");
    hasher.update(bytes);
    Ok(hasher.finalize().into())
}

/// Serializes one completed shard into canonical `CKPART01` bytes.
///
/// # Errors
///
/// Returns an error when identity, descriptor, counter, or resource invariants fail.
pub fn serialize_profile_shard(shard: &CkProfileShard) -> Result<Vec<u8>, CkProfileError> {
    validate_tables(&shard.identity, &shard.sites, &shard.counters)?;
    let identity = shard.identity.canonical_bytes()?;
    let sites = encode_sites(&shard.sites)?;
    let counters = encode_counters(&shard.counters)?;
    let flags = encode_flags(shard.overflowed, shard.incomplete_observations);
    encode_outer(
        SHARD_MAGIC,
        b"CK-PROFILE-SHARD\0",
        &[
            (1, identity.as_slice()),
            (2, sites.as_slice()),
            (3, counters.as_slice()),
            (4, shard.run_id.as_slice()),
            (5, flags.as_slice()),
        ],
    )
}

/// Parses and validates one completed `CKPART01` shard.
///
/// # Errors
///
/// Returns a stable validation error for every malformed or non-canonical input.
pub fn parse_profile_shard(bytes: &[u8]) -> Result<CkProfileShard, CkProfileError> {
    let fields = parse_outer(bytes, SHARD_MAGIC, b"CK-PROFILE-SHARD\0", &[1, 2, 3, 4, 5])?;
    let identity = decode_identity(fields[0])?;
    let sites = decode_sites(fields[1])?;
    let counters = decode_counters(fields[2])?;
    let run_id = fields[3]
        .try_into()
        .map_err(|_| CkProfileError::InvalidValue("shard.runId"))?;
    let (overflowed, incomplete_observations) = decode_flags(fields[4])?;
    validate_tables(&identity, &sites, &counters)?;
    Ok(CkProfileShard {
        identity,
        sites,
        counters,
        run_id,
        overflowed,
        incomplete_observations,
    })
}

/// Serializes one terminal aggregate into canonical `CKPROF01` bytes.
///
/// # Errors
///
/// Returns an error when identity, descriptor, counter, or aggregate invariants fail.
pub fn serialize_profile(profile: &CkProfile) -> Result<Vec<u8>, CkProfileError> {
    validate_tables(&profile.identity, &profile.sites, &profile.counters)?;
    if profile.completed_runs == 0 || profile.merged_shards == 0 {
        return Err(CkProfileError::InvalidValue("profile.aggregateCounts"));
    }
    let identity = profile.identity.canonical_bytes()?;
    let sites = encode_sites(&profile.sites)?;
    let counters = encode_counters(&profile.counters)?;
    let runs = profile.completed_runs.to_be_bytes();
    let shards = profile.merged_shards.to_be_bytes();
    let flags = encode_flags(profile.overflowed, profile.incomplete_observations);
    encode_outer(
        PROFILE_MAGIC,
        b"CK-PROFILE-FINAL\0",
        &[
            (1, identity.as_slice()),
            (2, sites.as_slice()),
            (3, counters.as_slice()),
            (4, runs.as_slice()),
            (5, shards.as_slice()),
            (6, flags.as_slice()),
        ],
    )
}

/// Parses and validates one terminal `CKPROF01` aggregate.
///
/// # Errors
///
/// Returns a stable validation error for every malformed or non-canonical input.
pub fn parse_profile(bytes: &[u8]) -> Result<CkProfile, CkProfileError> {
    let fields = parse_outer(
        bytes,
        PROFILE_MAGIC,
        b"CK-PROFILE-FINAL\0",
        &[1, 2, 3, 4, 5, 6],
    )?;
    let identity = decode_identity(fields[0])?;
    let sites = decode_sites(fields[1])?;
    let counters = decode_counters(fields[2])?;
    let completed_runs = u64::from_be_bytes(
        fields[3]
            .try_into()
            .map_err(|_| CkProfileError::InvalidValue("profile.completedRuns"))?,
    );
    let merged_shards = u32::from_be_bytes(
        fields[4]
            .try_into()
            .map_err(|_| CkProfileError::InvalidValue("profile.mergedShards"))?,
    );
    let (overflowed, incomplete_observations) = decode_flags(fields[5])?;
    if completed_runs == 0 || merged_shards == 0 || merged_shards > CK_PROFILE_MAX_SHARDS {
        return Err(CkProfileError::InvalidValue("profile.aggregateCounts"));
    }
    validate_tables(&identity, &sites, &counters)?;
    Ok(CkProfile {
        identity,
        sites,
        counters,
        completed_runs,
        merged_shards,
        overflowed,
        incomplete_observations,
    })
}

fn validate_tables(
    identity: &CkProfileIdentity,
    sites: &[CkProfileSiteDescriptor],
    counters: &[CkProfileCounterRecord],
) -> Result<(), CkProfileError> {
    identity.validate()?;
    if sites.len() > usize::try_from(CK_PROFILE_MAX_SITES).unwrap_or(usize::MAX) {
        return Err(CkProfileError::ResourceLimit("site count"));
    }
    if sites.len() != counters.len() {
        return Err(CkProfileError::CounterTableMismatch);
    }
    ensure_sites_canonical(sites)?;
    ensure_counters_canonical(counters)?;
    for (site, counter) in sites.iter().zip(counters) {
        if site.id != counter.site_id || !counter_matches_site(&counter.counter, &site.kind) {
            return Err(CkProfileError::CounterTableMismatch);
        }
    }
    if profile_site_table_digest(sites)? != identity.module.site_table_digest {
        return Err(CkProfileError::SiteTableMismatch);
    }
    Ok(())
}

fn counter_matches_site(counter: &CkProfileCounter, site: &CkProfileSiteKind) -> bool {
    match (counter, site) {
        (
            CkProfileCounter::Scalar(_),
            CkProfileSiteKind::FunctionEntry | CkProfileSiteKind::Edge { .. },
        )
        | (
            CkProfileCounter::Histogram { .. },
            CkProfileSiteKind::LoopTripHistogram { .. }
            | CkProfileSiteKind::SliceLengthHistogram { .. },
        ) => true,
        (
            CkProfileCounter::CandidateConstant { candidates, .. },
            CkProfileSiteKind::CandidateConstant {
                candidates: expected,
                ..
            },
        ) => candidates.len() == expected.len(),
        _ => false,
    }
}

fn ensure_sites_canonical(sites: &[CkProfileSiteDescriptor]) -> Result<(), CkProfileError> {
    for pair in sites.windows(2) {
        match pair[0].id.cmp(&pair[1].id) {
            Ordering::Less => {}
            Ordering::Equal if pair[0] != pair[1] => return Err(CkProfileError::SiteIdCollision),
            _ => return Err(CkProfileError::NonCanonicalOrder("site table")),
        }
    }
    Ok(())
}

fn ensure_counters_canonical(counters: &[CkProfileCounterRecord]) -> Result<(), CkProfileError> {
    if counters
        .windows(2)
        .any(|pair| pair[0].site_id >= pair[1].site_id)
    {
        return Err(CkProfileError::NonCanonicalOrder("counter table"));
    }
    Ok(())
}

fn encode_outer(
    magic: &[u8; 8],
    domain: &[u8],
    fields: &[(u16, &[u8])],
) -> Result<Vec<u8>, CkProfileError> {
    let mut output = Vec::new();
    output.extend_from_slice(magic);
    output.extend_from_slice(&CK_PROFILE_FORMAT_SCHEMA.to_be_bytes());
    for (tag, payload) in fields {
        push_field(&mut output, *tag, payload)?;
    }
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(&output);
    output.extend_from_slice(&hasher.finalize());
    if u64::try_from(output.len()).map_err(|_| CkProfileError::LengthOverflow)?
        > CK_PROFILE_MAX_BYTES
    {
        return Err(CkProfileError::ResourceLimit("profile bytes"));
    }
    Ok(output)
}

fn parse_outer<'a>(
    bytes: &'a [u8],
    magic: &[u8; 8],
    domain: &[u8],
    required_tags: &[u16],
) -> Result<Vec<&'a [u8]>, CkProfileError> {
    if u64::try_from(bytes.len()).map_err(|_| CkProfileError::LengthOverflow)?
        > CK_PROFILE_MAX_BYTES
    {
        return Err(CkProfileError::ResourceLimit("profile bytes"));
    }
    let minimum = OUTER_HEADER_BYTES
        .checked_add(DIGEST_BYTES)
        .ok_or(CkProfileError::LengthOverflow)?;
    if bytes.len() < minimum {
        return Err(CkProfileError::Truncated);
    }
    if bytes.get(..8) != Some(magic.as_slice()) {
        return Err(CkProfileError::UnexpectedMagic);
    }
    let schema = u32::from_be_bytes(
        bytes[8..12]
            .try_into()
            .map_err(|_| CkProfileError::Truncated)?,
    );
    if schema != CK_PROFILE_FORMAT_SCHEMA {
        return Err(CkProfileError::UnsupportedSchema {
            kind: "profile format",
            expected: CK_PROFILE_FORMAT_SCHEMA,
            observed: schema,
        });
    }
    let body_end = bytes
        .len()
        .checked_sub(DIGEST_BYTES)
        .ok_or(CkProfileError::LengthOverflow)?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(&bytes[..body_end]);
    if hasher.finalize().as_slice() != &bytes[body_end..] {
        return Err(CkProfileError::DigestMismatch);
    }
    let mut cursor = WireCursor::new(&bytes[OUTER_HEADER_BYTES..body_end]);
    let mut fields = Vec::with_capacity(required_tags.len());
    let mut previous = 0u16;
    while !cursor.is_empty() {
        let tag = cursor.read_u16()?;
        if tag <= previous {
            return Err(CkProfileError::NonCanonicalOrder("outer fields"));
        }
        previous = tag;
        let length =
            usize::try_from(cursor.read_u32()?).map_err(|_| CkProfileError::LengthOverflow)?;
        if u64::try_from(length).map_err(|_| CkProfileError::LengthOverflow)? > CK_PROFILE_MAX_BYTES
        {
            return Err(CkProfileError::ResourceLimit("outer field bytes"));
        }
        let payload = cursor.read_exact(length)?;
        if !required_tags.contains(&tag) {
            return Err(CkProfileError::UnknownField {
                context: "outer",
                tag,
            });
        }
        fields.push((tag, payload));
    }
    let mut ordered = Vec::with_capacity(required_tags.len());
    for required in required_tags {
        let Some((_, payload)) = fields.iter().find(|(tag, _)| tag == required) else {
            return Err(CkProfileError::MissingField {
                context: "outer",
                field: *required,
            });
        };
        ordered.push(*payload);
    }
    Ok(ordered)
}

fn push_field(output: &mut Vec<u8>, tag: u16, payload: &[u8]) -> Result<(), CkProfileError> {
    let length = u32::try_from(payload.len()).map_err(|_| CkProfileError::LengthOverflow)?;
    output.extend_from_slice(&tag.to_be_bytes());
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(payload);
    Ok(())
}

fn encode_sites(sites: &[CkProfileSiteDescriptor]) -> Result<Vec<u8>, CkProfileError> {
    ensure_sites_canonical(sites)?;
    let count = u32::try_from(sites.len()).map_err(|_| CkProfileError::LengthOverflow)?;
    if count > CK_PROFILE_MAX_SITES {
        return Err(CkProfileError::ResourceLimit("site count"));
    }
    let mut output = Vec::new();
    output.extend_from_slice(&count.to_be_bytes());
    for site in sites {
        output.extend_from_slice(&site.id.0);
        output.extend_from_slice(&site.function_digest);
        output.extend_from_slice(&site.location.to_be_bytes());
        match &site.kind {
            CkProfileSiteKind::FunctionEntry => output.push(1),
            CkProfileSiteKind::Edge {
                from_block,
                to_block,
                reconstructed,
            } => {
                output.push(2);
                output.extend_from_slice(&from_block.to_be_bytes());
                output.extend_from_slice(&to_block.to_be_bytes());
                output.push(u8::from(*reconstructed));
            }
            CkProfileSiteKind::LoopTripHistogram { loop_identity } => {
                output.push(3);
                output.extend_from_slice(&loop_identity.to_be_bytes());
            }
            CkProfileSiteKind::SliceLengthHistogram { decision_identity } => {
                output.push(4);
                output.extend_from_slice(&decision_identity.to_be_bytes());
            }
            CkProfileSiteKind::CandidateConstant {
                decision_identity,
                candidates,
            } => {
                if candidates.is_empty()
                    || candidates.len() > usize::from(CK_PROFILE_MAX_CANDIDATES)
                {
                    return Err(CkProfileError::ResourceLimit("candidate constants"));
                }
                output.push(5);
                output.extend_from_slice(&decision_identity.to_be_bytes());
                output.push(
                    u8::try_from(candidates.len()).map_err(|_| CkProfileError::LengthOverflow)?,
                );
                for candidate in candidates {
                    output.extend_from_slice(&candidate.to_be_bytes());
                }
            }
        }
    }
    Ok(output)
}

pub(super) fn encode_sites_for_runtime(
    sites: &[CkProfileSiteDescriptor],
) -> Result<Vec<u8>, CkProfileError> {
    encode_sites(sites)
}

fn decode_sites(bytes: &[u8]) -> Result<Vec<CkProfileSiteDescriptor>, CkProfileError> {
    let mut cursor = WireCursor::new(bytes);
    let count = cursor.read_u32()?;
    if count > CK_PROFILE_MAX_SITES {
        return Err(CkProfileError::ResourceLimit("site count"));
    }
    let capacity = usize::try_from(count).map_err(|_| CkProfileError::LengthOverflow)?;
    let mut sites = Vec::with_capacity(capacity);
    for _ in 0..count {
        let id = CkProfileSiteId(cursor.read_array()?);
        let function_digest = cursor.read_array()?;
        let location = cursor.read_u32()?;
        let kind = match cursor.read_u8()? {
            1 => CkProfileSiteKind::FunctionEntry,
            2 => CkProfileSiteKind::Edge {
                from_block: cursor.read_u32()?,
                to_block: cursor.read_u32()?,
                reconstructed: cursor.read_bool("site.edge.reconstructed")?,
            },
            3 => CkProfileSiteKind::LoopTripHistogram {
                loop_identity: cursor.read_u32()?,
            },
            4 => CkProfileSiteKind::SliceLengthHistogram {
                decision_identity: cursor.read_u32()?,
            },
            5 => {
                let decision_identity = cursor.read_u32()?;
                let candidate_count = cursor.read_u8()?;
                if candidate_count == 0 || candidate_count > CK_PROFILE_MAX_CANDIDATES {
                    return Err(CkProfileError::ResourceLimit("candidate constants"));
                }
                let mut candidates = Vec::with_capacity(usize::from(candidate_count));
                for _ in 0..candidate_count {
                    candidates.push(cursor.read_i64()?);
                }
                CkProfileSiteKind::CandidateConstant {
                    decision_identity,
                    candidates,
                }
            }
            _ => return Err(CkProfileError::InvalidValue("site.kind")),
        };
        sites.push(CkProfileSiteDescriptor {
            id,
            function_digest,
            location,
            kind,
        });
    }
    if !cursor.is_empty() {
        return Err(CkProfileError::NonCanonicalOrder("site trailing bytes"));
    }
    ensure_sites_canonical(&sites)?;
    Ok(sites)
}

fn encode_counters(counters: &[CkProfileCounterRecord]) -> Result<Vec<u8>, CkProfileError> {
    ensure_counters_canonical(counters)?;
    let count = u32::try_from(counters.len()).map_err(|_| CkProfileError::LengthOverflow)?;
    if count > CK_PROFILE_MAX_SITES {
        return Err(CkProfileError::ResourceLimit("counter count"));
    }
    let mut output = Vec::new();
    output.extend_from_slice(&count.to_be_bytes());
    for record in counters {
        output.extend_from_slice(&record.site_id.0);
        match &record.counter {
            CkProfileCounter::Scalar(value) => {
                output.push(1);
                output.extend_from_slice(&value.to_be_bytes());
            }
            CkProfileCounter::Histogram { buckets, saturated } => {
                output.push(2);
                for value in buckets {
                    output.extend_from_slice(&value.to_be_bytes());
                }
                output.push(u8::from(*saturated));
            }
            CkProfileCounter::CandidateConstant {
                candidates,
                other,
                saturated,
            } => {
                if candidates.is_empty()
                    || candidates.len() > usize::from(CK_PROFILE_MAX_CANDIDATES)
                {
                    return Err(CkProfileError::ResourceLimit("candidate counters"));
                }
                output.push(3);
                output.push(
                    u8::try_from(candidates.len()).map_err(|_| CkProfileError::LengthOverflow)?,
                );
                for value in candidates {
                    output.extend_from_slice(&value.to_be_bytes());
                }
                output.extend_from_slice(&other.to_be_bytes());
                output.push(u8::from(*saturated));
            }
        }
    }
    Ok(output)
}

pub(super) fn encode_counters_for_runtime(
    counters: &[CkProfileCounterRecord],
) -> Result<Vec<u8>, CkProfileError> {
    encode_counters(counters)
}

fn decode_counters(bytes: &[u8]) -> Result<Vec<CkProfileCounterRecord>, CkProfileError> {
    let mut cursor = WireCursor::new(bytes);
    let count = cursor.read_u32()?;
    if count > CK_PROFILE_MAX_SITES {
        return Err(CkProfileError::ResourceLimit("counter count"));
    }
    let capacity = usize::try_from(count).map_err(|_| CkProfileError::LengthOverflow)?;
    let mut counters = Vec::with_capacity(capacity);
    for _ in 0..count {
        let site_id = CkProfileSiteId(cursor.read_array()?);
        let counter = match cursor.read_u8()? {
            1 => CkProfileCounter::Scalar(cursor.read_u64()?),
            2 => {
                let mut buckets = [0u64; 16];
                for value in &mut buckets {
                    *value = cursor.read_u64()?;
                }
                CkProfileCounter::Histogram {
                    buckets,
                    saturated: cursor.read_bool("counter.histogram.saturated")?,
                }
            }
            3 => {
                let candidate_count = cursor.read_u8()?;
                if candidate_count == 0 || candidate_count > CK_PROFILE_MAX_CANDIDATES {
                    return Err(CkProfileError::ResourceLimit("candidate counters"));
                }
                let mut candidates = Vec::with_capacity(usize::from(candidate_count));
                for _ in 0..candidate_count {
                    candidates.push(cursor.read_u64()?);
                }
                CkProfileCounter::CandidateConstant {
                    candidates,
                    other: cursor.read_u64()?,
                    saturated: cursor.read_bool("counter.constant.saturated")?,
                }
            }
            _ => return Err(CkProfileError::InvalidValue("counter.kind")),
        };
        counters.push(CkProfileCounterRecord { site_id, counter });
    }
    if !cursor.is_empty() {
        return Err(CkProfileError::NonCanonicalOrder("counter trailing bytes"));
    }
    ensure_counters_canonical(&counters)?;
    Ok(counters)
}

const fn encode_flags(overflowed: bool, incomplete: bool) -> [u8; 2] {
    [overflowed as u8, incomplete as u8]
}

fn decode_flags(bytes: &[u8]) -> Result<(bool, bool), CkProfileError> {
    if bytes.len() != 2 {
        return Err(CkProfileError::InvalidValue("profile.flags"));
    }
    Ok((
        decode_bool(bytes[0], "flags.overflowed")?,
        decode_bool(bytes[1], "flags.incomplete")?,
    ))
}

fn decode_bool(value: u8, field: &'static str) -> Result<bool, CkProfileError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(CkProfileError::InvalidValue(field)),
    }
}

struct WireCursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> WireCursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn read_exact(&mut self, length: usize) -> Result<&'a [u8], CkProfileError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(CkProfileError::LengthOverflow)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(CkProfileError::Truncated)?;
        self.offset = end;
        Ok(value)
    }

    fn read_array<const N: usize>(&mut self) -> Result<[u8; N], CkProfileError> {
        self.read_exact(N)?
            .try_into()
            .map_err(|_| CkProfileError::Truncated)
    }

    fn read_u8(&mut self) -> Result<u8, CkProfileError> {
        Ok(self.read_exact(1)?[0])
    }

    fn read_bool(&mut self, field: &'static str) -> Result<bool, CkProfileError> {
        decode_bool(self.read_u8()?, field)
    }

    fn read_u16(&mut self) -> Result<u16, CkProfileError> {
        Ok(u16::from_be_bytes(self.read_array()?))
    }

    fn read_u32(&mut self) -> Result<u32, CkProfileError> {
        Ok(u32::from_be_bytes(self.read_array()?))
    }

    fn read_u64(&mut self) -> Result<u64, CkProfileError> {
        Ok(u64::from_be_bytes(self.read_array()?))
    }

    fn read_i64(&mut self) -> Result<i64, CkProfileError> {
        Ok(i64::from_be_bytes(self.read_array()?))
    }

    fn is_empty(&self) -> bool {
        self.offset == self.bytes.len()
    }
}
