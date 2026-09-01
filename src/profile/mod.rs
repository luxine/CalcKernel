//! CK-owned workload profile identities, wire formats, merge, and inspection.

mod analysis;
mod apply;
mod cost;
mod format;
mod generation;
mod identity;
mod inspect;
mod merge;

use std::path::Path;

use thiserror::Error;

pub use analysis::{
    CkProfileAnalysis, CkProfileAnalyzedSite, CkProfileFunctionWork, CkProfileObservation,
    CkProfileUnknownReason, CkProfileWorkTerm, profile_is_cold, profile_ratio_at_least,
    profile_site_dominant_outcome,
};
pub use apply::{
    CkProfileMappingTransfer, CkProfileTransferEntry, CkTransferredProfileCounter, apply_profile,
    transfer_profile_counts,
};
pub use cost::{
    CkAffineCostFormula, CkProfileCostClass, CkProfileCostDecision, CkProfileCostDomain,
    CkProfileCostProposal, CkSignedMagnitude, profile_histogram_bucket_range,
    verify_profile_cost_proposal,
};
pub use format::{
    CkProfile, CkProfileCounter, CkProfileCounterRecord, CkProfileShard, CkProfileSiteDescriptor,
    CkProfileSiteId, CkProfileSiteKind, parse_profile, parse_profile_shard,
    profile_site_table_digest, serialize_profile, serialize_profile_shard,
};
pub use generation::{
    CK_PROFILE_NO_WIRE_OFFSET, CkProfileDirectoryAnchor, CkProfileDirectoryIdentity,
    CkProfileShardTemplate, anchor_profile_directory, create_profile_shard_template,
    profile_histogram_bucket,
};
pub use identity::{
    CK_PROFILE_MAX_BYTES, CK_PROFILE_MAX_CANDIDATES, CK_PROFILE_MAX_SHARDS, CK_PROFILE_MAX_SITES,
    CkCompilerProfileIdentity, CkModuleProfileIdentity, CkProfileContract, CkProfileCpuPolicy,
    CkProfileEndianness, CkProfileIdentity, CkProfileModes, CkProfileObjectFormat,
    CkProfileOptimizationFamily, CkProfileSchemaIdentity, CkProfileTargetIdentity,
    CkProfileTopology,
};
pub use inspect::{inspect_profile_json, inspect_profile_text};
pub use merge::{
    CkProfileMergeOutput, merge_profile_inputs, merge_profile_shards, read_profile_input,
    validate_profile_output_path,
};

/// A stable validation or I/O failure in the CK profile subsystem.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CkProfileError {
    /// The input has the wrong profile or shard magic.
    #[error("profile magic is invalid for this operation")]
    UnexpectedMagic,
    /// The outer or nested schema is unsupported.
    #[error("unsupported {kind} schema {observed}; expected {expected}")]
    UnsupportedSchema {
        /// Schema family.
        kind: &'static str,
        /// Supported value.
        expected: u32,
        /// Input value.
        observed: u32,
    },
    /// The final SHA-256 does not cover the input bytes.
    #[error("profile digest mismatch")]
    DigestMismatch,
    /// The input ended before a declared field was complete.
    #[error("profile input is truncated")]
    Truncated,
    /// A checked length or count exceeded the frozen resource contract.
    #[error("profile resource limit exceeded: {0}")]
    ResourceLimit(&'static str),
    /// An integer length or offset overflowed.
    #[error("profile length arithmetic overflow")]
    LengthOverflow,
    /// A TLV tag is unknown in the current schema.
    #[error("unknown profile field {tag} in {context}")]
    UnknownField {
        /// Stable nesting context.
        context: &'static str,
        /// Numeric field tag.
        tag: u16,
    },
    /// Fields or records are not in canonical order.
    #[error("non-canonical profile ordering in {0}")]
    NonCanonicalOrder(&'static str),
    /// A required field is absent.
    #[error("missing required profile field {field} in {context}")]
    MissingField {
        /// Stable nesting context.
        context: &'static str,
        /// Numeric field tag.
        field: u16,
    },
    /// A fixed enum or boolean value is not valid.
    #[error("invalid profile value for {0}")]
    InvalidValue(&'static str),
    /// A profile string is not canonical UTF-8.
    #[error("profile string is not valid UTF-8")]
    InvalidUtf8,
    /// Two inputs contain the same completed run identity.
    #[error("duplicate profile run identity")]
    DuplicateRunIdentity,
    /// The same canonical shard bytes were supplied twice.
    #[error("duplicate profile shard content")]
    DuplicateShardContent,
    /// A shard identity differs from the first input.
    #[error("profile identity mismatch at {field}: expected {expected}, observed {observed}")]
    IdentityMismatch {
        /// First stable mismatching field path.
        field: &'static str,
        /// Expected canonical value or digest.
        expected: String,
        /// Observed canonical value or digest.
        observed: String,
    },
    /// Two site tables are not byte-identical.
    #[error("profile site table mismatch")]
    SiteTableMismatch,
    /// Counters do not match the authoritative site descriptor table.
    #[error("profile counter table does not match its site table")]
    CounterTableMismatch,
    /// A parsed profile is not compatible with the canonical use topology.
    #[error("profile application failed: {0}")]
    Application(&'static str),
    /// A CFG count-transfer record is absent or malformed.
    #[error("profile mapping transfer is invalid: {0}")]
    MappingTransfer(&'static str),
    /// Checked integer analysis could not represent the required value.
    #[error("profile arithmetic overflow in {0}")]
    ArithmeticOverflow(&'static str),
    /// A symbolic site ID names more than one descriptor.
    #[error("profile site identifier collision")]
    SiteIdCollision,
    /// Merge input is a symlink or reparse-like indirection.
    #[error("profile input is a symbolic link: {0}")]
    SymlinkInput(String),
    /// A directory entry has an unsupported relevant profile suffix or type.
    #[error("unsupported profile merge input: {0}")]
    UnsupportedMergeInput(String),
    /// A filesystem operation failed.
    #[error("profile I/O failed for {path}: {message}")]
    Io {
        /// Affected path.
        path: String,
        /// Platform error text.
        message: String,
    },
}

impl CkProfileError {
    pub(crate) fn io(path: &Path, error: std::io::Error) -> Self {
        Self::Io {
            path: path.display().to_string(),
            message: error.to_string(),
        }
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}
