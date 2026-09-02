use crate::{
    KirVerifiedProgramState, TuningPlan, TuningSpace, apply_tuning_plan, check_tuning_plan,
};
use std::{fs::OpenOptions, io::Write, path::Path};

use super::artifact::{
    ArtifactIdentity, TuneArtifactBytes, TuneArtifactKind, derive_artifact_identity,
};

/// Byte package produced by the shared verified Native build pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuneTrialBuildRequest {
    kind: TuneArtifactKind,
    bytes: TuneArtifactBytes,
    object_graph: Vec<(String, Vec<u8>)>,
    link_recipe: Vec<String>,
}

impl TuneTrialBuildRequest {
    #[must_use]
    pub fn new(
        kind: TuneArtifactKind,
        primary: Vec<u8>,
        header: Option<Vec<u8>>,
        import_library: Option<Vec<u8>>,
        object_graph: Vec<(String, Vec<u8>)>,
        link_recipe: Vec<String>,
    ) -> Self {
        Self {
            kind,
            bytes: TuneArtifactBytes {
                primary,
                header,
                import_library,
            },
            object_graph,
            link_recipe,
        }
    }

    /// Imports bytes only from the shared verified Native packaging pipeline.
    #[cfg(feature = "native-toolchain")]
    pub fn from_verified_native_build(build: &crate::VerifiedNativeBuild) -> Result<Self, String> {
        let kind = match build.kind() {
            crate::NativeArtifactKind::Executable => TuneArtifactKind::Executable,
            crate::NativeArtifactKind::Dynamic => TuneArtifactKind::Dynamic,
            crate::NativeArtifactKind::Static | crate::NativeArtifactKind::Object => {
                return Err(
                    "offline tuning accepts only executable or dynamic artifacts".to_string(),
                );
            }
        };
        Ok(Self::new(
            kind,
            build.primary().to_vec(),
            build.header().map(<[u8]>::to_vec),
            build.import_library().map(<[u8]>::to_vec),
            build.object_graph().to_vec(),
            build.link_recipe().to_vec(),
        ))
    }
}

/// A verified tuning artifact that deliberately has no publication conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonPublishableTuneTrial {
    plan: TuningPlan,
    post_state_digest: [u8; 32],
    identity: ArtifactIdentity,
    bytes: TuneArtifactBytes,
    object_graph: Vec<(String, Vec<u8>)>,
    link_recipe: Vec<String>,
}

impl NonPublishableTuneTrial {
    #[must_use]
    pub const fn identity(&self) -> &ArtifactIdentity {
        &self.identity
    }

    #[must_use]
    pub fn primary_size(&self) -> u64 {
        self.identity.roles[0].size
    }

    #[must_use]
    pub const fn plan_digest(&self) -> [u8; 32] {
        self.plan.digest
    }

    pub(crate) fn verify_internal_identity(&self) -> Result<(), String> {
        let identity = derive_artifact_identity(
            self.identity.kind,
            &self.bytes,
            &self.object_graph,
            &self.link_recipe,
        )?;
        if identity != self.identity {
            return Err("tuning trial artifact identity mismatch".to_string());
        }
        Ok(())
    }

    pub(crate) const fn plan(&self) -> &TuningPlan {
        &self.plan
    }

    pub(crate) const fn post_state_digest(&self) -> [u8; 32] {
        self.post_state_digest
    }

    pub(crate) const fn post_state_digest_bytes(&self) -> [u8; 32] {
        self.post_state_digest
    }

    pub(crate) const fn artifact_kind_name(&self) -> &'static str {
        match self.identity.kind {
            TuneArtifactKind::Executable => "executable",
            TuneArtifactKind::Dynamic => "dynamic",
        }
    }

    pub(crate) fn stage_primary_for_measurement(&self, path: &Path) -> Result<(), String> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|error| error.to_string())?;
        file.write_all(&self.bytes.primary)
            .map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        #[cfg(unix)]
        if self.identity.kind == TuneArtifactKind::Executable {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}

/// Freezes one checked plan and actual bytes returned by the shared Native pipeline.
pub fn compile_tune_trial(
    state: &KirVerifiedProgramState,
    space: &TuningSpace,
    plan: &TuningPlan,
    request: TuneTrialBuildRequest,
) -> Result<NonPublishableTuneTrial, String> {
    check_tuning_plan(state, space, plan).map_err(|error| error.to_string())?;
    let replayed = apply_tuning_plan(state, space, plan).map_err(|error| error.to_string())?;
    let identity = derive_artifact_identity(
        request.kind,
        &request.bytes,
        &request.object_graph,
        &request.link_recipe,
    )?;
    Ok(NonPublishableTuneTrial {
        plan: plan.clone(),
        post_state_digest: crate::tuning_kir_state_digest(&replayed)
            .map_err(|error| error.to_string())?,
        identity,
        bytes: request.bytes,
        object_graph: request.object_graph,
        link_recipe: request.link_recipe,
    })
}
