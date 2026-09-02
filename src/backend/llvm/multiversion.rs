use crate::{
    CkPgoOptimizerPlan, EmitLlvmOptions, FunctionId, KirConsumer, KirMultiversionBundle,
    KirMultiversionPlanningRequest, KirMultiversionPlatform, KirMultiversionTargetSet,
    KirMultiversionTargetTier, KirMultiversionTierId, KirOptimizationLevel,
    check_kir_multiversion_bundle, materialized_tier, project_pgo_plan_for_kir,
    run_kir_pass_pipeline,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

use super::{
    NativeContext, NativeError, NativeObject, NativeOptimizationLevel, NativeStage, NativeTarget,
    kir_lower::{
        lower_native_multiversion_baseline_module, lower_native_multiversion_variant_module,
    },
};

/// Materialized target machines and their canonical checked KIR target set.
/// The target machines remain separate so no cross-variant LTO can occur.
#[derive(Debug)]
pub struct NativeMultiversionTargetSet {
    target_set: KirMultiversionTargetSet,
    targets: Vec<(KirMultiversionTierId, NativeTarget)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeMultiversionObjectRole {
    Baseline,
    Variant {
        root: FunctionId,
        tier: KirMultiversionTierId,
    },
    DispatchRuntime,
}

/// One independently emitted and re-parsed object in a checked multiversion
/// bundle. Object order and names are canonical assembler input for stage 09.
#[derive(Debug)]
pub struct NativeMultiversionObject {
    name: String,
    role: NativeMultiversionObjectRole,
    digest: [u8; 32],
    object: NativeObject,
}

impl NativeMultiversionObject {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn role(&self) -> NativeMultiversionObjectRole {
        self.role
    }

    #[must_use]
    pub const fn digest(&self) -> &[u8; 32] {
        &self.digest
    }

    #[must_use]
    pub const fn object(&self) -> &NativeObject {
        &self.object
    }
}

#[derive(Debug)]
pub struct NativeMultiversionObjectBundle {
    target_set_digest: [u8; 32],
    dispatch_runtime_digest: [u8; 32],
    objects: Vec<NativeMultiversionObject>,
}

impl NativeMultiversionObjectBundle {
    #[must_use]
    pub const fn target_set_digest(&self) -> &[u8; 32] {
        &self.target_set_digest
    }

    #[must_use]
    pub const fn dispatch_runtime_digest(&self) -> &[u8; 32] {
        &self.dispatch_runtime_digest
    }

    #[must_use]
    pub fn objects(&self) -> &[NativeMultiversionObject] {
        &self.objects
    }

    /// Returns the canonical, path-free named-object manifest after checking
    /// order, roles, names, and every physical object digest.
    pub fn manifest_bytes(&self) -> Result<Vec<u8>, NativeError> {
        self.validate()?;
        let mut output = Vec::with_capacity(128 + self.objects.len() * 128);
        output.extend_from_slice(b"CKC-MV-OBJECTS\0");
        output.extend_from_slice(&1_u32.to_be_bytes());
        output.extend_from_slice(&self.target_set_digest);
        output.extend_from_slice(&self.dispatch_runtime_digest);
        output.extend_from_slice(&(self.objects.len() as u32).to_be_bytes());
        for object in &self.objects {
            output.extend_from_slice(&(object.name.len() as u32).to_be_bytes());
            output.extend_from_slice(object.name.as_bytes());
            match object.role {
                NativeMultiversionObjectRole::Baseline => output.push(1),
                NativeMultiversionObjectRole::Variant { root, tier } => {
                    output.push(2);
                    output.extend_from_slice(&root.index().to_be_bytes());
                    let tier = tier.stable_name().as_bytes();
                    output.extend_from_slice(&(tier.len() as u32).to_be_bytes());
                    output.extend_from_slice(tier);
                }
                NativeMultiversionObjectRole::DispatchRuntime => output.push(3),
            }
            output.extend_from_slice(&object.digest);
        }
        Ok(output)
    }

    /// Content identity of the complete ordered object bundle.
    pub fn bundle_digest(&self) -> Result<[u8; 32], NativeError> {
        Ok(Sha256::digest(self.manifest_bytes()?).into())
    }

    pub(in crate::backend) fn validate(&self) -> Result<(), NativeError> {
        if self.objects.len() < 2
            || self.objects.first().map(NativeMultiversionObject::role)
                != Some(NativeMultiversionObjectRole::Baseline)
            || self.objects.last().map(NativeMultiversionObject::role)
                != Some(NativeMultiversionObjectRole::DispatchRuntime)
        {
            return Err(object_error(
                "multiversion object bundle must be baseline, ordered variants, then dispatch runtime",
            ));
        }
        let namespace = object_namespace(&self.target_set_digest);
        let mut names = BTreeSet::new();
        let mut prior_variant = None;
        for (index, object) in self.objects.iter().enumerate() {
            if object.name.is_empty()
                || object.name.len() > 255
                || object.name.starts_with('.')
                || !object
                    .name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
                || !object.name.ends_with(".o")
                || !object.name.contains(&namespace)
                || !names.insert(object.name.as_str())
            {
                return Err(object_error(
                    "multiversion object name is unsafe, duplicated, or not target-set namespaced",
                ));
            }
            if object.object.is_empty()
                || <[u8; 32]>::from(Sha256::digest(object.object.as_bytes())) != object.digest
            {
                return Err(object_error("multiversion physical object digest mismatch"));
            }
            match object.role {
                NativeMultiversionObjectRole::Baseline if index == 0 => {}
                NativeMultiversionObjectRole::DispatchRuntime
                    if index + 1 == self.objects.len() =>
                {
                    if object.digest != self.dispatch_runtime_digest {
                        return Err(object_error("dispatch runtime identity mismatch"));
                    }
                }
                NativeMultiversionObjectRole::Variant { root, tier }
                    if index > 0 && index + 1 < self.objects.len() =>
                {
                    let key = (root.index(), tier.stable_name());
                    if prior_variant.is_some_and(|prior| prior >= key) {
                        return Err(object_error(
                            "multiversion variant object order is not canonical",
                        ));
                    }
                    prior_variant = Some(key);
                }
                _ => {
                    return Err(object_error(
                        "multiversion object role does not match its canonical position",
                    ));
                }
            }
        }
        Ok(())
    }

    #[doc(hidden)]
    pub fn expected_layout(
        bundle: &KirMultiversionBundle,
    ) -> Vec<(String, NativeMultiversionObjectRole)> {
        let namespace = object_namespace(&bundle.target_set.digest);
        let mut layout = vec![(
            format!("baseline-{namespace}.o"),
            NativeMultiversionObjectRole::Baseline,
        )];
        for root in &bundle.roots {
            for variant in &root.variants {
                layout.push((
                    format!(
                        "variant-f{}-{}-{namespace}.o",
                        variant.root.index(),
                        variant.tier.stable_name().replace('-', "_")
                    ),
                    NativeMultiversionObjectRole::Variant {
                        root: variant.root,
                        tier: variant.tier,
                    },
                ));
            }
        }
        layout.push((
            format!("dispatch-runtime-{namespace}.o"),
            NativeMultiversionObjectRole::DispatchRuntime,
        ));
        layout
    }

    /// Re-parses and validates every object recovered from the private cache.
    /// The closed constructor prevents cached bytes from bypassing target or
    /// bundle validation.
    #[doc(hidden)]
    pub fn from_cached_objects(
        target: &NativeTarget,
        target_set_digest: [u8; 32],
        dispatch_runtime_digest: [u8; 32],
        cached: Vec<(String, NativeMultiversionObjectRole, Vec<u8>)>,
    ) -> Result<Self, NativeError> {
        let mut objects = Vec::with_capacity(cached.len());
        for (name, role, bytes) in cached {
            let object = target.parse_cached_object(&bytes)?;
            objects.push(named_object(name, role, object));
        }
        let bundle = Self {
            target_set_digest,
            dispatch_runtime_digest,
            objects,
        };
        bundle.validate()?;
        Ok(bundle)
    }
}

/// Emits baseline, every accepted enhanced module, and private detector support
/// as separate validated objects. It performs no linker or archive assembly.
pub fn emit_native_multiversion_objects(
    context: &NativeContext,
    targets: &NativeMultiversionTargetSet,
    request: &KirMultiversionPlanningRequest,
    bundle: &KirMultiversionBundle,
    pgo: Option<&CkPgoOptimizerPlan>,
    options: &EmitLlvmOptions,
) -> Result<NativeMultiversionObjectBundle, NativeError> {
    check_kir_multiversion_bundle(request, bundle).map_err(error)?;
    if targets.target_set() != &bundle.target_set {
        return Err(error(
            "materialized target set does not match the checked multiversion bundle",
        ));
    }
    let baseline_target = targets
        .target(KirMultiversionTierId::Baseline)
        .ok_or_else(|| error("multiversion baseline TargetMachine is missing"))?;
    let mut baseline_result =
        run_kir_pass_pipeline(bundle.baseline.clone(), KirOptimizationLevel::O0, None);
    if !baseline_result.errors.is_empty() {
        return Err(error(format!(
            "multiversion baseline revalidation failed: {}",
            baseline_result.errors.join("; ")
        )));
    }
    baseline_result.pgo = pgo
        .map(|plan| project_pgo_plan_for_kir(&bundle.baseline, plan).map_err(error))
        .transpose()?;
    let baseline = baseline_target.emit_object(
        lower_native_multiversion_baseline_module(
            context,
            baseline_target,
            &baseline_result,
            bundle,
            options,
        )?
        .verify()?
        .audit()?
        .optimize(baseline_target, NativeOptimizationLevel::O3)?,
    )?;
    let expected_layout = NativeMultiversionObjectBundle::expected_layout(bundle);
    let mut objects = vec![named_object(
        expected_layout[0].0.clone(),
        expected_layout[0].1,
        baseline,
    )];

    for root in &bundle.roots {
        for variant in &root.variants {
            let target = targets
                .target(variant.tier)
                .ok_or_else(|| error("multiversion variant TargetMachine is missing"))?;
            let mut result =
                run_kir_pass_pipeline(variant.module.clone(), KirOptimizationLevel::O0, None);
            if !result.errors.is_empty() {
                return Err(error(format!(
                    "multiversion variant revalidation failed: {}",
                    result.errors.join("; ")
                )));
            }
            result.pgo = pgo
                .map(|plan| project_pgo_plan_for_kir(&variant.module, plan).map_err(error))
                .transpose()?;
            let object = target.emit_object(
                lower_native_multiversion_variant_module(
                    context, target, &result, variant, options,
                )?
                .verify()?
                .audit()?
                .optimize(target, NativeOptimizationLevel::O3)?,
            )?;
            let layout = &expected_layout[objects.len()];
            objects.push(named_object(layout.0.clone(), layout.1, object));
        }
    }

    let runtime = baseline_target
        .parse_cached_object(crate::backend::native_runtime::embedded_dispatch_runtime_object())?;
    let dispatch_runtime_digest = Sha256::digest(runtime.as_bytes()).into();
    objects.push(named_object(
        expected_layout.last().expect("dispatch layout").0.clone(),
        NativeMultiversionObjectRole::DispatchRuntime,
        runtime,
    ));
    Ok(NativeMultiversionObjectBundle {
        target_set_digest: bundle.target_set.digest,
        dispatch_runtime_digest,
        objects,
    })
}

fn named_object(
    name: String,
    role: NativeMultiversionObjectRole,
    object: NativeObject,
) -> NativeMultiversionObject {
    let digest = Sha256::digest(object.as_bytes()).into();
    NativeMultiversionObject {
        name,
        role,
        digest,
        object,
    }
}

fn object_namespace(digest: &[u8; 32]) -> String {
    let mut output = String::with_capacity(16);
    for byte in &digest[..8] {
        use std::fmt::Write;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

impl NativeMultiversionTargetSet {
    pub fn host(consumer: KirConsumer) -> Result<Self, NativeError> {
        if !matches!(
            consumer,
            KirConsumer::NativeLibrary | KirConsumer::NativeExecutable
        ) {
            return Err(error("multiversion target set requires a Native consumer"));
        }
        let baseline = NativeTarget::host_with_cpu(super::NativeCpu::Multiversion)?;
        let triple = baseline.triple()?;
        let platform = KirMultiversionPlatform::from_triple(&triple).map_err(error)?;
        let fixture =
            KirMultiversionTargetSet::schema1_for_triple(&triple, consumer).map_err(error)?;
        drop(baseline);

        let mut targets = Vec::with_capacity(fixture.tiers.len());
        let mut tiers = Vec::with_capacity(fixture.tiers.len());
        for descriptor in fixture.tiers {
            let target = NativeTarget::explicit_multiversion(
                &triple,
                &descriptor.cpu,
                &descriptor.llvm_features,
            )?;
            if target.triple()? != triple {
                return Err(error(
                    "explicit multiversion TargetMachine changed the host triple",
                ));
            }
            let data_layout = target.data_layout()?;
            let profile = target.kir_profile(consumer)?;
            let (llvm_identity, bridge_identity) = profile.producer_identity();
            let llvm_identity = llvm_identity.unwrap_or_default().to_string();
            let bridge_identity = bridge_identity.unwrap_or_default().to_string();
            let tier = materialized_tier(
                platform,
                descriptor.id,
                triple.clone(),
                data_layout,
                profile,
                llvm_identity,
                bridge_identity,
            )
            .map_err(error)?;
            targets.push((descriptor.id, target));
            tiers.push(tier);
        }
        let target_set = KirMultiversionTargetSet::from_materialized(platform, consumer, tiers)
            .map_err(error)?;
        Ok(Self {
            target_set,
            targets,
        })
    }

    #[must_use]
    pub const fn target_set(&self) -> &KirMultiversionTargetSet {
        &self.target_set
    }

    #[must_use]
    pub fn tier(&self, id: KirMultiversionTierId) -> Option<&KirMultiversionTargetTier> {
        self.target_set.tier(id)
    }

    #[must_use]
    pub fn target(&self, id: KirMultiversionTierId) -> Option<&NativeTarget> {
        self.targets
            .iter()
            .find_map(|(candidate, target)| (*candidate == id).then_some(target))
    }
}

fn error(message: impl Into<String>) -> NativeError {
    NativeError::new(NativeStage::Target, 1, message.into())
}

fn object_error(message: impl Into<String>) -> NativeError {
    NativeError::new(NativeStage::Object, 1, message.into())
}
