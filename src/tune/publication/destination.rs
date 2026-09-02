use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
};

use sha2::{Digest, Sha256};

use super::PublicationError;

const DESTINATION_DOMAIN: &[u8] = b"CK-TUNE-DESTINATION\0";
const OUTPUT_SET_DOMAIN: &[u8] = b"CK-TUNE-OUTPUT-SET\0";

/// Native artifact path shape accepted by the tuning publication layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuneArtifactPaths {
    pub primary: PathBuf,
    pub header: Option<PathBuf>,
    pub import_library: Option<PathBuf>,
}

/// Converts the shared Native path type without making tuning require Native support.
pub trait IntoTuneOutputPaths {
    fn into_tune_output_paths(self) -> TuneArtifactPaths;
}

impl IntoTuneOutputPaths for &TuneArtifactPaths {
    fn into_tune_output_paths(self) -> TuneArtifactPaths {
        self.clone()
    }
}

#[cfg(feature = "native-toolchain")]
impl IntoTuneOutputPaths for &crate::NativeArtifactPaths {
    fn into_tune_output_paths(self) -> TuneArtifactPaths {
        TuneArtifactPaths {
            primary: self.primary.clone(),
            header: self.header.clone(),
            import_library: self.import_library.clone(),
        }
    }
}

/// Publication roles in mandatory primary-last order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum PublicationRole {
    Decision = 0,
    Primary = 1,
    Header = 2,
    ImportLibrary = 3,
}

impl PublicationRole {
    pub(crate) const fn publication_rank(self) -> u8 {
        match self {
            Self::Decision => 0,
            Self::Header => 1,
            Self::ImportLibrary => 2,
            Self::Primary => 3,
        }
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Decision => "decision",
            Self::Primary => "primary",
            Self::Header => "header",
            Self::ImportLibrary => "import",
        }
    }
}

/// One canonical destination and its full lock identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDestination {
    pub role: PublicationRole,
    pub path: PathBuf,
    pub destination_id: [u8; 32],
    pub lookup_leaf: String,
    pub(crate) existing_identity: Option<(u128, u128)>,
}

/// Canonical same-directory decision plus artifact output set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuneOutputSet {
    parent: PathBuf,
    destinations: Vec<ResolvedDestination>,
    set_id: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ParentIdentity {
    platform: u8,
    volume: u128,
    file: u128,
    case_sensitive: bool,
}

impl TuneOutputSet {
    /// Resolves the complete output set and rejects path, namespace, and file aliases.
    pub fn resolve<P: IntoTuneOutputPaths>(
        paths: P,
        decision_path: &Path,
        protected_inputs: &[PathBuf],
    ) -> Result<Self, PublicationError> {
        let paths = paths.into_tune_output_paths();
        if paths.import_library.is_some() && paths.header.is_none() {
            return Err(PublicationError::InvalidDestination(
                "import library requires a header",
            ));
        }
        let output_kind = if paths.header.is_some() { 2 } else { 1 };
        let requested = [
            Some((PublicationRole::Decision, decision_path.to_path_buf())),
            paths.header.map(|path| (PublicationRole::Header, path)),
            paths
                .import_library
                .map(|path| (PublicationRole::ImportLibrary, path)),
            Some((PublicationRole::Primary, paths.primary)),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

        let first_parent = requested
            .first()
            .and_then(|(_, path)| path.parent())
            .ok_or(PublicationError::InvalidDestination("missing parent"))?;
        let parent = canonical_parent(first_parent)?;
        let parent_identity = parent_identity(&parent)?;
        let mut destinations = Vec::with_capacity(requested.len());
        for (role, path) in requested {
            let requested_parent = path
                .parent()
                .ok_or(PublicationError::InvalidDestination("missing parent"))?;
            if canonical_parent(requested_parent)? != parent {
                return Err(PublicationError::InvalidDestination(
                    "all outputs must share one canonical parent",
                ));
            }
            destinations.push(resolve_one(role, &parent, parent_identity, &path)?);
        }
        destinations.sort_by_key(|destination| destination.role.publication_rank());
        let ids = destinations
            .iter()
            .map(|destination| destination.destination_id)
            .collect::<BTreeSet<_>>();
        if ids.len() != destinations.len() || has_existing_alias(&destinations) {
            return Err(PublicationError::InvalidDestination(
                "duplicate or aliased output",
            ));
        }
        for protected in protected_inputs {
            if aliases_protected(&parent, parent_identity, &destinations, protected)? {
                return Err(PublicationError::InvalidDestination(
                    "output aliases a protected input",
                ));
            }
        }
        let set_id = output_set_id(output_kind, &destinations)?;
        Ok(Self {
            parent,
            destinations,
            set_id,
        })
    }

    #[must_use]
    pub fn parent(&self) -> &Path {
        &self.parent
    }

    #[must_use]
    pub const fn set_id(&self) -> [u8; 32] {
        self.set_id
    }

    #[must_use]
    pub fn destinations(&self) -> &[ResolvedDestination] {
        &self.destinations
    }

    pub(crate) fn primary_is_executable(&self) -> bool {
        !self
            .destinations
            .iter()
            .any(|destination| destination.role == PublicationRole::Header)
    }

    pub(crate) fn same_namespace(&self, other: &Self) -> bool {
        self.parent == other.parent
            && self.set_id == other.set_id
            && self.destinations.len() == other.destinations.len()
            && self
                .destinations
                .iter()
                .zip(&other.destinations)
                .all(|(left, right)| {
                    left.role == right.role
                        && left.path == right.path
                        && left.destination_id == right.destination_id
                        && left.lookup_leaf == right.lookup_leaf
                })
    }
}

fn resolve_one(
    role: PublicationRole,
    parent: &Path,
    parent_identity: ParentIdentity,
    requested: &Path,
) -> Result<ResolvedDestination, PublicationError> {
    let requested_leaf = requested.file_name().and_then(|name| name.to_str()).ok_or(
        PublicationError::InvalidDestination("leaf must be canonical ASCII"),
    )?;
    validate_leaf(requested_leaf)?;
    let requested_path = parent.join(requested_leaf);
    let (canonical_leaf, existing_identity) = match fs::symlink_metadata(&requested_path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(PublicationError::InvalidDestination(
                    "existing output is not a no-follow regular file",
                ));
            }
            let identity = file_identity(&requested_path, &metadata)?;
            let leaf = authoritative_leaf(parent, requested_leaf, identity, parent_identity)?;
            (leaf, Some(identity))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            (requested_leaf.to_string(), None)
        }
        Err(error) => return Err(error.into()),
    };
    validate_leaf(&canonical_leaf)?;
    let lookup_leaf = if parent_identity.case_sensitive {
        canonical_leaf.clone()
    } else {
        canonical_leaf.to_ascii_lowercase()
    };
    let destination_id = destination_id(parent_identity, &lookup_leaf)?;
    Ok(ResolvedDestination {
        role,
        path: parent.join(canonical_leaf),
        destination_id,
        lookup_leaf,
        existing_identity,
    })
}

fn canonical_parent(path: &Path) -> Result<PathBuf, PublicationError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(PublicationError::InvalidDestination(
                        "parent traversal escapes root",
                    ));
                }
            }
        }
    }
    let canonical = fs::canonicalize(&normalized)?;
    let metadata = fs::symlink_metadata(&normalized)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(PublicationError::InvalidDestination(
            "parent is not a no-follow directory",
        ));
    }
    Ok(canonical)
}

fn validate_leaf(leaf: &str) -> Result<(), PublicationError> {
    let bytes = leaf.as_bytes();
    if bytes.is_empty()
        || bytes.len() > 255
        || !bytes[0].is_ascii_alphanumeric()
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        || matches!(leaf, "." | "..")
        || leaf.starts_with(".ckc-tune-")
        || leaf.ends_with(['.', ' '])
        || is_windows_device_name(leaf)
    {
        return Err(PublicationError::InvalidDestination("illegal leaf"));
    }
    Ok(())
}

fn is_windows_device_name(leaf: &str) -> bool {
    let stem = leaf.split('.').next().unwrap_or(leaf).to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$")
        || stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|suffix| suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9'))
}

fn parent_identity(parent: &Path) -> Result<ParentIdentity, PublicationError> {
    let metadata = fs::metadata(parent)?;
    let (volume, file) = file_identity(parent, &metadata)?;
    Ok(ParentIdentity {
        platform: if cfg!(windows) { 2 } else { 1 },
        volume,
        file,
        case_sensitive: directory_case_sensitive(parent)?,
    })
}

#[cfg(unix)]
fn file_identity(_path: &Path, metadata: &fs::Metadata) -> Result<(u128, u128), PublicationError> {
    use std::os::unix::fs::MetadataExt;
    Ok((u128::from(metadata.dev()), u128::from(metadata.ino())))
}

#[cfg(windows)]
fn file_identity(path: &Path, _metadata: &fs::Metadata) -> Result<(u128, u128), PublicationError> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx,
    };

    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(0x0200_0000 | 0x0020_0000)
        .open(path)?;
    let mut info = FILE_ID_INFO::default();
    let result = unsafe {
        // SAFETY: the handle is live and `info` points to a correctly sized writable buffer.
        GetFileInformationByHandleEx(
            file.as_raw_handle().cast(),
            FileIdInfo,
            (&raw mut info).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if result == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok((
        u128::from(info.VolumeSerialNumber),
        u128::from_le_bytes(info.FileId.Identifier),
    ))
}

#[cfg(all(not(unix), not(windows)))]
fn file_identity(_path: &Path, _metadata: &fs::Metadata) -> Result<(u128, u128), PublicationError> {
    Err(PublicationError::Identity("unsupported file identity"))
}

#[cfg(target_os = "macos")]
fn directory_case_sensitive(parent: &Path) -> Result<bool, PublicationError> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(parent.as_os_str().as_bytes())
        .map_err(|_| PublicationError::InvalidDestination("NUL in parent"))?;
    // SAFETY: the path is a live NUL-terminated byte string and macOS defines
    // `_PC_CASE_SENSITIVE` as selector 11.
    let result = unsafe { libc::pathconf(path.as_ptr(), 11) };
    match result {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(PublicationError::Identity(
            "unknown directory case behavior",
        )),
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn directory_case_sensitive(_parent: &Path) -> Result<bool, PublicationError> {
    Ok(true)
}

#[cfg(windows)]
fn directory_case_sensitive(parent: &Path) -> Result<bool, PublicationError> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_CASE_SENSITIVE_INFO, FileCaseSensitiveInfo, GetFileInformationByHandleEx,
    };

    let directory = fs::OpenOptions::new()
        .read(true)
        .custom_flags(0x0200_0000 | 0x0020_0000)
        .open(parent)?;
    let mut info = FILE_CASE_SENSITIVE_INFO::default();
    let result = unsafe {
        // SAFETY: the directory handle and correctly sized output buffer are live.
        GetFileInformationByHandleEx(
            directory.as_raw_handle().cast(),
            FileCaseSensitiveInfo,
            (&raw mut info).cast(),
            std::mem::size_of::<FILE_CASE_SENSITIVE_INFO>() as u32,
        )
    };
    if result == 0 {
        return Err(PublicationError::Identity(
            "unknown directory case behavior",
        ));
    }
    Ok(info.Flags & 1 != 0)
}

#[cfg(all(not(unix), not(windows)))]
fn directory_case_sensitive(_parent: &Path) -> Result<bool, PublicationError> {
    Err(PublicationError::Identity(
        "unknown directory case behavior",
    ))
}

#[cfg(not(windows))]
fn authoritative_leaf(
    parent: &Path,
    requested: &str,
    identity: (u128, u128),
    parent_identity: ParentIdentity,
) -> Result<String, PublicationError> {
    let mut matches = Vec::new();
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if file_identity(&entry.path(), &metadata)? == identity {
            let leaf = entry
                .file_name()
                .into_string()
                .map_err(|_| PublicationError::InvalidDestination("non-UTF-8 leaf"))?;
            if leaf == requested
                || (!parent_identity.case_sensitive && leaf.eq_ignore_ascii_case(requested))
            {
                matches.push(leaf);
            }
        }
    }
    matches.sort();
    matches.into_iter().next().ok_or(PublicationError::Identity(
        "existing destination has no authoritative directory entry",
    ))
}

#[cfg(windows)]
fn authoritative_leaf(
    parent: &Path,
    requested: &str,
    identity: (u128, u128),
    parent_identity: ParentIdentity,
) -> Result<String, PublicationError> {
    use std::{
        ffi::OsString,
        os::windows::ffi::{OsStrExt, OsStringExt},
    };
    use windows_sys::Win32::Storage::FileSystem::{GetLongPathNameW, GetShortPathNameW};

    fn query_path(
        path: &Path,
        query: unsafe extern "system" fn(*const u16, *mut u16, u32) -> u32,
    ) -> Result<PathBuf, PublicationError> {
        let input = path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let mut output = vec![0u16; 32_768];
        let count = unsafe {
            // SAFETY: `input` is NUL-terminated and `output` is writable for its declared size.
            query(input.as_ptr(), output.as_mut_ptr(), output.len() as u32)
        };
        let count = usize::try_from(count)
            .map_err(|_| PublicationError::Identity("Windows path length overflow"))?;
        if count == 0 || count >= output.len() {
            return Err(PublicationError::Identity(
                "unsupported Windows long/short-name query",
            ));
        }
        output.truncate(count);
        Ok(PathBuf::from(OsString::from_wide(&output)))
    }

    let requested_path = parent.join(requested);
    let long_path = query_path(&requested_path, GetLongPathNameW)?;
    let long_metadata = fs::symlink_metadata(&long_path)?;
    if long_metadata.file_type().is_symlink()
        || !long_metadata.is_file()
        || file_identity(&long_path, &long_metadata)? != identity
    {
        return Err(PublicationError::Identity(
            "inconsistent Windows authoritative long name",
        ));
    }
    let short_path = query_path(&long_path, GetShortPathNameW)?;
    let long_leaf = long_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or(PublicationError::Identity("non-UTF-8 Windows long leaf"))?;
    let short_leaf = short_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or(PublicationError::Identity("non-UTF-8 Windows short leaf"))?;
    let matches = if parent_identity.case_sensitive {
        requested == long_leaf || requested == short_leaf
    } else {
        requested.eq_ignore_ascii_case(long_leaf) || requested.eq_ignore_ascii_case(short_leaf)
    };
    if !matches {
        return Err(PublicationError::Identity(
            "requested Windows leaf is not an authoritative spelling",
        ));
    }
    Ok(long_leaf.to_string())
}

fn has_existing_alias(destinations: &[ResolvedDestination]) -> bool {
    for (index, left) in destinations.iter().enumerate() {
        for right in &destinations[index + 1..] {
            if left.existing_identity.is_some() && left.existing_identity == right.existing_identity
            {
                return true;
            }
        }
    }
    false
}

fn aliases_protected(
    output_parent: &Path,
    parent_identity: ParentIdentity,
    destinations: &[ResolvedDestination],
    protected: &Path,
) -> Result<bool, PublicationError> {
    let protected_absolute = if protected.is_absolute() {
        protected.to_path_buf()
    } else {
        std::env::current_dir()?.join(protected)
    };
    if let Ok(metadata) = fs::metadata(&protected_absolute) {
        let identity = file_identity(&protected_absolute, &metadata)?;
        if destinations
            .iter()
            .any(|destination| destination.existing_identity == Some(identity))
        {
            return Ok(true);
        }
    }
    let Some(leaf) = protected_absolute
        .file_name()
        .and_then(|leaf| leaf.to_str())
    else {
        return Ok(false);
    };
    let Some(parent) = protected_absolute.parent() else {
        return Ok(false);
    };
    let Ok(parent) = canonical_parent(parent) else {
        return Ok(false);
    };
    if parent != output_parent {
        return Ok(false);
    }
    let lookup = if parent_identity.case_sensitive {
        leaf.to_string()
    } else {
        leaf.to_ascii_lowercase()
    };
    Ok(destinations
        .iter()
        .any(|destination| destination.lookup_leaf == lookup))
}

fn destination_id(parent: ParentIdentity, lookup_leaf: &str) -> Result<[u8; 32], PublicationError> {
    let mut parent_fields = Vec::new();
    field(&mut parent_fields, 1, &[parent.platform])?;
    field(&mut parent_fields, 2, &parent.volume.to_be_bytes())?;
    field(&mut parent_fields, 3, &parent.file.to_be_bytes())?;
    field(
        &mut parent_fields,
        4,
        &[if parent.case_sensitive { 1 } else { 2 }],
    )?;
    let parent_record = record(&parent_fields)?;
    let mut key_fields = Vec::new();
    field(&mut key_fields, 1, &parent_record)?;
    field(&mut key_fields, 2, &text(lookup_leaf)?)?;
    Ok(domain_hash(DESTINATION_DOMAIN, &record(&key_fields)?))
}

fn output_set_id(
    output_kind: u8,
    destinations: &[ResolvedDestination],
) -> Result<[u8; 32], PublicationError> {
    let decision = destinations
        .iter()
        .find(|destination| destination.role == PublicationRole::Decision)
        .ok_or(PublicationError::Identity("missing decision destination"))?;
    let mut artifact_destinations = destinations
        .iter()
        .filter(|destination| destination.role != PublicationRole::Decision)
        .collect::<Vec<_>>();
    artifact_destinations.sort_by_key(|destination| destination.role as u8);
    let mut list = u32::try_from(artifact_destinations.len())
        .map_err(|_| PublicationError::Identity("too many destinations"))?
        .to_be_bytes()
        .to_vec();
    for destination in artifact_destinations {
        let mut fields = Vec::new();
        field(&mut fields, 1, &[destination.role as u8])?;
        field(&mut fields, 2, &destination.destination_id)?;
        list.extend_from_slice(&record(&fields)?);
    }
    let mut fields = Vec::new();
    field(&mut fields, 1, &[output_kind])?;
    field(&mut fields, 2, &decision.destination_id)?;
    field(&mut fields, 3, &list)?;
    Ok(domain_hash(OUTPUT_SET_DOMAIN, &record(&fields)?))
}

fn domain_hash(domain: &[u8], value: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(value);
    hasher.finalize().into()
}

fn field(output: &mut Vec<u8>, tag: u16, value: &[u8]) -> Result<(), PublicationError> {
    output.extend_from_slice(&tag.to_be_bytes());
    output.extend_from_slice(
        &u32::try_from(value.len())
            .map_err(|_| PublicationError::Identity("canonical field overflow"))?
            .to_be_bytes(),
    );
    output.extend_from_slice(value);
    Ok(())
}

fn record(value: &[u8]) -> Result<Vec<u8>, PublicationError> {
    let mut output = u32::try_from(value.len())
        .map_err(|_| PublicationError::Identity("canonical record overflow"))?
        .to_be_bytes()
        .to_vec();
    output.extend_from_slice(value);
    Ok(output)
}

fn text(value: &str) -> Result<Vec<u8>, PublicationError> {
    let mut output = u32::try_from(value.len())
        .map_err(|_| PublicationError::Identity("canonical text overflow"))?
        .to_be_bytes()
        .to_vec();
    output.extend_from_slice(value.as_bytes());
    Ok(output)
}
