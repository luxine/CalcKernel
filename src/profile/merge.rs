use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Component, Path, PathBuf},
};

use sha2::{Digest, Sha256};

use super::CkProfileError;
use super::format::{
    CkProfile, CkProfileCounter, CkProfileCounterRecord, CkProfileShard, parse_profile,
    parse_profile_shard, serialize_profile, serialize_profile_shard,
};
use super::identity::{CK_PROFILE_MAX_BYTES, CK_PROFILE_MAX_SHARDS};

/// Canonical terminal merge result and directory-scan diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkProfileMergeOutput {
    pub profile: CkProfile,
    pub profile_bytes: Vec<u8>,
    pub ignored_temporary_files: u32,
}

/// Merges already parsed raw shards into one terminal aggregate.
///
/// # Errors
///
/// Rejects empty input, duplicate runs/content, incompatible identities or site
/// tables, malformed counter shapes, and checked arithmetic overflow.
pub fn merge_profile_shards(shards: &[CkProfileShard]) -> Result<CkProfile, CkProfileError> {
    if shards.is_empty() {
        return Err(CkProfileError::InvalidValue("merge.empty"));
    }
    if shards.len() > usize::try_from(CK_PROFILE_MAX_SHARDS).unwrap_or(usize::MAX) {
        return Err(CkProfileError::ResourceLimit("merge shard count"));
    }
    let first = &shards[0];
    let mut run_ids = BTreeSet::new();
    let mut shard_digests = BTreeSet::new();
    let mut counters = first.counters.clone();
    let mut overflowed =
        first.overflowed || counters.iter().any(|item| item.counter.is_saturated());
    let mut incomplete = first.incomplete_observations;
    for (index, shard) in shards.iter().enumerate() {
        if !run_ids.insert(shard.run_id) {
            return Err(CkProfileError::DuplicateRunIdentity);
        }
        let canonical = serialize_profile_shard(shard)?;
        let digest: [u8; 32] = Sha256::digest(&canonical).into();
        if !shard_digests.insert(digest) {
            return Err(CkProfileError::DuplicateShardContent);
        }
        if let Some((field, expected, observed)) = first.identity.first_mismatch(&shard.identity) {
            return Err(CkProfileError::IdentityMismatch {
                field,
                expected,
                observed,
            });
        }
        if first.sites != shard.sites {
            return Err(CkProfileError::SiteTableMismatch);
        }
        if index != 0 {
            add_counter_tables(&mut counters, &shard.counters, &mut overflowed)?;
        }
        overflowed |= shard.overflowed
            || shard
                .counters
                .iter()
                .any(|item| item.counter.is_saturated());
        incomplete |= shard.incomplete_observations;
    }
    let merged_shards = u32::try_from(shards.len()).map_err(|_| CkProfileError::LengthOverflow)?;
    Ok(CkProfile {
        identity: first.identity.clone(),
        sites: first.sites.clone(),
        counters,
        completed_runs: u64::from(merged_shards),
        merged_shards,
        overflowed,
        incomplete_observations: incomplete,
    })
}

/// Scans explicit files and one directory level, then merges completed shards.
///
/// # Errors
///
/// Rejects symlinks, recursive or terminal profile inputs, duplicate shard
/// content, filesystem failures, and every validation error from shard parsing.
pub fn merge_profile_inputs(paths: &[PathBuf]) -> Result<CkProfileMergeOutput, CkProfileError> {
    if paths.is_empty() {
        return Err(CkProfileError::InvalidValue("merge.empty"));
    }
    let mut files = Vec::new();
    let mut ignored_temporary_files = 0u32;
    for path in paths {
        scan_input(path, &mut files, &mut ignored_temporary_files)?;
    }
    if files.len() > usize::try_from(CK_PROFILE_MAX_SHARDS).unwrap_or(usize::MAX) {
        return Err(CkProfileError::ResourceLimit("merge shard count"));
    }
    let mut inputs = Vec::with_capacity(files.len());
    for path in files {
        let bytes = read_no_follow(&path)?;
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        let shard = parse_profile_shard(&bytes)?;
        inputs.push((digest, shard));
    }
    inputs.sort_by_key(|(digest, _)| *digest);
    for pair in inputs.windows(2) {
        if pair[0].0 == pair[1].0 {
            return Err(CkProfileError::DuplicateShardContent);
        }
    }
    let shards = inputs
        .into_iter()
        .map(|(_, shard)| shard)
        .collect::<Vec<_>>();
    let profile = merge_profile_shards(&shards)?;
    let profile_bytes = serialize_profile(&profile)?;
    Ok(CkProfileMergeOutput {
        profile,
        profile_bytes,
        ignored_temporary_files,
    })
}

/// Validates that a profile output path contains no symlink/reparse component
/// and no parent-directory traversal before an atomic writer opens it.
///
/// # Errors
///
/// Returns a stable path or I/O error when the output path is not safe to use.
pub fn validate_profile_output_path(path: &Path) -> Result<(), CkProfileError> {
    reject_symlink_components(path)
}

/// Reads one terminal `.ckprof` through the same no-follow and resource-limit
/// boundary used by merge inputs.
///
/// # Errors
///
/// Rejects non-terminal names, symbolic components, non-files, oversized input,
/// and every parser validation failure.
pub fn read_profile_input(path: &Path) -> Result<(CkProfile, Vec<u8>), CkProfileError> {
    reject_symlink_components(path)?;
    if !is_final_profile_name(path) {
        return Err(CkProfileError::UnsupportedMergeInput(
            path.display().to_string(),
        ));
    }
    let bytes = read_no_follow(path)?;
    let profile = parse_profile(&bytes)?;
    Ok((profile, bytes))
}

fn scan_input(
    path: &Path,
    files: &mut Vec<PathBuf>,
    ignored_temporary_files: &mut u32,
) -> Result<(), CkProfileError> {
    reject_symlink_components(path)?;
    let metadata = fs::symlink_metadata(path).map_err(|error| CkProfileError::io(path, error))?;
    if metadata.file_type().is_symlink() {
        return Err(CkProfileError::SymlinkInput(path.display().to_string()));
    }
    if metadata.is_file() {
        require_completed_shard_name(path)?;
        files.push(path.to_path_buf());
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(CkProfileError::UnsupportedMergeInput(
            path.display().to_string(),
        ));
    }
    let mut entries = fs::read_dir(path)
        .map_err(|error| CkProfileError::io(path, error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| CkProfileError::io(path, error))?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let entry_path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| CkProfileError::io(&entry_path, error))?;
        if file_type.is_symlink() {
            return Err(CkProfileError::SymlinkInput(
                entry_path.display().to_string(),
            ));
        }
        if file_type.is_dir() {
            return Err(CkProfileError::UnsupportedMergeInput(
                entry_path.display().to_string(),
            ));
        }
        if is_temporary_name(&entry_path) {
            *ignored_temporary_files = ignored_temporary_files
                .checked_add(1)
                .ok_or(CkProfileError::LengthOverflow)?;
            continue;
        }
        if is_completed_shard_name(&entry_path) {
            if !file_type.is_file() {
                return Err(CkProfileError::UnsupportedMergeInput(
                    entry_path.display().to_string(),
                ));
            }
            files.push(entry_path);
        } else if is_final_profile_name(&entry_path) {
            return Err(CkProfileError::UnsupportedMergeInput(
                entry_path.display().to_string(),
            ));
        }
    }
    Ok(())
}

fn require_completed_shard_name(path: &Path) -> Result<(), CkProfileError> {
    if is_completed_shard_name(path) {
        Ok(())
    } else {
        Err(CkProfileError::UnsupportedMergeInput(
            path.display().to_string(),
        ))
    }
}

fn is_completed_shard_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".ckprof-part"))
}

fn is_final_profile_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".ckprof"))
}

fn is_temporary_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.ends_with(".ckprof-part.tmp") || name.contains(".ckprof-part.stage-")
        })
}

fn reject_symlink_components(path: &Path) -> Result<(), CkProfileError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| CkProfileError::io(path, error))?
            .join(path)
    };
    let mut current = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                current.push(component.as_os_str());
            }
            Component::CurDir => continue,
            Component::ParentDir => {
                return Err(CkProfileError::UnsupportedMergeInput(
                    path.display().to_string(),
                ));
            }
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(CkProfileError::SymlinkInput(current.display().to_string()));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(CkProfileError::io(&current, error)),
        }
    }
    Ok(())
}

fn read_no_follow(path: &Path) -> Result<Vec<u8>, CkProfileError> {
    let file = open_no_follow(path)?;
    let metadata = file
        .metadata()
        .map_err(|error| CkProfileError::io(path, error))?;
    if !metadata.is_file() || metadata.len() > CK_PROFILE_MAX_BYTES {
        return Err(CkProfileError::ResourceLimit("profile bytes"));
    }
    let capacity = usize::try_from(metadata.len()).map_err(|_| CkProfileError::LengthOverflow)?;
    let mut bytes = Vec::with_capacity(capacity);
    file.take(CK_PROFILE_MAX_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| CkProfileError::io(path, error))?;
    if u64::try_from(bytes.len()).map_err(|_| CkProfileError::LengthOverflow)?
        > CK_PROFILE_MAX_BYTES
    {
        return Err(CkProfileError::ResourceLimit("profile bytes"));
    }
    Ok(bytes)
}

#[cfg(unix)]
fn open_no_follow(path: &Path) -> Result<File, CkProfileError> {
    use std::os::unix::fs::OpenOptionsExt;

    #[cfg(target_os = "linux")]
    const O_NOFOLLOW: i32 = 0x20_000;
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    const O_NOFOLLOW: i32 = 0x100;
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "ios")))]
    const O_NOFOLLOW: i32 = 0;

    OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW)
        .open(path)
        .map_err(|error| CkProfileError::io(path, error))
}

#[cfg(windows)]
fn open_no_follow(path: &Path) -> Result<File, CkProfileError> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|error| CkProfileError::io(path, error))
}

fn add_counter_tables(
    aggregate: &mut [CkProfileCounterRecord],
    observed: &[CkProfileCounterRecord],
    overflowed: &mut bool,
) -> Result<(), CkProfileError> {
    if aggregate.len() != observed.len() {
        return Err(CkProfileError::CounterTableMismatch);
    }
    for (target, input) in aggregate.iter_mut().zip(observed) {
        if target.site_id != input.site_id {
            return Err(CkProfileError::CounterTableMismatch);
        }
        add_counter(&mut target.counter, &input.counter, overflowed)?;
    }
    Ok(())
}

fn add_counter(
    aggregate: &mut CkProfileCounter,
    observed: &CkProfileCounter,
    overflowed: &mut bool,
) -> Result<(), CkProfileError> {
    match (aggregate, observed) {
        (CkProfileCounter::Scalar(target), CkProfileCounter::Scalar(input)) => {
            saturating_add(target, *input, overflowed);
        }
        (
            CkProfileCounter::Histogram {
                buckets: target,
                saturated: target_saturated,
            },
            CkProfileCounter::Histogram {
                buckets: input,
                saturated: input_saturated,
            },
        ) => {
            let mut counter_overflowed = false;
            for (target, input) in target.iter_mut().zip(input) {
                saturating_add(target, *input, &mut counter_overflowed);
            }
            *target_saturated |= *input_saturated || counter_overflowed;
            *overflowed |= counter_overflowed;
        }
        (
            CkProfileCounter::CandidateConstant {
                candidates: target,
                other: target_other,
                saturated: target_saturated,
            },
            CkProfileCounter::CandidateConstant {
                candidates: input,
                other: input_other,
                saturated: input_saturated,
            },
        ) if target.len() == input.len() => {
            let mut counter_overflowed = false;
            for (target, input) in target.iter_mut().zip(input) {
                saturating_add(target, *input, &mut counter_overflowed);
            }
            saturating_add(target_other, *input_other, &mut counter_overflowed);
            *target_saturated |= *input_saturated || counter_overflowed;
            *overflowed |= counter_overflowed;
        }
        _ => return Err(CkProfileError::CounterTableMismatch),
    }
    Ok(())
}

fn saturating_add(target: &mut u64, input: u64, overflowed: &mut bool) {
    match target.checked_add(input) {
        Some(value) => *target = value,
        None => {
            *target = u64::MAX;
            *overflowed = true;
        }
    }
}
