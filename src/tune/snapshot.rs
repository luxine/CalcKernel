use std::collections::BTreeSet;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

use super::input_map::{TuneInputMapEntry, encode_input_map};
use super::manifest::{TuneCaseRole, TuneManifest};

const MAX_RUNNER_BYTES: u64 = 1 << 30;
const MAX_INPUT_BYTES: u64 = 1 << 30;
const MAX_TOTAL_INPUT_BYTES: u64 = 4 << 30;

/// An immutable in-memory runner and declared-input snapshot.
pub struct CapturedWorkload {
    runner_bytes: Vec<u8>,
    runner_digest: [u8; 32],
    inputs: Vec<CapturedInput>,
    environment: Vec<CapturedEnvironment>,
    manifest_digest: [u8; 32],
    args: Vec<String>,
    timeout_ms: u32,
    cases: Vec<super::TuneCase>,
}

#[derive(Debug)]
struct CapturedInput {
    logical_path: String,
    bytes: Vec<u8>,
    digest: [u8; 32],
}

/// Public, path-safe identity of one captured workload input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuneCapturedInputIdentity {
    pub logical_path: String,
    pub bytes: u64,
    pub digest: [u8; 32],
}

struct CapturedEnvironment {
    identity: TuneEnvironmentIdentity,
    value: Vec<u8>,
}

/// Public, secret-free identity of one inherited environment value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuneEnvironmentIdentity {
    pub name: String,
    pub value_bytes: u64,
    pub value_digest: [u8; 32],
}

impl fmt::Debug for CapturedWorkload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CapturedWorkload")
            .field("runner_digest", &self.runner_digest)
            .field("input_count", &self.inputs.len())
            .field("environment_identities", &self.environment_identities())
            .field("manifest_digest", &self.manifest_digest)
            .finish_non_exhaustive()
    }
}

/// Fresh files and exact CKTIMAP1 path for one invocation.
#[derive(Debug)]
pub struct StagedInvocationInputs {
    files: Vec<PathBuf>,
    map_path: PathBuf,
}

impl StagedInvocationInputs {
    /// Returns the manifest-order staged input files.
    #[must_use]
    pub fn files(&self) -> &[PathBuf] {
        &self.files
    }

    /// Returns the exact read-only CKTIMAP1 file.
    #[must_use]
    pub fn map_path(&self) -> &Path {
        &self.map_path
    }
}

/// Snapshot and staging failures.
#[derive(Debug, thiserror::Error)]
pub enum TuneSnapshotError {
    #[error("snapshot I/O failed")]
    Io(#[from] std::io::Error),
    #[error("path contains a symlink or unsupported component")]
    UnsafePath,
    #[error("runner is not a host-native executable")]
    InvalidRunner,
    #[error("snapshot resource limit exceeded")]
    ResourceLimit,
    #[error("two manifest inputs resolve to the same file")]
    DuplicateInput,
    #[error("staged input digest mismatch")]
    DigestMismatch,
    #[error("input-map encoding failed")]
    InputMap,
    #[error("required inherited environment variable is missing: {0}")]
    MissingEnvironment(String),
    #[error("inherited environment value is not representable")]
    InvalidEnvironment,
}

/// Captures the runner and inputs into immutable private memory.
///
/// # Errors
///
/// Rejects indirection, non-regular inputs, duplicate identities, foreign runner
/// formats, executable-permission failures, races, and resource limits.
pub fn capture_workload(manifest: &TuneManifest) -> Result<CapturedWorkload, TuneSnapshotError> {
    let runner_path = resolve_without_indirection(&manifest.runner_path)?;
    let runner_bytes = read_regular_nofollow(&runner_path, MAX_RUNNER_BYTES)?;
    validate_runner(&runner_path, &runner_bytes)?;
    let runner_digest = Sha256::digest(&runner_bytes).into();
    let environment = capture_environment(manifest)?;
    debug_assert!(environment.iter().all(|entry| {
        u64::try_from(entry.value.len()).ok() == Some(entry.identity.value_bytes)
    }));

    let root = resolve_directory_without_indirection(&manifest.input_root)?;
    let mut inputs = Vec::with_capacity(manifest.inputs.len());
    let mut identities = BTreeSet::new();
    let mut total = 0u64;
    for logical_path in &manifest.inputs {
        let path = root.join(logical_path);
        let resolved = resolve_beneath_without_indirection(&root, &path)?;
        let identity = file_identity(&resolved)?;
        if !identities.insert(identity) {
            return Err(TuneSnapshotError::DuplicateInput);
        }
        let bytes = read_regular_nofollow(&resolved, MAX_INPUT_BYTES)?;
        total = total
            .checked_add(u64::try_from(bytes.len()).map_err(|_| TuneSnapshotError::ResourceLimit)?)
            .ok_or(TuneSnapshotError::ResourceLimit)?;
        if total > MAX_TOTAL_INPUT_BYTES {
            return Err(TuneSnapshotError::ResourceLimit);
        }
        let digest = Sha256::digest(&bytes).into();
        inputs.push(CapturedInput {
            logical_path: logical_path.clone(),
            bytes,
            digest,
        });
    }
    let manifest_digest = derive_manifest_digest(
        manifest,
        &environment,
        &inputs,
        runner_bytes.len(),
        runner_digest,
    )?;
    Ok(CapturedWorkload {
        runner_bytes,
        runner_digest,
        inputs,
        environment,
        manifest_digest,
        args: manifest.args.clone(),
        timeout_ms: manifest.timeout_ms,
        cases: manifest.cases.clone(),
    })
}

/// Creates a fresh read-only flat input set and exact CKTIMAP1 file.
///
/// # Errors
///
/// Fails if the destination exists, any write is partial, or rehashing differs.
pub fn stage_invocation_inputs(
    captured: &CapturedWorkload,
    run_root: &Path,
) -> Result<StagedInvocationInputs, TuneSnapshotError> {
    fs::create_dir(run_root)?;
    let input_root = run_root.join("inputs");
    fs::create_dir(&input_root)?;
    let mut files = Vec::with_capacity(captured.inputs.len());
    let mut entries = Vec::with_capacity(captured.inputs.len());
    let mut folded_names = BTreeSet::new();
    for (ordinal, input) in captured.inputs.iter().enumerate() {
        let basename = format!("{ordinal:08x}-{}.bin", hex(&input.digest));
        if !folded_names.insert(basename.to_ascii_lowercase()) {
            return Err(TuneSnapshotError::DuplicateInput);
        }
        let path = input_root.join(&basename);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.write_all(&input.bytes)?;
        file.sync_all()?;
        let mut permissions = file.metadata()?.permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions)?;
        let staged = read_regular_nofollow(&path, MAX_INPUT_BYTES)?;
        if Sha256::digest(&staged).as_slice() != input.digest {
            return Err(TuneSnapshotError::DigestMismatch);
        }
        entries.push(TuneInputMapEntry {
            logical_path: input.logical_path.clone(),
            staged_basename: basename,
            bytes: u64::try_from(input.bytes.len())
                .map_err(|_| TuneSnapshotError::ResourceLimit)?,
            digest: input.digest,
        });
        files.push(path);
    }
    let map_path = run_root.join("inputs.cktimap");
    let map_bytes = encode_input_map(&entries).map_err(|_| TuneSnapshotError::InputMap)?;
    let mut map = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&map_path)?;
    map.write_all(&map_bytes)?;
    map.sync_all()?;
    let mut permissions = map.metadata()?.permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&map_path, permissions)?;
    Ok(StagedInvocationInputs { files, map_path })
}

impl CapturedWorkload {
    /// Returns the immutable runner byte digest.
    #[must_use]
    pub const fn runner_digest(&self) -> [u8; 32] {
        self.runner_digest
    }

    /// Returns the immutable runner bytes for private staging only.
    #[must_use]
    pub fn runner_bytes(&self) -> &[u8] {
        &self.runner_bytes
    }

    /// Returns the canonical manifest-material digest.
    #[must_use]
    pub const fn manifest_digest(&self) -> [u8; 32] {
        self.manifest_digest
    }

    /// Returns only public environment identity, never inherited values.
    #[must_use]
    pub fn environment_identities(&self) -> Vec<&TuneEnvironmentIdentity> {
        self.environment
            .iter()
            .map(|entry| &entry.identity)
            .collect()
    }

    /// Returns path-safe identities without exposing captured input contents.
    #[must_use]
    pub fn input_identities(&self) -> Vec<TuneCapturedInputIdentity> {
        self.inputs
            .iter()
            .map(|input| TuneCapturedInputIdentity {
                logical_path: input.logical_path.clone(),
                bytes: u64::try_from(input.bytes.len()).unwrap_or(u64::MAX),
                digest: input.digest,
            })
            .collect()
    }

    /// Returns the accepted runner argv in execution order.
    #[must_use]
    pub fn runner_args(&self) -> &[String] {
        &self.args
    }

    /// Returns the accepted timeout.
    #[must_use]
    pub const fn invocation_timeout_ms(&self) -> u32 {
        self.timeout_ms
    }

    /// Returns canonical case identities.
    #[must_use]
    pub fn case_identities(&self) -> &[super::TuneCase] {
        &self.cases
    }

    pub(crate) fn args(&self) -> &[String] {
        &self.args
    }

    pub(crate) const fn timeout_ms(&self) -> u32 {
        self.timeout_ms
    }

    pub(crate) fn cases(&self) -> &[super::TuneCase] {
        &self.cases
    }

    pub(crate) fn environment_values(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.environment
            .iter()
            .map(|entry| (entry.identity.name.as_str(), entry.value.as_slice()))
    }
}

fn read_regular_nofollow(path: &Path, limit: u64) -> Result<Vec<u8>, TuneSnapshotError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(TuneSnapshotError::ResourceLimit);
    }
    let capacity = usize::try_from(metadata.len()).map_err(|_| TuneSnapshotError::ResourceLimit)?;
    let mut bytes = Vec::with_capacity(capacity);
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).map_err(|_| TuneSnapshotError::ResourceLimit)? > limit {
        return Err(TuneSnapshotError::ResourceLimit);
    }
    Ok(bytes)
}

fn validate_runner(_path: &Path, bytes: &[u8]) -> Result<(), TuneSnapshotError> {
    #[cfg(target_os = "linux")]
    let valid_format = bytes.starts_with(b"\x7fELF");
    #[cfg(target_os = "macos")]
    let valid_format = matches!(
        bytes.get(..4),
        Some(b"\xfe\xed\xfa\xcf" | b"\xcf\xfa\xed\xfe")
    );
    #[cfg(windows)]
    let valid_format = bytes.starts_with(b"MZ");
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    let valid_format = false;
    if !valid_format {
        return Err(TuneSnapshotError::InvalidRunner);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::metadata(_path)?.permissions().mode() & 0o111 == 0 {
            return Err(TuneSnapshotError::InvalidRunner);
        }
    }
    Ok(())
}

fn resolve_without_indirection(path: &Path) -> Result<PathBuf, TuneSnapshotError> {
    reject_symlink_components(path)?;
    path.canonicalize().map_err(TuneSnapshotError::from)
}

fn resolve_directory_without_indirection(path: &Path) -> Result<PathBuf, TuneSnapshotError> {
    let resolved = resolve_without_indirection(path)?;
    if !fs::metadata(&resolved)?.is_dir() {
        return Err(TuneSnapshotError::UnsafePath);
    }
    Ok(resolved)
}

fn resolve_beneath_without_indirection(
    root: &Path,
    path: &Path,
) -> Result<PathBuf, TuneSnapshotError> {
    reject_symlink_components(path)?;
    let resolved = path.canonicalize()?;
    if !resolved.starts_with(root) {
        return Err(TuneSnapshotError::UnsafePath);
    }
    Ok(resolved)
}

fn reject_symlink_components(path: &Path) -> Result<(), TuneSnapshotError> {
    let mut current = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_)
            | Component::RootDir
            | Component::CurDir
            | Component::ParentDir => {
                current.push(component.as_os_str());
            }
            Component::Normal(part) => {
                current.push(part);
                if fs::symlink_metadata(&current)?.file_type().is_symlink() {
                    return Err(TuneSnapshotError::UnsafePath);
                }
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn file_identity(path: &Path) -> Result<(u64, u64), TuneSnapshotError> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::metadata(path)?;
    Ok((metadata.dev(), metadata.ino()))
}

#[cfg(windows)]
fn file_identity(path: &Path) -> Result<(u64, u64), TuneSnapshotError> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };
    let file = fs::File::open(path)?;
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: the file handle and output pointer stay valid for the call.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &mut info) } == 0 {
        return Err(TuneSnapshotError::Io(std::io::Error::last_os_error()));
    }
    Ok((
        u64::from(info.dwVolumeSerialNumber),
        (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
    ))
}

#[cfg(not(any(unix, windows)))]
fn file_identity(path: &Path) -> Result<(u64, u64), TuneSnapshotError> {
    let metadata = fs::metadata(path)?;
    Ok((metadata.len(), 0))
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

fn capture_environment(
    manifest: &TuneManifest,
) -> Result<Vec<CapturedEnvironment>, TuneSnapshotError> {
    let mut names = manifest.inherit_env.clone();
    #[cfg(windows)]
    {
        for required in ["SystemRoot", "WINDIR"] {
            if !names.iter().any(|name| name.eq_ignore_ascii_case(required)) {
                names.push(required.to_owned());
            }
        }
        names.sort_by_key(|name| name.to_ascii_lowercase());
    }
    #[cfg(not(windows))]
    names.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    if names.len() > 16 {
        return Err(TuneSnapshotError::ResourceLimit);
    }
    let mut total = 0usize;
    let mut entries = Vec::with_capacity(names.len());
    for name in names {
        let value = std::env::var_os(&name)
            .ok_or_else(|| TuneSnapshotError::MissingEnvironment(name.clone()))?;
        #[cfg(unix)]
        let value = {
            use std::os::unix::ffi::OsStringExt;
            value.into_vec()
        };
        #[cfg(windows)]
        let value = value
            .into_string()
            .map_err(|_| TuneSnapshotError::InvalidEnvironment)?
            .into_bytes();
        #[cfg(not(any(unix, windows)))]
        let value = value.to_string_lossy().into_owned().into_bytes();
        if value.len() > 4_096 || value.contains(&0) {
            return Err(TuneSnapshotError::InvalidEnvironment);
        }
        total = total
            .checked_add(value.len())
            .ok_or(TuneSnapshotError::ResourceLimit)?;
        if total > 65_536 {
            return Err(TuneSnapshotError::ResourceLimit);
        }
        let mut material = encode_text(&name)?;
        material.extend_from_slice(
            &u32::try_from(value.len())
                .map_err(|_| TuneSnapshotError::ResourceLimit)?
                .to_be_bytes(),
        );
        material.extend_from_slice(&value);
        let value_digest = domain_hash(b"CK-TUNE-ENV-VALUE\0", &material);
        entries.push(CapturedEnvironment {
            identity: TuneEnvironmentIdentity {
                name,
                value_bytes: u64::try_from(value.len())
                    .map_err(|_| TuneSnapshotError::ResourceLimit)?,
                value_digest,
            },
            value,
        });
    }
    Ok(entries)
}

fn derive_manifest_digest(
    manifest: &TuneManifest,
    environment: &[CapturedEnvironment],
    inputs: &[CapturedInput],
    runner_bytes: usize,
    runner_digest: [u8; 32],
) -> Result<[u8; 32], TuneSnapshotError> {
    let mut material = Vec::new();
    field(&mut material, 1, &1u32.to_be_bytes())?;
    field(
        &mut material,
        2,
        &encode_list(
            manifest
                .args
                .iter()
                .map(|argument| encode_text(argument))
                .collect::<Result<Vec<_>, _>>()?,
        )?,
    )?;
    let environment_records = environment
        .iter()
        .map(|entry| {
            let mut record = Vec::new();
            field(&mut record, 1, &encode_text(&entry.identity.name)?)?;
            field(&mut record, 2, &entry.identity.value_bytes.to_be_bytes())?;
            field(&mut record, 3, &entry.identity.value_digest)?;
            encode_record(&record)
        })
        .collect::<Result<Vec<_>, TuneSnapshotError>>()?;
    field(&mut material, 3, &encode_list(environment_records)?)?;
    field(&mut material, 4, &manifest.timeout_ms.to_be_bytes())?;
    let input_records = inputs
        .iter()
        .map(|input| {
            let mut record = Vec::new();
            field(&mut record, 1, &encode_text(&input.logical_path)?)?;
            field(&mut record, 2, &input.digest)?;
            field(
                &mut record,
                3,
                &u64::try_from(input.bytes.len())
                    .map_err(|_| TuneSnapshotError::ResourceLimit)?
                    .to_be_bytes(),
            )?;
            encode_record(&record)
        })
        .collect::<Result<Vec<_>, TuneSnapshotError>>()?;
    field(&mut material, 5, &encode_list(input_records)?)?;
    let case_records = manifest
        .cases
        .iter()
        .map(|case| {
            let mut record = Vec::new();
            field(&mut record, 1, &encode_text(&case.id)?)?;
            field(
                &mut record,
                2,
                &[match case.role {
                    TuneCaseRole::Search => 1,
                    TuneCaseRole::Validation => 2,
                }],
            )?;
            field(&mut record, 3, &case.seed.to_be_bytes())?;
            field(&mut record, 4, &case.weight.to_be_bytes())?;
            field(&mut record, 5, &case.expected_digest)?;
            encode_record(&record)
        })
        .collect::<Result<Vec<_>, TuneSnapshotError>>()?;
    field(&mut material, 6, &encode_list(case_records)?)?;
    field(
        &mut material,
        7,
        &u64::try_from(runner_bytes)
            .map_err(|_| TuneSnapshotError::ResourceLimit)?
            .to_be_bytes(),
    )?;
    field(&mut material, 8, &runner_digest)?;
    Ok(domain_hash(
        b"CK-TUNE-MANIFEST\0",
        &encode_record(&material)?,
    ))
}

fn field(output: &mut Vec<u8>, tag: u16, value: &[u8]) -> Result<(), TuneSnapshotError> {
    output.extend_from_slice(&tag.to_be_bytes());
    output.extend_from_slice(
        &u32::try_from(value.len())
            .map_err(|_| TuneSnapshotError::ResourceLimit)?
            .to_be_bytes(),
    );
    output.extend_from_slice(value);
    Ok(())
}

fn encode_text(value: &str) -> Result<Vec<u8>, TuneSnapshotError> {
    let mut output = u32::try_from(value.len())
        .map_err(|_| TuneSnapshotError::ResourceLimit)?
        .to_be_bytes()
        .to_vec();
    output.extend_from_slice(value.as_bytes());
    Ok(output)
}

fn encode_record(value: &[u8]) -> Result<Vec<u8>, TuneSnapshotError> {
    let mut output = u32::try_from(value.len())
        .map_err(|_| TuneSnapshotError::ResourceLimit)?
        .to_be_bytes()
        .to_vec();
    output.extend_from_slice(value);
    Ok(output)
}

fn encode_list(values: Vec<Vec<u8>>) -> Result<Vec<u8>, TuneSnapshotError> {
    let mut output = u32::try_from(values.len())
        .map_err(|_| TuneSnapshotError::ResourceLimit)?
        .to_be_bytes()
        .to_vec();
    for value in values {
        output.extend_from_slice(&value);
    }
    Ok(output)
}

fn domain_hash(domain: &[u8], value: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(value);
    hasher.finalize().into()
}
