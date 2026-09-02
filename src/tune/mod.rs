//! Bounded, reproducible offline auto-tuning records and algorithms.

mod decision;
mod input_map;
mod inspect;
mod manifest;
mod schema;
mod snapshot;

pub use decision::{TuneDecision, TuneDecisionError, decode_tune_decision, encode_tune_decision};
pub use input_map::{TuneInputMapEntry, TuneInputMapError, decode_input_map, encode_input_map};
pub use inspect::{inspect_tune_json, inspect_tune_text};
pub use manifest::{TuneCase, TuneCaseRole, TuneManifest, TuneManifestError};
pub use schema::{
    DECISION_DIGEST_DOMAIN, MAX_TUNE_DECISION_BYTES, PLAN_DIGEST_DOMAIN, POLICY_DIGEST_DOMAIN,
    TUNE_DECISION_MAGIC, TUNE_DECISION_SCHEMA, TUNE_INSPECTION_SCHEMA, TUNE_MANIFEST_SCHEMA,
    TUNE_MEASUREMENT_SCHEMA, TUNE_PLAN_SCHEMA, TuneBudget, TuneContract,
};
pub use snapshot::{
    CapturedWorkload, StagedInvocationInputs, TuneEnvironmentIdentity, TuneSnapshotError,
    capture_workload, stage_invocation_inputs,
};
