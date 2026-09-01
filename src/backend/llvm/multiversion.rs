use crate::{
    EmitLlvmOptions, FunctionId, KirConsumer, KirMultiversionBundle,
    KirMultiversionPlanningRequest, KirMultiversionPlatform, KirMultiversionTargetSet,
    KirMultiversionTargetTier, KirMultiversionTierId, KirOptimizationLevel,
    check_kir_multiversion_bundle, materialized_tier, run_kir_pass_pipeline,
};
use sha2::{Digest, Sha256};

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
}

/// Emits baseline, every accepted enhanced module, and private detector support
/// as separate validated objects. It performs no linker or archive assembly.
pub fn emit_native_multiversion_objects(
    context: &NativeContext,
    targets: &NativeMultiversionTargetSet,
    request: &KirMultiversionPlanningRequest,
    bundle: &KirMultiversionBundle,
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
    let baseline_result =
        run_kir_pass_pipeline(bundle.baseline.clone(), KirOptimizationLevel::O0, None);
    if !baseline_result.errors.is_empty() {
        return Err(error(format!(
            "multiversion baseline revalidation failed: {}",
            baseline_result.errors.join("; ")
        )));
    }
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
    let namespace = object_namespace(&bundle.target_set.digest);
    let mut objects = vec![named_object(
        format!("baseline-{namespace}.o"),
        NativeMultiversionObjectRole::Baseline,
        baseline,
    )];

    for root in &bundle.roots {
        for variant in &root.variants {
            let target = targets
                .target(variant.tier)
                .ok_or_else(|| error("multiversion variant TargetMachine is missing"))?;
            let result =
                run_kir_pass_pipeline(variant.module.clone(), KirOptimizationLevel::O0, None);
            if !result.errors.is_empty() {
                return Err(error(format!(
                    "multiversion variant revalidation failed: {}",
                    result.errors.join("; ")
                )));
            }
            let object = target.emit_object(
                lower_native_multiversion_variant_module(
                    context, target, &result, variant, options,
                )?
                .verify()?
                .audit()?
                .optimize(target, NativeOptimizationLevel::O3)?,
            )?;
            objects.push(named_object(
                format!(
                    "variant-f{}-{}-{namespace}.o",
                    variant.root.index(),
                    variant.tier.stable_name().replace('-', "_")
                ),
                NativeMultiversionObjectRole::Variant {
                    root: variant.root,
                    tier: variant.tier,
                },
                object,
            ));
        }
    }

    let runtime = baseline_target
        .parse_cached_object(crate::backend::native_runtime::embedded_dispatch_runtime_object())?;
    let dispatch_runtime_digest = Sha256::digest(runtime.as_bytes()).into();
    objects.push(named_object(
        format!("dispatch-runtime-{namespace}.o"),
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
