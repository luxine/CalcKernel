//! Bounded, reproducible offline auto-tuning records and algorithms.

mod artifact;
mod cache;
mod calibration;
mod decision;
mod decision_encode;
mod environment;
mod frontier;
mod input_map;
mod inspect;
mod manifest;
mod measure;
mod plan;
mod publication;
mod replay;
mod runner;
mod schema;
mod search;
mod selection;
mod session;
mod snapshot;
mod trial;

pub use artifact::{
    ArtifactIdentity, TuneArtifactKind, TuneArtifactRole, TuneArtifactRoleIdentity,
};
pub use cache::{
    CachedTuneEntry, TUNE_CACHE_HARD_LIMIT, TUNE_CACHE_NAMESPACE, TuneCache, TuneCacheDomain,
    TuneCacheKey, TuneCacheReceipt,
};
pub use calibration::{
    CalibrationObservation, CalibrationRecord, calibrate_case_observations, calibrate_cases,
};
pub use decision::{
    TuneDecision, TuneDecisionError, TuneReplayOutput, TuneReplayRequirements,
    decode_tune_decision, encode_tune_decision,
};
pub use decision_encode::{
    TuneDecisionBuildInput, TuneDecisionCandidate, TuneDecisionIdentity, TuneDecisionOutput,
    TuneRecordedCacheOrigin, derive_tune_session_digest, encode_completed_tune_decision,
};
pub use environment::{BaselineSessionSeed, SessionDigestMaterial, derive_session_digest};
pub use frontier::canonical_frontier_digest;
pub use input_map::{TuneInputMapEntry, TuneInputMapError, decode_input_map, encode_input_map};
pub use inspect::{inspect_tune_json, inspect_tune_text};
pub use manifest::{TuneCase, TuneCaseRole, TuneManifest, TuneManifestError};
pub use measure::{
    MeasurementChannel, MeasurementCoordinate, MeasurementEvent, MeasurementEventOutcome,
    MeasurementFailure, MeasurementPhase, MeasurementRow, MeasurementRun, MeasurementScheduler,
    MeasurementStream, TimeoutRecord, verify_search_measurement_run,
};
pub(crate) use plan::plan_digest;
pub use plan::{TunePlanChoice, TuningPlan};
pub use publication::{
    IntoTuneOutputPaths, JournalPhase, PublicationError, PublicationFault, PublicationJournal,
    PublicationRole, PublicationSet, RecoveryDirection, ResolvedDestination, TuneArtifactPaths,
    TuneOutputSet, TunePublishArtifacts, decode_publication_journal, encode_publication_journal,
};
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
pub use selection::{
    CandidateOutcome, CandidateRank, CaseMedian, RoundPlan, RoundSummary, SearchEntrant, Selection,
    SelectionEntrant, SelectionError, SelectionReason, StreamStatistics, derive_round_summary,
    derive_search_entrants, derive_selection, stream_statistics,
};
pub use session::{DecisionAssemblyError, assemble_decision};
pub use snapshot::{
    CapturedWorkload, StagedInvocationInputs, TuneCapturedInputIdentity, TuneEnvironmentIdentity,
    TuneSnapshotError, capture_workload, stage_invocation_inputs,
};
pub use trial::{NonPublishableTuneTrial, TuneTrialBuildRequest, compile_tune_trial};
