use std::{fs, io::Read, path::PathBuf};

use sha2::{Digest, Sha256};

use super::{
    PublicationError, PublicationRole, TuneOutputSet, lock::hex,
    platform::open_regular_nofollow_read,
};

const JOURNAL_MAGIC: &[u8; 8] = b"CKTJNL01";
const JOURNAL_DOMAIN: &[u8] = b"CK-TUNE-JOURNAL\0";
const JOURNAL_SCHEMA: u32 = 1;
const MAX_JOURNAL_BYTES: usize = 128 * 1024;

/// Role-tagged final artifact bytes. Decision bytes remain a separate verified input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunePublishArtifacts {
    pub primary: Vec<u8>,
    pub header: Option<Vec<u8>>,
    pub import_library: Option<Vec<u8>>,
}

/// Durable publication phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum JournalPhase {
    Prepared = 1,
    BackedUp = 2,
    DecisionPublished = 3,
    SidecarsPublished = 4,
    PrimaryPublished = 5,
    Committed = 6,
}

impl JournalPhase {
    fn parse(value: u8) -> Result<Self, PublicationError> {
        match value {
            1 => Ok(Self::Prepared),
            2 => Ok(Self::BackedUp),
            3 => Ok(Self::DecisionPublished),
            4 => Ok(Self::SidecarsPublished),
            5 => Ok(Self::PrimaryPublished),
            6 => Ok(Self::Committed),
            _ => Err(PublicationError::Identity("journal phase")),
        }
    }

    fn successor(self) -> Option<Self> {
        match self {
            Self::Prepared => Some(Self::BackedUp),
            Self::BackedUp => Some(Self::DecisionPublished),
            Self::DecisionPublished => Some(Self::SidecarsPublished),
            Self::SidecarsPublished => Some(Self::PrimaryPublished),
            Self::PrimaryPublished => Some(Self::Committed),
            Self::Committed => None,
        }
    }
}

/// Recovery direction made durable in the journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RecoveryDirection {
    Forward = 1,
    Rollback = 2,
}

impl RecoveryDirection {
    fn parse(value: u8) -> Result<Self, PublicationError> {
        match value {
            1 => Ok(Self::Forward),
            2 => Ok(Self::Rollback),
            _ => Err(PublicationError::Identity("journal direction")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ContentIdentity {
    pub present: bool,
    pub digest: [u8; 32],
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JournalDestination {
    pub role: PublicationRole,
    pub path: PathBuf,
    pub destination_id: [u8; 32],
    pub stage_basename: String,
    pub backup_basename: String,
    pub old: ContentIdentity,
    pub new: ContentIdentity,
}

/// Exact bounded CKTJNL01 state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationJournal {
    generation: u64,
    phase: JournalPhase,
    direction: RecoveryDirection,
    transaction_id: [u8; 16],
    set_id: [u8; 32],
    destinations: Vec<JournalDestination>,
}

impl PublicationJournal {
    /// Freezes old/new identities and exact sibling names for Prepared generation 1.
    pub fn prepared(
        output: &TuneOutputSet,
        transaction_id: [u8; 16],
        decision_bytes: &[u8],
        artifacts: &TunePublishArtifacts,
    ) -> Result<Self, PublicationError> {
        validate_artifact_shape(output, artifacts)?;
        let set_hex = hex(&output.set_id());
        let tx_hex = hex(&transaction_id);
        let mut destinations = Vec::with_capacity(output.destinations().len());
        for destination in output.destinations() {
            let bytes = match destination.role {
                PublicationRole::Decision => decision_bytes,
                PublicationRole::Primary => &artifacts.primary,
                PublicationRole::Header => artifacts
                    .header
                    .as_deref()
                    .ok_or(PublicationError::Identity("missing header bytes"))?,
                PublicationRole::ImportLibrary => artifacts
                    .import_library
                    .as_deref()
                    .ok_or(PublicationError::Identity("missing import-library bytes"))?,
            };
            if bytes.is_empty() {
                return Err(PublicationError::Identity("empty publication output"));
            }
            destinations.push(JournalDestination {
                role: destination.role,
                path: destination.path.clone(),
                destination_id: destination.destination_id,
                stage_basename: format!(
                    ".ckc-tune-set-{set_hex}.{tx_hex}.{}.stage",
                    destination.role.name()
                ),
                backup_basename: format!(
                    ".ckc-tune-set-{set_hex}.{tx_hex}.{}.backup",
                    destination.role.name()
                ),
                old: identity_at(&destination.path)?,
                new: content_identity(bytes)?,
            });
        }
        let journal = Self {
            generation: 1,
            phase: JournalPhase::Prepared,
            direction: RecoveryDirection::Forward,
            transaction_id,
            set_id: output.set_id(),
            destinations,
        };
        journal.validate()?;
        Ok(journal)
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn phase(&self) -> JournalPhase {
        self.phase
    }

    #[must_use]
    pub const fn direction(&self) -> RecoveryDirection {
        self.direction
    }

    #[must_use]
    pub const fn set_id(&self) -> [u8; 32] {
        self.set_id
    }

    #[must_use]
    pub const fn transaction_id(&self) -> [u8; 16] {
        self.transaction_id
    }

    pub(crate) fn destinations(&self) -> &[JournalDestination] {
        &self.destinations
    }

    /// Creates the exact next forward successor.
    pub fn advance(&self, phase: JournalPhase) -> Result<Self, PublicationError> {
        if self.direction != RecoveryDirection::Forward || self.phase.successor() != Some(phase) {
            return Err(PublicationError::Identity("journal forward successor"));
        }
        let mut next = self.clone();
        next.generation = next
            .generation
            .checked_add(1)
            .ok_or(PublicationError::Identity("journal generation overflow"))?;
        next.phase = phase;
        next.validate()?;
        Ok(next)
    }

    /// Makes the only legal forward-to-rollback transition durable.
    pub fn begin_rollback(&self) -> Result<Self, PublicationError> {
        if self.direction != RecoveryDirection::Forward
            || self.phase >= JournalPhase::PrimaryPublished
        {
            return Err(PublicationError::Identity("journal rollback transition"));
        }
        let mut rollback = self.clone();
        rollback.direction = RecoveryDirection::Rollback;
        rollback.generation = rollback
            .generation
            .checked_add(1)
            .ok_or(PublicationError::Identity("journal generation overflow"))?;
        rollback.validate()?;
        Ok(rollback)
    }

    pub(crate) fn validate_against(&self, output: &TuneOutputSet) -> Result<(), PublicationError> {
        if self.set_id != output.set_id()
            || self.destinations.len() != output.destinations().len()
            || self
                .destinations
                .iter()
                .zip(output.destinations())
                .any(|(journal, resolved)| {
                    journal.role != resolved.role
                        || journal.path != resolved.path
                        || journal.destination_id != resolved.destination_id
                })
        {
            return Err(PublicationError::Identity("journal output-set identity"));
        }
        Ok(())
    }

    pub(crate) fn revalidate_paths(&self) -> Result<TuneOutputSet, PublicationError> {
        let decision = self
            .destinations
            .iter()
            .find(|destination| destination.role == PublicationRole::Decision)
            .ok_or(PublicationError::Identity("journal decision destination"))?;
        let primary = self
            .destinations
            .iter()
            .find(|destination| destination.role == PublicationRole::Primary)
            .ok_or(PublicationError::Identity("journal primary destination"))?;
        let paths = super::TuneArtifactPaths {
            primary: primary.path.clone(),
            header: self
                .destinations
                .iter()
                .find(|destination| destination.role == PublicationRole::Header)
                .map(|destination| destination.path.clone()),
            import_library: self
                .destinations
                .iter()
                .find(|destination| destination.role == PublicationRole::ImportLibrary)
                .map(|destination| destination.path.clone()),
        };
        let output = TuneOutputSet::resolve(&paths, &decision.path, &[])?;
        self.validate_against(&output)?;
        Ok(output)
    }

    pub(crate) fn is_successor_of(&self, prior: &Self) -> bool {
        if self.transaction_id != prior.transaction_id
            || self.set_id != prior.set_id
            || self.destinations != prior.destinations
            || self.generation != prior.generation + 1
        {
            return false;
        }
        (self.direction == RecoveryDirection::Forward
            && prior.direction == RecoveryDirection::Forward
            && prior.phase.successor() == Some(self.phase))
            || (self.direction == RecoveryDirection::Rollback
                && prior.direction == RecoveryDirection::Forward
                && self.phase == prior.phase
                && prior.phase < JournalPhase::PrimaryPublished)
    }

    fn validate(&self) -> Result<(), PublicationError> {
        let generation_valid = match self.direction {
            RecoveryDirection::Forward => self.generation == self.phase as u64,
            RecoveryDirection::Rollback => {
                self.phase < JournalPhase::PrimaryPublished
                    && self.generation == u64::from(self.phase as u8) + 1
            }
        };
        if !generation_valid || !valid_role_layout(&self.destinations) {
            return Err(PublicationError::Identity("journal state"));
        }
        let set_hex = hex(&self.set_id);
        let tx_hex = hex(&self.transaction_id);
        for destination in &self.destinations {
            if !destination.path.is_absolute()
                || destination.stage_basename
                    != format!(
                        ".ckc-tune-set-{set_hex}.{tx_hex}.{}.stage",
                        destination.role.name()
                    )
                || destination.backup_basename
                    != format!(
                        ".ckc-tune-set-{set_hex}.{tx_hex}.{}.backup",
                        destination.role.name()
                    )
                || (!destination.old.present
                    && (destination.old.digest != [0; 32] || destination.old.size != 0))
                || (destination.old.present && destination.old.digest == [0; 32])
                || !destination.new.present
                || destination.new.digest == [0; 32]
            {
                return Err(PublicationError::Identity("journal destination"));
            }
        }
        Ok(())
    }
}

/// Encodes exact CKTJNL01 bytes with trailing domain-separated SHA-256.
pub fn encode_publication_journal(
    journal: &PublicationJournal,
) -> Result<Vec<u8>, PublicationError> {
    journal.validate()?;
    let mut output = Vec::new();
    output.extend_from_slice(JOURNAL_MAGIC);
    output.extend_from_slice(&JOURNAL_SCHEMA.to_be_bytes());
    output.extend_from_slice(&journal.generation.to_be_bytes());
    output.push(journal.phase as u8);
    output.push(journal.direction as u8);
    output.extend_from_slice(&journal.transaction_id);
    output.extend_from_slice(&journal.set_id);
    output.push(
        u8::try_from(journal.destinations.len())
            .map_err(|_| PublicationError::Identity("journal destination count"))?,
    );
    for destination in &journal.destinations {
        output.push(destination.role as u8);
        push_blob(&mut output, &path_bytes(&destination.path)?)?;
        output.extend_from_slice(&destination.destination_id);
        push_blob(&mut output, destination.stage_basename.as_bytes())?;
        push_blob(&mut output, destination.backup_basename.as_bytes())?;
        output.push(u8::from(destination.old.present));
        output.extend_from_slice(&destination.old.digest);
        output.extend_from_slice(&destination.old.size.to_be_bytes());
        output.extend_from_slice(&destination.new.digest);
        output.extend_from_slice(&destination.new.size.to_be_bytes());
    }
    if output.len() + 32 > MAX_JOURNAL_BYTES {
        return Err(PublicationError::Identity("journal size"));
    }
    let digest = domain_hash(JOURNAL_DOMAIN, &output);
    output.extend_from_slice(&digest);
    Ok(output)
}

/// Decodes and structurally validates bounded exact CKTJNL01 bytes.
pub fn decode_publication_journal(bytes: &[u8]) -> Result<PublicationJournal, PublicationError> {
    if bytes.len() > MAX_JOURNAL_BYTES || bytes.len() < 103 {
        return Err(PublicationError::Identity("journal size"));
    }
    let body_end = bytes.len() - 32;
    if domain_hash(JOURNAL_DOMAIN, &bytes[..body_end]) != bytes[body_end..] {
        return Err(PublicationError::Identity("journal digest"));
    }
    let mut cursor = Cursor::new(&bytes[..body_end]);
    if cursor.take(8)? != JOURNAL_MAGIC || cursor.u32()? != JOURNAL_SCHEMA {
        return Err(PublicationError::Identity("journal magic or schema"));
    }
    let generation = cursor.u64()?;
    let phase = JournalPhase::parse(cursor.u8()?)?;
    let direction = RecoveryDirection::parse(cursor.u8()?)?;
    let transaction_id = cursor.array()?;
    let set_id = cursor.array()?;
    let count = usize::from(cursor.u8()?);
    if !(2..=4).contains(&count) {
        return Err(PublicationError::Identity("journal destination count"));
    }
    let mut destinations = Vec::with_capacity(count);
    for _ in 0..count {
        let role = parse_role(cursor.u8()?)?;
        let path = bytes_path(cursor.blob(4_096)?)?;
        let destination_id = cursor.array()?;
        let stage_basename = ascii_name(cursor.blob(255)?)?;
        let backup_basename = ascii_name(cursor.blob(255)?)?;
        let old_present = match cursor.u8()? {
            0 => false,
            1 => true,
            _ => return Err(PublicationError::Identity("journal old-present flag")),
        };
        destinations.push(JournalDestination {
            role,
            path,
            destination_id,
            stage_basename,
            backup_basename,
            old: ContentIdentity {
                present: old_present,
                digest: cursor.array()?,
                size: cursor.u64()?,
            },
            new: ContentIdentity {
                present: true,
                digest: cursor.array()?,
                size: cursor.u64()?,
            },
        });
    }
    if !cursor.remaining().is_empty() {
        return Err(PublicationError::Identity("journal trailing bytes"));
    }
    let journal = PublicationJournal {
        generation,
        phase,
        direction,
        transaction_id,
        set_id,
        destinations,
    };
    journal.validate()?;
    journal.revalidate_paths()?;
    Ok(journal)
}

pub(crate) fn identity_at(path: &std::path::Path) -> Result<ContentIdentity, PublicationError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(PublicationError::Identity(
                    "publication member is not a regular file",
                ));
            }
            let mut file = open_regular_nofollow_read(path)?;
            let mut hasher = Sha256::new();
            let mut size = 0u64;
            let mut buffer = [0u8; 64 * 1024];
            loop {
                let read = file.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
                size = size
                    .checked_add(
                        u64::try_from(read)
                            .map_err(|_| PublicationError::Identity("content size overflow"))?,
                    )
                    .ok_or(PublicationError::Identity("content size overflow"))?;
            }
            Ok(ContentIdentity {
                present: true,
                digest: hasher.finalize().into(),
                size,
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(ContentIdentity {
            present: false,
            digest: [0; 32],
            size: 0,
        }),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn content_identity(bytes: &[u8]) -> Result<ContentIdentity, PublicationError> {
    Ok(ContentIdentity {
        present: true,
        digest: Sha256::digest(bytes).into(),
        size: u64::try_from(bytes.len())
            .map_err(|_| PublicationError::Identity("content size overflow"))?,
    })
}

fn validate_artifact_shape(
    output: &TuneOutputSet,
    artifacts: &TunePublishArtifacts,
) -> Result<(), PublicationError> {
    let roles = output
        .destinations()
        .iter()
        .map(|destination| destination.role)
        .collect::<Vec<_>>();
    if artifacts.primary.is_empty()
        || roles.contains(&PublicationRole::Header) != artifacts.header.is_some()
        || roles.contains(&PublicationRole::ImportLibrary) != artifacts.import_library.is_some()
    {
        return Err(PublicationError::Identity("artifact role layout"));
    }
    Ok(())
}

fn valid_role_layout(destinations: &[JournalDestination]) -> bool {
    let roles = destinations
        .iter()
        .map(|destination| destination.role)
        .collect::<Vec<_>>();
    matches!(
        roles.as_slice(),
        [PublicationRole::Decision, PublicationRole::Primary]
            | [
                PublicationRole::Decision,
                PublicationRole::Header,
                PublicationRole::Primary
            ]
            | [
                PublicationRole::Decision,
                PublicationRole::Header,
                PublicationRole::ImportLibrary,
                PublicationRole::Primary
            ]
    )
}

fn parse_role(value: u8) -> Result<PublicationRole, PublicationError> {
    match value {
        0 => Ok(PublicationRole::Decision),
        1 => Ok(PublicationRole::Primary),
        2 => Ok(PublicationRole::Header),
        3 => Ok(PublicationRole::ImportLibrary),
        _ => Err(PublicationError::Identity("journal role")),
    }
}

fn push_blob(output: &mut Vec<u8>, value: &[u8]) -> Result<(), PublicationError> {
    output.extend_from_slice(
        &u32::try_from(value.len())
            .map_err(|_| PublicationError::Identity("journal blob length"))?
            .to_be_bytes(),
    );
    output.extend_from_slice(value);
    Ok(())
}

#[cfg(unix)]
fn path_bytes(path: &std::path::Path) -> Result<Vec<u8>, PublicationError> {
    use std::os::unix::ffi::OsStrExt;
    let bytes = path.as_os_str().as_bytes();
    if bytes.contains(&0) || bytes.len() > 4_096 {
        return Err(PublicationError::Identity("journal path"));
    }
    Ok(bytes.to_vec())
}

#[cfg(unix)]
fn bytes_path(bytes: &[u8]) -> Result<PathBuf, PublicationError> {
    use std::os::unix::ffi::OsStringExt;
    if bytes.contains(&0) {
        return Err(PublicationError::Identity("journal path"));
    }
    Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec())))
}

#[cfg(windows)]
fn path_bytes(path: &std::path::Path) -> Result<Vec<u8>, PublicationError> {
    let text = path
        .to_str()
        .ok_or(PublicationError::Identity("journal Windows path"))?;
    if text.len() > 4_096 || text.contains('\0') {
        return Err(PublicationError::Identity("journal Windows path"));
    }
    Ok(text.as_bytes().to_vec())
}

#[cfg(windows)]
fn bytes_path(bytes: &[u8]) -> Result<PathBuf, PublicationError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| PublicationError::Identity("journal Windows path"))?;
    Ok(PathBuf::from(text))
}

#[cfg(all(not(unix), not(windows)))]
fn path_bytes(_path: &std::path::Path) -> Result<Vec<u8>, PublicationError> {
    Err(PublicationError::Identity("unsupported journal path"))
}

#[cfg(all(not(unix), not(windows)))]
fn bytes_path(_bytes: &[u8]) -> Result<PathBuf, PublicationError> {
    Err(PublicationError::Identity("unsupported journal path"))
}

fn ascii_name(bytes: &[u8]) -> Result<String, PublicationError> {
    if !bytes.is_ascii() || bytes.contains(&0) || bytes.contains(&b'/') || bytes.contains(&b'\\') {
        return Err(PublicationError::Identity("journal basename"));
    }
    String::from_utf8(bytes.to_vec()).map_err(|_| PublicationError::Identity("journal basename"))
}

fn domain_hash(domain: &[u8], bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    hasher.finalize().into()
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], PublicationError> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or(PublicationError::Identity("journal offset"))?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(PublicationError::Identity("truncated journal"))?;
        self.offset = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, PublicationError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, PublicationError> {
        Ok(u32::from_be_bytes(
            self.take(4)?.try_into().expect("four bytes"),
        ))
    }

    fn u64(&mut self) -> Result<u64, PublicationError> {
        Ok(u64::from_be_bytes(
            self.take(8)?.try_into().expect("eight bytes"),
        ))
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], PublicationError> {
        Ok(self.take(N)?.try_into().expect("exact array length"))
    }

    fn blob(&mut self, limit: usize) -> Result<&'a [u8], PublicationError> {
        let count = usize::try_from(self.u32()?)
            .map_err(|_| PublicationError::Identity("journal blob length"))?;
        if count > limit {
            return Err(PublicationError::Identity("journal blob bound"));
        }
        self.take(count)
    }

    fn remaining(&self) -> &'a [u8] {
        &self.bytes[self.offset..]
    }
}
