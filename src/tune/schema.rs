/// CKTUNE decision-file magic.
pub const TUNE_DECISION_MAGIC: &[u8; 8] = b"CKTUNE01";
/// CKTUNE decision schema version.
pub const TUNE_DECISION_SCHEMA: u32 = 1;
/// Workload manifest schema version.
pub const TUNE_MANIFEST_SCHEMA: u32 = 1;
/// Measurement schema version.
pub const TUNE_MEASUREMENT_SCHEMA: u32 = 1;
/// Inspection schema version.
pub const TUNE_INSPECTION_SCHEMA: u32 = 1;
/// Tuning-plan schema version.
pub const TUNE_PLAN_SCHEMA: u32 = 1;
/// Maximum canonical decision-file size.
pub const MAX_TUNE_DECISION_BYTES: usize = 32 * 1024 * 1024;
/// Outer decision digest domain, including its terminal NUL.
pub const DECISION_DIGEST_DOMAIN: &[u8] = b"CK-TUNING-DECISION\0";
/// Policy digest domain, including its terminal NUL.
pub const POLICY_DIGEST_DOMAIN: &[u8] = b"CK-TUNE-POLICY\0";
/// Plan digest domain, including its terminal NUL.
pub const PLAN_DIGEST_DOMAIN: &[u8] = b"CK-TUNE-PLAN\0";

/// Closed CK 0.14 tuning budget presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuneBudget {
    Quick,
    Standard,
    Thorough,
}

/// Candidate and wall-clock limits selected by one tuning preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TuneContract {
    pub beam_width: u32,
    pub expansion_limit: u32,
    pub compile_attempt_limit: u32,
    pub measured_finalist_limit: u32,
    pub validation_entrant_limit: u32,
    pub wall_clock_ms: u64,
}

impl TuneBudget {
    /// Returns the immutable schema-1 limits for this preset.
    #[must_use]
    pub const fn contract(self) -> TuneContract {
        match self {
            Self::Quick => TuneContract {
                beam_width: 4,
                expansion_limit: 1_024,
                compile_attempt_limit: 8,
                measured_finalist_limit: 4,
                validation_entrant_limit: 2,
                wall_clock_ms: 600_000,
            },
            Self::Standard => TuneContract {
                beam_width: 8,
                expansion_limit: 4_096,
                compile_attempt_limit: 16,
                measured_finalist_limit: 8,
                validation_entrant_limit: 3,
                wall_clock_ms: 1_800_000,
            },
            Self::Thorough => TuneContract {
                beam_width: 16,
                expansion_limit: 16_384,
                compile_attempt_limit: 32,
                measured_finalist_limit: 16,
                validation_entrant_limit: 4,
                wall_clock_ms: 7_200_000,
            },
        }
    }
}
