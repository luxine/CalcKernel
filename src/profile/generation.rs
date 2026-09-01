use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use super::{
    CkProfileCounter, CkProfileCounterRecord, CkProfileError, CkProfileIdentity, CkProfileShard,
    CkProfileSiteDescriptor, CkProfileSiteKind, serialize_profile_shard,
};

/// Sentinel used when a site has no per-site saturation byte in the wire image.
pub const CK_PROFILE_NO_WIRE_OFFSET: u32 = u32::MAX;

/// Platform file identity captured when a generation artifact is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CkProfileDirectoryIdentity {
    pub first: u64,
    pub second: u64,
}

/// Absolute, no-indirection output directory embedded only in generation artifacts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkProfileDirectoryAnchor {
    pub path: PathBuf,
    pub identity: CkProfileDirectoryIdentity,
}

/// One canonical, mutable shard image plus the closed counter patch map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkProfileShardTemplate {
    pub bytes: Vec<u8>,
    pub counter_offsets: Vec<u32>,
    pub site_first_counters: Vec<u32>,
    pub site_counter_counts: Vec<u32>,
    pub site_saturation_offsets: Vec<u32>,
    pub run_id_offset: u32,
    pub overflow_flag_offset: u32,
    pub digest_offset: u32,
}

/// Validates and anchors an existing profile collection directory.
///
/// # Errors
///
/// Rejects relative paths, missing/non-directory components, symbolic links,
/// unsupported path encodings, and platforms that cannot expose a stable file
/// identity.
pub fn anchor_profile_directory(path: &Path) -> Result<CkProfileDirectoryAnchor, CkProfileError> {
    if !path.is_absolute() {
        return Err(CkProfileError::InvalidValue(
            "generation.directory.absolute",
        ));
    }
    if path
        .as_os_str()
        .to_str()
        .is_none_or(|text| text.contains('\0'))
    {
        return Err(CkProfileError::InvalidValue(
            "generation.directory.encoding",
        ));
    }
    validate_components_without_indirection(path)?;
    let metadata = fs::metadata(path).map_err(|error| CkProfileError::io(path, error))?;
    if !metadata.is_dir() {
        return Err(CkProfileError::InvalidValue("generation.directory.type"));
    }
    let identity = directory_identity(&metadata)?;
    Ok(CkProfileDirectoryAnchor {
        path: path.to_path_buf(),
        identity,
    })
}

/// Builds the canonical zero-counter shard image used by the Native collector.
///
/// # Errors
///
/// Returns the same canonicality/resource failures as shard serialization, or
/// an offset overflow when the runtime patch table cannot be represented.
pub fn create_profile_shard_template(
    identity: CkProfileIdentity,
    sites: Vec<CkProfileSiteDescriptor>,
) -> Result<CkProfileShardTemplate, CkProfileError> {
    let counters = zero_counters(&sites);
    let identity_bytes = identity.canonical_bytes()?;
    let site_bytes = super::format::encode_sites_for_runtime(&sites)?;
    let counter_bytes = super::format::encode_counters_for_runtime(&counters)?;
    let counter_payload_offset = checked_u32(
        12usize
            .checked_add(6)
            .and_then(|value| value.checked_add(identity_bytes.len()))
            .and_then(|value| value.checked_add(6))
            .and_then(|value| value.checked_add(site_bytes.len()))
            .and_then(|value| value.checked_add(6))
            .ok_or(CkProfileError::LengthOverflow)?,
    )?;
    let layout = counter_layout(&counters, counter_payload_offset)?;
    let run_id_offset = checked_u32(
        usize::try_from(counter_payload_offset)
            .map_err(|_| CkProfileError::LengthOverflow)?
            .checked_add(counter_bytes.len())
            .and_then(|value| value.checked_add(6))
            .ok_or(CkProfileError::LengthOverflow)?,
    )?;
    let overflow_flag_offset = run_id_offset
        .checked_add(16)
        .and_then(|value| value.checked_add(6))
        .ok_or(CkProfileError::LengthOverflow)?;
    let shard = CkProfileShard {
        identity,
        sites,
        counters,
        run_id: [0; 16],
        overflowed: false,
        incomplete_observations: false,
    };
    let bytes = serialize_profile_shard(&shard)?;
    let digest_offset = checked_u32(
        bytes
            .len()
            .checked_sub(32)
            .ok_or(CkProfileError::LengthOverflow)?,
    )?;
    if usize::try_from(overflow_flag_offset).ok() >= Some(bytes.len())
        || usize::try_from(run_id_offset)
            .ok()
            .is_none_or(|offset| offset + 16 > bytes.len())
    {
        return Err(CkProfileError::LengthOverflow);
    }
    Ok(CkProfileShardTemplate {
        bytes,
        counter_offsets: layout.counter_offsets,
        site_first_counters: layout.site_first_counters,
        site_counter_counts: layout.site_counter_counts,
        site_saturation_offsets: layout.site_saturation_offsets,
        run_id_offset,
        overflow_flag_offset,
        digest_offset,
    })
}

/// Maps a schema-1 u32 length or trip count to its canonical histogram bucket.
#[must_use]
pub const fn profile_histogram_bucket(value: u32) -> u32 {
    match value {
        0 => 0,
        1 => 1,
        2 => 2,
        3..=4 => 3,
        5..=8 => 4,
        9..=16 => 5,
        17..=32 => 6,
        33..=64 => 7,
        65..=128 => 8,
        129..=256 => 9,
        257..=512 => 10,
        513..=1024 => 11,
        1025..=2048 => 12,
        2049..=4096 => 13,
        4097..=65536 => 14,
        65537..=u32::MAX => 15,
    }
}

fn zero_counters(sites: &[CkProfileSiteDescriptor]) -> Vec<CkProfileCounterRecord> {
    sites
        .iter()
        .map(|site| CkProfileCounterRecord {
            site_id: site.id,
            counter: match &site.kind {
                CkProfileSiteKind::FunctionEntry | CkProfileSiteKind::Edge { .. } => {
                    CkProfileCounter::Scalar(0)
                }
                CkProfileSiteKind::LoopTripHistogram { .. }
                | CkProfileSiteKind::SliceLengthHistogram { .. } => CkProfileCounter::Histogram {
                    buckets: [0; 16],
                    saturated: false,
                },
                CkProfileSiteKind::CandidateConstant { candidates, .. } => {
                    CkProfileCounter::CandidateConstant {
                        candidates: vec![0; candidates.len()],
                        other: 0,
                        saturated: false,
                    }
                }
            },
        })
        .collect()
}

struct CounterLayout {
    counter_offsets: Vec<u32>,
    site_first_counters: Vec<u32>,
    site_counter_counts: Vec<u32>,
    site_saturation_offsets: Vec<u32>,
}

fn counter_layout(
    counters: &[CkProfileCounterRecord],
    payload_offset: u32,
) -> Result<CounterLayout, CkProfileError> {
    let mut wire = payload_offset
        .checked_add(4)
        .ok_or(CkProfileError::LengthOverflow)?;
    let mut counter_offsets = Vec::new();
    let mut site_first_counters = Vec::with_capacity(counters.len());
    let mut site_counter_counts = Vec::with_capacity(counters.len());
    let mut site_saturation_offsets = Vec::with_capacity(counters.len());
    for record in counters {
        wire = wire.checked_add(17).ok_or(CkProfileError::LengthOverflow)?;
        site_first_counters.push(checked_u32(counter_offsets.len())?);
        match &record.counter {
            CkProfileCounter::Scalar(_) => {
                counter_offsets.push(wire);
                site_counter_counts.push(1);
                site_saturation_offsets.push(CK_PROFILE_NO_WIRE_OFFSET);
                wire = wire.checked_add(8).ok_or(CkProfileError::LengthOverflow)?;
            }
            CkProfileCounter::Histogram { .. } => {
                for _ in 0..16 {
                    counter_offsets.push(wire);
                    wire = wire.checked_add(8).ok_or(CkProfileError::LengthOverflow)?;
                }
                site_counter_counts.push(16);
                site_saturation_offsets.push(wire);
                wire = wire.checked_add(1).ok_or(CkProfileError::LengthOverflow)?;
            }
            CkProfileCounter::CandidateConstant { candidates, .. } => {
                wire = wire.checked_add(1).ok_or(CkProfileError::LengthOverflow)?;
                let cells = candidates
                    .len()
                    .checked_add(1)
                    .ok_or(CkProfileError::LengthOverflow)?;
                for _ in 0..cells {
                    counter_offsets.push(wire);
                    wire = wire.checked_add(8).ok_or(CkProfileError::LengthOverflow)?;
                }
                site_counter_counts.push(checked_u32(cells)?);
                site_saturation_offsets.push(wire);
                wire = wire.checked_add(1).ok_or(CkProfileError::LengthOverflow)?;
            }
        }
    }
    Ok(CounterLayout {
        counter_offsets,
        site_first_counters,
        site_counter_counts,
        site_saturation_offsets,
    })
}

fn checked_u32(value: usize) -> Result<u32, CkProfileError> {
    u32::try_from(value).map_err(|_| CkProfileError::LengthOverflow)
}

fn validate_components_without_indirection(path: &Path) -> Result<(), CkProfileError> {
    let mut current = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => current.push(prefix.as_os_str()),
            Component::RootDir => current.push(component.as_os_str()),
            Component::Normal(part) => current.push(part),
            Component::CurDir | Component::ParentDir => {
                return Err(CkProfileError::InvalidValue(
                    "generation.directory.normalized",
                ));
            }
        }
        let metadata =
            fs::symlink_metadata(&current).map_err(|error| CkProfileError::io(&current, error))?;
        if metadata.file_type().is_symlink() {
            return Err(CkProfileError::SymlinkInput(current.display().to_string()));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn directory_identity(
    metadata: &fs::Metadata,
) -> Result<CkProfileDirectoryIdentity, CkProfileError> {
    use std::os::unix::fs::MetadataExt;
    Ok(CkProfileDirectoryIdentity {
        first: metadata.dev(),
        second: metadata.ino(),
    })
}

#[cfg(windows)]
fn directory_identity(
    metadata: &fs::Metadata,
) -> Result<CkProfileDirectoryIdentity, CkProfileError> {
    use std::os::windows::fs::MetadataExt;
    let volume = metadata
        .volume_serial_number()
        .ok_or(CkProfileError::InvalidValue(
            "generation.directory.identity",
        ))?;
    let index = metadata.file_index().ok_or(CkProfileError::InvalidValue(
        "generation.directory.identity",
    ))?;
    Ok(CkProfileDirectoryIdentity {
        first: u64::from(volume),
        second: index,
    })
}

#[cfg(not(any(unix, windows)))]
fn directory_identity(_: &fs::Metadata) -> Result<CkProfileDirectoryIdentity, CkProfileError> {
    Err(CkProfileError::InvalidValue(
        "generation.directory.identity",
    ))
}
