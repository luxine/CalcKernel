mod destination;
mod journal;
mod lock;
mod platform;
mod recovery;

pub use destination::{
    IntoTuneOutputPaths, PublicationRole, ResolvedDestination, TuneArtifactPaths, TuneOutputSet,
};
pub use journal::{
    JournalPhase, PublicationJournal, RecoveryDirection, TunePublishArtifacts,
    decode_publication_journal, encode_publication_journal,
};
pub use lock::PublicationSet;
pub use recovery::PublicationFault;

/// Stable fail-closed tuning publication failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PublicationError {
    #[error("invalid tuning destination: {0}")]
    InvalidDestination(&'static str),
    #[error("tuning publication identity failure: {0}")]
    Identity(&'static str),
    #[error("tuning publication I/O failure: {0}")]
    Io(String),
    #[error("injected publication crash at {0}")]
    InjectedCrash(&'static str),
}

impl From<std::io::Error> for PublicationError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}
