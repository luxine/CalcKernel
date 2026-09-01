use crate::{
    KirConsumer, KirMultiversionPlatform, KirMultiversionTargetSet, KirMultiversionTargetTier,
    KirMultiversionTierId, materialized_tier,
};

use super::{NativeError, NativeStage, NativeTarget};

/// Materialized target machines and their canonical checked KIR target set.
/// The target machines remain separate so no cross-variant LTO can occur.
#[derive(Debug)]
pub struct NativeMultiversionTargetSet {
    target_set: KirMultiversionTargetSet,
    targets: Vec<(KirMultiversionTierId, NativeTarget)>,
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
