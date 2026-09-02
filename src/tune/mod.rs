//! Bounded, reproducible offline auto-tuning records and algorithms.

mod artifact;
mod calibration;
mod decision;
mod frontier;
mod input_map;
mod inspect;
mod manifest;
mod plan;
mod replay;
mod runner;
mod schema;
mod search;
mod snapshot;
mod trial;

pub use artifact::{
    ArtifactIdentity, TuneArtifactKind, TuneArtifactRole, TuneArtifactRoleIdentity,
};
pub use calibration::{
    CalibrationObservation, CalibrationRecord, calibrate_case_observations, calibrate_cases,
};
pub use decision::{TuneDecision, TuneDecisionError, decode_tune_decision, encode_tune_decision};
pub use frontier::canonical_frontier_digest;
pub use input_map::{TuneInputMapEntry, TuneInputMapError, decode_input_map, encode_input_map};
pub use inspect::{inspect_tune_json, inspect_tune_text};
pub use manifest::{TuneCase, TuneCaseRole, TuneManifest, TuneManifestError};
pub(crate) use plan::plan_digest;
pub use plan::{TunePlanChoice, TuningPlan};
pub use replay::{
    TuneFinalistSelection, select_size_valid_finalists, verify_tune_trials_with_source,
};
pub use runner::{
    CanonicalCandidateTimeout, InvocationResult, RunnerFailure, TuneInvocation, TuneRunner,
};
pub use schema::{
    DECISION_DIGEST_DOMAIN, MAX_TUNE_DECISION_BYTES, PLAN_DIGEST_DOMAIN, POLICY_DIGEST_DOMAIN,
    TUNE_DECISION_MAGIC, TUNE_DECISION_SCHEMA, TUNE_INSPECTION_SCHEMA, TUNE_MANIFEST_SCHEMA,
    TUNE_MEASUREMENT_SCHEMA, TUNE_PLAN_SCHEMA, TuneBudget, TuneContract,
};
pub use search::{ExpansionDisposition, ExpansionRecord, SearchFrontier, run_deterministic_search};
pub use snapshot::{
    CapturedWorkload, StagedInvocationInputs, TuneEnvironmentIdentity, TuneSnapshotError,
    capture_workload, stage_invocation_inputs,
};
pub use trial::{NonPublishableTuneTrial, TuneTrialBuildRequest, compile_tune_trial};
