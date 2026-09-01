use std::{marker::PhantomData, ptr::NonNull, rc::Rc};

use crate::{
    KirAlignmentClass, KirConsumer, KirCostSemantics, KirLaneType, KirLegalCost,
    KirNativeCpuPolicy, KirProfileOperation, KirTargetProfile, KirTargetProfileBuilder,
};

use super::{
    error::NativeError,
    ffi::{self, BridgeCpuPolicy, CkcLlvmTarget},
    object::{NativeObject, OptimizedNativeModule},
};

/// Host CPU feature policy for native object generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeCpu {
    /// Documented architecture baseline, independent of the build host model.
    Baseline,
    /// Complete CPU and feature set detected on the current host.
    Native,
}

/// Unique owner of the host LLVM target machine.
#[derive(Debug)]
pub struct NativeTarget {
    handle: NonNull<CkcLlvmTarget>,
    cpu_policy: NativeCpu,
    not_send_or_sync: PhantomData<Rc<()>>,
}

impl NativeTarget {
    /// Detects and creates the current host target machine.
    ///
    /// # Errors
    /// Returns a target-stage error when the host target is unavailable.
    pub fn host() -> Result<Self, NativeError> {
        Self::host_with_cpu(NativeCpu::Native)
    }

    /// Creates the host TargetMachine using an explicit CPU policy.
    pub fn host_with_cpu(cpu: NativeCpu) -> Result<Self, NativeError> {
        Ok(Self {
            handle: ffi::target_create_host(match cpu {
                NativeCpu::Baseline => BridgeCpuPolicy::Baseline,
                NativeCpu::Native => BridgeCpuPolicy::Native,
            })?,
            cpu_policy: cpu,
            not_send_or_sync: PhantomData,
        })
    }

    pub(super) const fn handle(&self) -> NonNull<CkcLlvmTarget> {
        self.handle
    }

    /// Returns the normalized target triple owned by the host TargetMachine.
    pub fn triple(&self) -> Result<String, NativeError> {
        ffi::target_triple(self.handle)
    }

    /// Returns the exact host TargetMachine data-layout string.
    pub fn data_layout(&self) -> Result<String, NativeError> {
        ffi::target_data_layout(self.handle)
    }

    /// Returns LLVM's CPU name for this TargetMachine.
    pub fn cpu(&self) -> Result<String, NativeError> {
        ffi::target_cpu(self.handle)
    }

    /// Returns LLVM's complete feature string for this TargetMachine.
    pub fn features(&self) -> Result<String, NativeError> {
        ffi::target_features(self.handle)
    }

    /// Constructs the canonical KIR target profile from this exact
    /// TargetMachine using the bridge's fixed TTI query universe.
    pub fn kir_profile(&self, consumer: KirConsumer) -> Result<KirTargetProfile, NativeError> {
        if !matches!(
            consumer,
            KirConsumer::NativeLibrary | KirConsumer::NativeExecutable
        ) {
            return Err(NativeError::new(
                super::error::NativeStage::Target,
                1,
                "native target profile requires a Native consumer".to_string(),
            ));
        }
        let triple = self.triple()?;
        let cpu = self.cpu()?;
        let features = self
            .features()?
            .split(',')
            .filter(|feature| !feature.is_empty())
            .map(str::to_string)
            .collect();
        let (pointer_width_bits, little_endian) = ffi::target_layout(self.handle)?;
        let pointer_width_bits = u16::try_from(pointer_width_bits).map_err(|_| {
            NativeError::new(
                super::error::NativeStage::Target,
                3,
                "LLVM target pointer width exceeds KIR profile schema".to_string(),
            )
        })?;
        let mut builder = KirTargetProfileBuilder::native(
            consumer,
            triple,
            pointer_width_bits,
            little_endian,
            match self.cpu_policy {
                NativeCpu::Baseline => KirNativeCpuPolicy::Baseline,
                NativeCpu::Native => KirNativeCpuPolicy::Native,
            },
            cpu,
            features,
        )
        .map_err(profile_error)?;
        let mut maximum_interleave_factor = 1u8;
        for key in KirTargetProfile::fixed_query_universe() {
            let result = ffi::target_profile_query(
                self.handle,
                ffi::CkcLlvmTargetProfileQuery {
                    operation: operation_tag(key.operation),
                    lane: lane_tag(key.lane),
                    lanes: u32::from(key.lanes),
                    semantics: semantics_tag(key.semantics),
                    alignment: match key.alignment {
                        KirAlignmentClass::NotApplicable => 0,
                        KirAlignmentClass::Bytes(value) => u32::from(value),
                    },
                },
            )?;
            maximum_interleave_factor = maximum_interleave_factor
                .max(u8::try_from(result.maximum_interleave_factor).unwrap_or(u8::MAX));
            if result.available {
                let legalization_parts = match u16::try_from(result.legalization_parts) {
                    Ok(parts) => parts,
                    Err(_) => {
                        builder.set_unavailable(key).map_err(profile_error)?;
                        continue;
                    }
                };
                builder
                    .set_legal(
                        key,
                        KirLegalCost {
                            cost: result.cost,
                            legalization_parts,
                            legalized_type: result.legalized_type,
                        },
                    )
                    .map_err(profile_error)?;
            } else {
                builder.set_unavailable(key).map_err(profile_error)?;
            }
        }
        builder.set_maximum_interleave_factor(maximum_interleave_factor);
        builder.set_producer_identity(
            "LLVM 22.1.8 TCK_RecipThroughput",
            format!("ckc-llvm-bridge-abi-{}", ffi::LLVM_BRIDGE_ABI_VERSION),
        );
        builder.build().map_err(profile_error)
    }

    /// Verifies and emits one module as a host object.
    ///
    /// # Errors
    /// Returns a typed module or object-emission error.
    pub fn emit_object(
        &self,
        mut module: OptimizedNativeModule<'_>,
    ) -> Result<NativeObject, NativeError> {
        ffi::target_emit_object(self.handle, module.module.handle()).map(NativeObject::from_handle)
    }

    /// Revalidates cached bytes as a host relocatable object before reuse.
    #[doc(hidden)]
    pub fn parse_cached_object(&self, bytes: &[u8]) -> Result<NativeObject, NativeError> {
        ffi::target_parse_object(self.handle, bytes).map(NativeObject::from_handle)
    }
}

fn profile_error(message: String) -> NativeError {
    NativeError::new(super::error::NativeStage::Target, 3, message)
}

const fn operation_tag(operation: KirProfileOperation) -> u32 {
    match operation {
        KirProfileOperation::Splat => 1,
        KirProfileOperation::Add => 2,
        KirProfileOperation::Subtract => 3,
        KirProfileOperation::Multiply => 4,
        KirProfileOperation::Divide => 5,
        KirProfileOperation::Remainder => 6,
        KirProfileOperation::Negate => 7,
        KirProfileOperation::MaskNot => 8,
        KirProfileOperation::BitAnd => 9,
        KirProfileOperation::BitOr => 10,
        KirProfileOperation::BitXor => 11,
        KirProfileOperation::ShiftLeft => 12,
        KirProfileOperation::ShiftRight => 13,
        KirProfileOperation::Compare => 14,
        KirProfileOperation::Select => 15,
        KirProfileOperation::Cast => 16,
        KirProfileOperation::Insert => 17,
        KirProfileOperation::Extract => 18,
        KirProfileOperation::Load => 19,
        KirProfileOperation::Store => 20,
        KirProfileOperation::ReduceAdd => 21,
        KirProfileOperation::ReduceMin => 22,
        KirProfileOperation::ReduceMax => 23,
        KirProfileOperation::Branch => 24,
        KirProfileOperation::RuntimePredicate => 25,
        KirProfileOperation::ReduceMultiply => 26,
    }
}

const fn lane_tag(lane: KirLaneType) -> u32 {
    match lane {
        KirLaneType::I32 => 1,
        KirLaneType::I64 => 2,
        KirLaneType::U32 => 3,
        KirLaneType::U64 => 4,
        KirLaneType::F64 => 5,
    }
}

const fn semantics_tag(semantics: KirCostSemantics) -> u32 {
    match semantics {
        KirCostSemantics::NotApplicable => 0,
        KirCostSemantics::Modular => 1,
        KirCostSemantics::Checked => 2,
        KirCostSemantics::StrictFloat => 3,
    }
}

impl Drop for NativeTarget {
    fn drop(&mut self) {
        // SAFETY: `NativeTarget` is the unique owner and calls dispose once.
        unsafe { ffi::target_dispose(self.handle) };
    }
}
