use crate::{CkProfileDirectoryAnchor, CkProfileIdentity, CkProfileKirPlan};

/// Complete generation-only input consumed by Native lowering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeProfileGeneration {
    pub(crate) plan: CkProfileKirPlan,
    pub(crate) identity: CkProfileIdentity,
    pub(crate) directory: CkProfileDirectoryAnchor,
}

impl NativeProfileGeneration {
    /// Binds a canonical generation plan and profile identity to one validated
    /// collection directory. Lowering independently revalidates every field.
    #[must_use]
    pub const fn new(
        plan: CkProfileKirPlan,
        identity: CkProfileIdentity,
        directory: CkProfileDirectoryAnchor,
    ) -> Self {
        Self {
            plan,
            identity,
            directory,
        }
    }

    /// Returns the full generation-only control entry name.
    ///
    /// # Errors
    ///
    /// Returns a profile validation failure when the identity is malformed.
    pub fn flush_symbol(&self) -> Result<String, crate::CkProfileError> {
        self.identity
            .digest_hex()
            .map(|digest| format!("ck_profile_flush_{digest}"))
    }
}
