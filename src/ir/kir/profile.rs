use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, OnceLock},
};

use sha2::{Digest, Sha256};

use super::KirConsumer;

pub const KIR_TARGET_PROFILE_SCHEMA: u16 = 1;
/// Canonical sentinel lane for mask-only operation costs. KIR masks retain
/// their lane count but deliberately do not carry a scalar lane type.
pub const KIR_MASK_COST_LANE: KirLaneType = KirLaneType::I32;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum KirTargetIdentity {
    Inspection,
    PortableC,
    WebAssembly,
    WebAssemblySimd128,
    Native { triple: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KirWasmFeatures {
    Baseline,
    Simd128,
}

impl KirWasmFeatures {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Simd128 => "simd128",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KirProfileLayout {
    PortableUnknown,
    Known {
        pointer_width_bits: u16,
        little_endian: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum KirNativeCpuPolicy {
    Baseline,
    Native,
    Multiversion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KirCpuIdentity {
    NotApplicable,
    Native {
        policy: KirNativeCpuPolicy,
        name: String,
        features: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KirLaneType {
    I32,
    I64,
    U32,
    U64,
    F64,
}

impl KirLaneType {
    const ALL: [Self; 5] = [Self::I32, Self::I64, Self::U32, Self::U64, Self::F64];

    pub const fn bit_width(self) -> u16 {
        match self {
            Self::I32 | Self::U32 => 32,
            Self::I64 | Self::U64 | Self::F64 => 64,
        }
    }

    const fn tag(self) -> u8 {
        match self {
            Self::I32 => 1,
            Self::I64 => 2,
            Self::U32 => 3,
            Self::U64 => 4,
            Self::F64 => 5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum KirProfileOperation {
    Splat,
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    Negate,
    MaskNot,
    BitAnd,
    BitOr,
    BitXor,
    ShiftLeft,
    ShiftRight,
    Compare,
    Select,
    Cast,
    Insert,
    Extract,
    Load,
    Store,
    ReduceAdd,
    ReduceMin,
    ReduceMax,
    Branch,
    RuntimePredicate,
    ReduceMultiply,
}

impl KirProfileOperation {
    const ALL: [Self; 26] = [
        Self::Splat,
        Self::Add,
        Self::Subtract,
        Self::Multiply,
        Self::Divide,
        Self::Remainder,
        Self::Negate,
        Self::MaskNot,
        Self::BitAnd,
        Self::BitOr,
        Self::BitXor,
        Self::ShiftLeft,
        Self::ShiftRight,
        Self::Compare,
        Self::Select,
        Self::Cast,
        Self::Insert,
        Self::Extract,
        Self::Load,
        Self::Store,
        Self::ReduceAdd,
        Self::ReduceMin,
        Self::ReduceMax,
        Self::Branch,
        Self::RuntimePredicate,
        Self::ReduceMultiply,
    ];

    const fn tag(self) -> u8 {
        self as u8 + 1
    }

    const fn uses_alignment(self) -> bool {
        matches!(self, Self::Load | Self::Store)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum KirCostSemantics {
    NotApplicable,
    Modular,
    Checked,
    StrictFloat,
}

impl KirCostSemantics {
    const fn tag(self) -> u8 {
        match self {
            Self::NotApplicable => 0,
            Self::Modular => 1,
            Self::Checked => 2,
            Self::StrictFloat => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum KirAlignmentClass {
    NotApplicable,
    Bytes(u16),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct KirCostKey {
    pub operation: KirProfileOperation,
    pub lane: KirLaneType,
    pub lanes: u8,
    pub semantics: KirCostSemantics,
    pub alignment: KirAlignmentClass,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirLegalCost {
    pub cost: u32,
    pub legalization_parts: u16,
    pub legalized_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KirOperationAvailability {
    Legal(KirLegalCost),
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirTargetProfile(Arc<KirTargetProfileData>);

#[derive(Debug)]
struct KirTargetProfileData {
    schema_version: u16,
    consumer: KirConsumer,
    target_identity: KirTargetIdentity,
    layout: KirProfileLayout,
    cpu_identity: KirCpuIdentity,
    legal_vector_widths: BTreeSet<u16>,
    legal_lane_types: BTreeSet<KirLaneType>,
    maximum_interleave_factor: u8,
    costs: BTreeMap<KirCostKey, KirOperationAvailability>,
    llvm_identity: Option<String>,
    bridge_identity: Option<String>,
    digest: [u8; 32],
    validation: OnceLock<Result<(), String>>,
}

impl PartialEq for KirTargetProfileData {
    fn eq(&self, other: &Self) -> bool {
        self.schema_version == other.schema_version
            && self.consumer == other.consumer
            && self.target_identity == other.target_identity
            && self.layout == other.layout
            && self.cpu_identity == other.cpu_identity
            && self.legal_vector_widths == other.legal_vector_widths
            && self.legal_lane_types == other.legal_lane_types
            && self.maximum_interleave_factor == other.maximum_interleave_factor
            && self.costs == other.costs
            && self.llvm_identity == other.llvm_identity
            && self.bridge_identity == other.bridge_identity
            && self.digest == other.digest
    }
}

impl Eq for KirTargetProfileData {}

impl Clone for KirTargetProfileData {
    fn clone(&self) -> Self {
        Self {
            schema_version: self.schema_version,
            consumer: self.consumer,
            target_identity: self.target_identity.clone(),
            layout: self.layout,
            cpu_identity: self.cpu_identity.clone(),
            legal_vector_widths: self.legal_vector_widths.clone(),
            legal_lane_types: self.legal_lane_types.clone(),
            maximum_interleave_factor: self.maximum_interleave_factor,
            costs: self.costs.clone(),
            llvm_identity: self.llvm_identity.clone(),
            bridge_identity: self.bridge_identity.clone(),
            digest: self.digest,
            // `Arc::make_mut` is the only in-module escape hatch used by
            // adversarial tests. A copied backing value is therefore a new,
            // untrusted value and must not inherit the source validation.
            validation: OnceLock::new(),
        }
    }
}

impl KirTargetProfile {
    #[must_use]
    pub fn inspection() -> Self {
        static PROFILE: OnceLock<KirTargetProfile> = OnceLock::new();
        PROFILE
            .get_or_init(|| {
                Self::portable(
                    KirConsumer::Inspection,
                    KirTargetIdentity::Inspection,
                    KirProfileLayout::PortableUnknown,
                )
            })
            .clone()
    }

    #[must_use]
    pub fn portable_c() -> Self {
        static PROFILE: OnceLock<KirTargetProfile> = OnceLock::new();
        PROFILE
            .get_or_init(|| {
                Self::portable(
                    KirConsumer::C,
                    KirTargetIdentity::PortableC,
                    KirProfileLayout::PortableUnknown,
                )
            })
            .clone()
    }

    #[must_use]
    pub fn webassembly() -> Self {
        static PROFILE: OnceLock<KirTargetProfile> = OnceLock::new();
        PROFILE
            .get_or_init(|| {
                Self::portable(
                    KirConsumer::WebAssembly,
                    KirTargetIdentity::WebAssembly,
                    KirProfileLayout::Known {
                        pointer_width_bits: 32,
                        little_endian: true,
                    },
                )
            })
            .clone()
    }

    #[must_use]
    pub fn webassembly_with_features(features: KirWasmFeatures) -> Self {
        match features {
            KirWasmFeatures::Baseline => Self::webassembly(),
            KirWasmFeatures::Simd128 => Self::webassembly_simd128(),
        }
    }

    fn webassembly_simd128() -> Self {
        static PROFILE: OnceLock<KirTargetProfile> = OnceLock::new();
        PROFILE
            .get_or_init(|| {
                let costs = portable_cost_entries()
                    .into_iter()
                    .map(|(key, availability)| {
                        if key.lanes == 1 {
                            return (key, availability);
                        }
                        let availability = wasm_simd128_operation_cost(&key).map_or(
                            KirOperationAvailability::Unavailable,
                            |cost| {
                                KirOperationAvailability::Legal(KirLegalCost {
                                    cost,
                                    legalization_parts: 1,
                                    legalized_type: wasm_simd128_type(key.lane, key.lanes),
                                })
                            },
                        );
                        (key, availability)
                    })
                    .collect();
                Self::new(
                    KirConsumer::WebAssembly,
                    KirTargetIdentity::WebAssemblySimd128,
                    KirProfileLayout::Known {
                        pointer_width_bits: 32,
                        little_endian: true,
                    },
                    KirCpuIdentity::NotApplicable,
                    BTreeSet::from([64, 128]),
                    BTreeSet::from([KirLaneType::F64, KirLaneType::I32, KirLaneType::U32]),
                    1,
                    costs,
                    None,
                    None,
                )
                .expect("built-in Wasm SIMD128 KIR target profile must be valid")
            })
            .clone()
    }

    #[must_use]
    pub fn wasm_features(&self) -> Option<KirWasmFeatures> {
        match &self.0.target_identity {
            KirTargetIdentity::WebAssembly => Some(KirWasmFeatures::Baseline),
            KirTargetIdentity::WebAssemblySimd128 => Some(KirWasmFeatures::Simd128),
            _ => None,
        }
    }

    #[must_use]
    pub fn for_consumer(consumer: KirConsumer) -> Self {
        match consumer {
            KirConsumer::Inspection => Self::inspection(),
            KirConsumer::C => Self::portable_c(),
            KirConsumer::WebAssembly => Self::webassembly(),
            KirConsumer::NativeLibrary | KirConsumer::NativeExecutable => {
                Self::conservative_native(consumer)
            }
        }
    }

    #[must_use]
    pub fn schema_version(&self) -> u16 {
        self.0.schema_version
    }

    #[must_use]
    pub fn consumer(&self) -> KirConsumer {
        self.0.consumer
    }

    #[must_use]
    pub fn target_identity(&self) -> &KirTargetIdentity {
        &self.0.target_identity
    }

    #[must_use]
    pub fn layout(&self) -> KirProfileLayout {
        self.0.layout
    }

    #[must_use]
    pub fn cpu_identity(&self) -> &KirCpuIdentity {
        &self.0.cpu_identity
    }

    #[must_use]
    pub fn maximum_interleave_factor(&self) -> u8 {
        self.0.maximum_interleave_factor
    }

    #[must_use]
    pub fn producer_identity(&self) -> (Option<&str>, Option<&str>) {
        (
            self.0.llvm_identity.as_deref(),
            self.0.bridge_identity.as_deref(),
        )
    }

    #[must_use]
    pub fn vector_operations_enabled(&self) -> bool {
        !self.0.legal_vector_widths.is_empty()
            && !self.0.legal_lane_types.is_empty()
            && self.0.costs.iter().any(|(key, availability)| {
                key.lanes > 1 && matches!(availability, KirOperationAvailability::Legal(_))
            })
    }

    #[must_use]
    pub fn cost_entry_count(&self) -> usize {
        self.0.costs.len()
    }

    #[must_use]
    pub fn fixed_query_universe() -> Vec<KirCostKey> {
        cost_universe().iter().cloned().collect()
    }

    #[must_use]
    pub fn operation_availability(&self, key: &KirCostKey) -> Option<&KirOperationAvailability> {
        self.0.costs.get(key)
    }

    #[must_use]
    pub fn supports_vector_shape(&self, lane: KirLaneType, lanes: u16) -> bool {
        lanes > 1
            && self.0.legal_lane_types.contains(&lane)
            && self
                .0
                .legal_vector_widths
                .contains(&(lane.bit_width() * lanes))
    }

    #[must_use]
    pub fn supports_mask_lanes(&self, lanes: u16) -> bool {
        self.0.costs.iter().any(|(key, availability)| {
            u16::from(key.lanes) == lanes
                && key.lanes > 1
                && matches!(availability, KirOperationAvailability::Legal(_))
        })
    }

    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        self.encode_without_digest()
    }

    #[must_use]
    pub fn digest_hex(&self) -> String {
        hex_digest(&self.0.digest)
    }

    pub fn validate(&self) -> Result<(), String> {
        self.0
            .validation
            .get_or_init(|| self.validate_uncached())
            .clone()
    }

    fn validate_uncached(&self) -> Result<(), String> {
        if self.0.schema_version != KIR_TARGET_PROFILE_SCHEMA {
            return Err("unsupported KIR target profile schema".to_string());
        }
        self.validate_identity()?;
        let expected = cost_universe();
        if self.0.costs.len() != expected.len()
            || expected.iter().any(|key| !self.0.costs.contains_key(key))
        {
            return Err("KIR target profile cost universe is incomplete".to_string());
        }
        for (key, availability) in &self.0.costs {
            if let KirOperationAvailability::Legal(cost) = availability {
                if key.operation == KirProfileOperation::MaskNot && key.lane != KIR_MASK_COST_LANE {
                    return Err(
                        "KIR target profile uses a non-canonical mask cost lane".to_string()
                    );
                }
                if cost.cost == 0 {
                    return Err(
                        "KIR target profile contains a zero cost for emitted work".to_string()
                    );
                }
                if cost.legalization_parts == 0 || cost.legalized_type.is_empty() {
                    return Err("KIR target profile contains invalid legalization data".to_string());
                }
                if key.lanes > 1
                    && (!self
                        .0
                        .legal_vector_widths
                        .contains(&(key.lane.bit_width() * u16::from(key.lanes)))
                        || !self.0.legal_lane_types.contains(&key.lane))
                {
                    return Err("KIR target profile contradicts vector legality".to_string());
                }
                if self.0.layout == KirProfileLayout::PortableUnknown
                    && key.operation.uses_alignment()
                    && key.lanes > 1
                {
                    return Err(
                        "unknown KIR target layout enables a layout-sensitive operation"
                            .to_string(),
                    );
                }
            }
        }
        if self.0.maximum_interleave_factor == 0 {
            return Err("KIR target profile has an invalid interleave factor".to_string());
        }
        let expected_digest = Sha256::digest(self.encode_without_digest());
        if expected_digest.as_slice() != self.0.digest {
            return Err("KIR target profile digest is stale".to_string());
        }
        Ok(())
    }

    fn portable(
        consumer: KirConsumer,
        target_identity: KirTargetIdentity,
        layout: KirProfileLayout,
    ) -> Self {
        let costs = portable_cost_entries();
        Self::new(
            consumer,
            target_identity,
            layout,
            KirCpuIdentity::NotApplicable,
            BTreeSet::new(),
            BTreeSet::new(),
            1,
            costs,
            None,
            None,
        )
        .expect("built-in portable KIR target profile must be valid")
    }

    fn conservative_native(consumer: KirConsumer) -> Self {
        static LIBRARY: OnceLock<KirTargetProfile> = OnceLock::new();
        static EXECUTABLE: OnceLock<KirTargetProfile> = OnceLock::new();
        let slot = match consumer {
            KirConsumer::NativeLibrary => &LIBRARY,
            KirConsumer::NativeExecutable => &EXECUTABLE,
            _ => unreachable!("native profile requires a Native consumer"),
        };
        slot.get_or_init(|| Self::build_conservative_native(consumer))
            .clone()
    }

    fn build_conservative_native(consumer: KirConsumer) -> Self {
        let target = KirTargetIdentity::Native {
            triple: env!("CKC_BUILD_TARGET").to_string(),
        };
        let layout = KirProfileLayout::Known {
            pointer_width_bits: usize::BITS as u16,
            little_endian: cfg!(target_endian = "little"),
        };
        Self::new(
            consumer,
            target,
            layout,
            KirCpuIdentity::Native {
                policy: KirNativeCpuPolicy::Baseline,
                name: "baseline-unqueried".to_string(),
                features: Vec::new(),
            },
            BTreeSet::new(),
            BTreeSet::new(),
            1,
            portable_cost_entries(),
            None,
            None,
        )
        .expect("conservative native KIR target profile must be valid")
    }

    #[allow(clippy::too_many_arguments)]
    fn new(
        consumer: KirConsumer,
        target_identity: KirTargetIdentity,
        layout: KirProfileLayout,
        mut cpu_identity: KirCpuIdentity,
        legal_vector_widths: BTreeSet<u16>,
        legal_lane_types: BTreeSet<KirLaneType>,
        maximum_interleave_factor: u8,
        entries: Vec<(KirCostKey, KirOperationAvailability)>,
        llvm_identity: Option<String>,
        bridge_identity: Option<String>,
    ) -> Result<Self, String> {
        if let KirCpuIdentity::Native { features, .. } = &mut cpu_identity {
            features.sort();
            features.dedup();
        }
        let mut costs = BTreeMap::new();
        for (key, availability) in entries {
            if costs.insert(key, availability).is_some() {
                return Err("KIR target profile contains a duplicate cost key".to_string());
            }
        }
        let mut data = KirTargetProfileData {
            schema_version: KIR_TARGET_PROFILE_SCHEMA,
            consumer,
            target_identity,
            layout,
            cpu_identity,
            legal_vector_widths,
            legal_lane_types,
            maximum_interleave_factor,
            costs,
            llvm_identity,
            bridge_identity,
            digest: [0; 32],
            validation: OnceLock::new(),
        };
        let digest = Sha256::digest(encode_profile_data(&data));
        data.digest.copy_from_slice(&digest);
        let profile = Self(Arc::new(data));
        profile.validate()?;
        Ok(profile)
    }

    fn validate_identity(&self) -> Result<(), String> {
        let identity_matches = matches!(
            (self.0.consumer, &self.0.target_identity),
            (KirConsumer::Inspection, KirTargetIdentity::Inspection)
                | (KirConsumer::C, KirTargetIdentity::PortableC)
                | (KirConsumer::WebAssembly, KirTargetIdentity::WebAssembly)
                | (
                    KirConsumer::WebAssembly,
                    KirTargetIdentity::WebAssemblySimd128
                )
                | (
                    KirConsumer::NativeLibrary | KirConsumer::NativeExecutable,
                    KirTargetIdentity::Native { .. }
                )
        );
        if !identity_matches {
            return Err("KIR target profile consumer and target identity disagree".to_string());
        }
        match (&self.0.target_identity, self.0.layout, &self.0.cpu_identity) {
            (
                KirTargetIdentity::Inspection | KirTargetIdentity::PortableC,
                KirProfileLayout::PortableUnknown,
                KirCpuIdentity::NotApplicable,
            )
            | (
                KirTargetIdentity::WebAssembly | KirTargetIdentity::WebAssemblySimd128,
                KirProfileLayout::Known {
                    pointer_width_bits: 32,
                    little_endian: true,
                },
                KirCpuIdentity::NotApplicable,
            ) => Ok(()),
            (
                KirTargetIdentity::Native { triple },
                KirProfileLayout::Known {
                    pointer_width_bits, ..
                },
                KirCpuIdentity::Native { name, .. },
            ) if !triple.is_empty()
                && matches!(pointer_width_bits, 16 | 32 | 64 | 128)
                && !name.is_empty() =>
            {
                Ok(())
            }
            _ => Err("KIR target profile has contradictory identity or layout data".to_string()),
        }
    }

    fn encode_without_digest(&self) -> Vec<u8> {
        encode_profile_data(&self.0)
    }
}

pub struct KirTargetProfileBuilder {
    consumer: KirConsumer,
    target_identity: KirTargetIdentity,
    layout: KirProfileLayout,
    cpu_identity: KirCpuIdentity,
    legal_vector_widths: BTreeSet<u16>,
    legal_lane_types: BTreeSet<KirLaneType>,
    maximum_interleave_factor: u8,
    costs: BTreeMap<KirCostKey, KirOperationAvailability>,
    llvm_identity: Option<String>,
    bridge_identity: Option<String>,
}

impl KirTargetProfileBuilder {
    pub fn native(
        consumer: KirConsumer,
        triple: impl Into<String>,
        pointer_width_bits: u16,
        little_endian: bool,
        policy: KirNativeCpuPolicy,
        cpu_name: impl Into<String>,
        features: Vec<String>,
    ) -> Result<Self, String> {
        if !matches!(
            consumer,
            KirConsumer::NativeLibrary | KirConsumer::NativeExecutable
        ) {
            return Err("native KIR target profile builder requires a Native consumer".to_string());
        }
        Ok(Self {
            consumer,
            target_identity: KirTargetIdentity::Native {
                triple: triple.into(),
            },
            layout: KirProfileLayout::Known {
                pointer_width_bits,
                little_endian,
            },
            cpu_identity: KirCpuIdentity::Native {
                policy,
                name: cpu_name.into(),
                features,
            },
            legal_vector_widths: BTreeSet::new(),
            legal_lane_types: BTreeSet::new(),
            maximum_interleave_factor: 1,
            costs: portable_cost_entries().into_iter().collect(),
            llvm_identity: None,
            bridge_identity: None,
        })
    }

    pub fn set_legal(&mut self, key: KirCostKey, cost: KirLegalCost) -> Result<(), String> {
        if !cost_universe().contains(&key) {
            return Err("cost key is outside the KIR target profile query universe".to_string());
        }
        if key.lanes > 1 {
            self.legal_lane_types.insert(key.lane);
            self.legal_vector_widths
                .insert(key.lane.bit_width() * u16::from(key.lanes));
        }
        self.costs
            .insert(key, KirOperationAvailability::Legal(cost));
        Ok(())
    }

    pub fn set_unavailable(&mut self, key: KirCostKey) -> Result<(), String> {
        if !cost_universe().contains(&key) {
            return Err("cost key is outside the KIR target profile query universe".to_string());
        }
        self.costs
            .insert(key, KirOperationAvailability::Unavailable);
        Ok(())
    }

    pub fn set_maximum_interleave_factor(&mut self, factor: u8) {
        self.maximum_interleave_factor = factor;
    }

    pub fn set_producer_identity(
        &mut self,
        llvm_identity: impl Into<String>,
        bridge_identity: impl Into<String>,
    ) {
        self.llvm_identity = Some(llvm_identity.into());
        self.bridge_identity = Some(bridge_identity.into());
    }

    pub fn build(self) -> Result<KirTargetProfile, String> {
        KirTargetProfile::new(
            self.consumer,
            self.target_identity,
            self.layout,
            self.cpu_identity,
            self.legal_vector_widths,
            self.legal_lane_types,
            self.maximum_interleave_factor,
            self.costs.into_iter().collect(),
            self.llvm_identity,
            self.bridge_identity,
        )
    }
}

fn encode_profile_data(profile: &KirTargetProfileData) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"CK-KIR-TARGET-PROFILE\0");
    put_u16(&mut bytes, profile.schema_version);
    bytes.push(consumer_tag(profile.consumer));
    encode_target(&mut bytes, &profile.target_identity);
    encode_layout(&mut bytes, profile.layout);
    encode_cpu(&mut bytes, &profile.cpu_identity);
    put_u32(&mut bytes, profile.legal_vector_widths.len() as u32);
    for width in &profile.legal_vector_widths {
        put_u16(&mut bytes, *width);
    }
    put_u32(&mut bytes, profile.legal_lane_types.len() as u32);
    for lane in &profile.legal_lane_types {
        bytes.push(lane.tag());
    }
    bytes.push(profile.maximum_interleave_factor);
    put_u32(&mut bytes, profile.costs.len() as u32);
    for (key, availability) in &profile.costs {
        bytes.push(key.operation.tag());
        bytes.push(key.lane.tag());
        bytes.push(key.lanes);
        bytes.push(key.semantics.tag());
        match key.alignment {
            KirAlignmentClass::NotApplicable => bytes.push(0),
            KirAlignmentClass::Bytes(alignment) => {
                bytes.push(1);
                put_u16(&mut bytes, alignment);
            }
        }
        match availability {
            KirOperationAvailability::Unavailable => bytes.push(0),
            KirOperationAvailability::Legal(cost) => {
                bytes.push(1);
                put_u32(&mut bytes, cost.cost);
                put_u16(&mut bytes, cost.legalization_parts);
                put_string(&mut bytes, &cost.legalized_type);
            }
        }
    }
    put_optional_string(&mut bytes, profile.llvm_identity.as_deref());
    put_optional_string(&mut bytes, profile.bridge_identity.as_deref());
    bytes
}

fn portable_cost_entries() -> Vec<(KirCostKey, KirOperationAvailability)> {
    cost_universe()
        .iter()
        .cloned()
        .map(|key| {
            let availability = if key.lanes == 1
                && (key.operation != KirProfileOperation::MaskNot || key.lane == KIR_MASK_COST_LANE)
            {
                KirOperationAvailability::Legal(KirLegalCost {
                    cost: portable_scalar_cost(key.operation),
                    legalization_parts: 1,
                    legalized_type: format!("{:?}", key.lane).to_ascii_lowercase(),
                })
            } else {
                KirOperationAvailability::Unavailable
            };
            (key, availability)
        })
        .collect()
}

fn wasm_simd128_operation_cost(key: &KirCostKey) -> Option<u32> {
    let narrow_integer_shape = matches!(
        (key.lane, key.lanes),
        (KirLaneType::I32 | KirLaneType::U32, 2)
    );
    if narrow_integer_shape {
        return match key.operation {
            KirProfileOperation::Splat | KirProfileOperation::Cast
                if key.semantics == KirCostSemantics::NotApplicable
                    && key.alignment == KirAlignmentClass::NotApplicable =>
            {
                Some(1)
            }
            KirProfileOperation::Load
                if key.semantics == KirCostSemantics::NotApplicable
                    && key.alignment == KirAlignmentClass::Bytes(4) =>
            {
                Some(2)
            }
            KirProfileOperation::RuntimePredicate
                if key.lane == KirLaneType::U32
                    && key.semantics == KirCostSemantics::NotApplicable
                    && key.alignment == KirAlignmentClass::NotApplicable =>
            {
                Some(16)
            }
            _ => None,
        };
    }
    let supported_shape = matches!(
        (key.lane, key.lanes),
        (KirLaneType::F64, 2) | (KirLaneType::I32 | KirLaneType::U32, 4)
    );
    if !supported_shape {
        return None;
    }
    match key.operation {
        KirProfileOperation::Splat
            if key.semantics == KirCostSemantics::NotApplicable
                && key.alignment == KirAlignmentClass::NotApplicable =>
        {
            Some(1)
        }
        KirProfileOperation::Add
        | KirProfileOperation::Subtract
        | KirProfileOperation::Multiply
        | KirProfileOperation::Negate
            if key.alignment == KirAlignmentClass::NotApplicable
                && key.semantics
                    == if key.lane == KirLaneType::F64 {
                        KirCostSemantics::StrictFloat
                    } else {
                        KirCostSemantics::Modular
                    } =>
        {
            Some(1)
        }
        KirProfileOperation::Divide
            if key.lane == KirLaneType::F64
                && key.semantics == KirCostSemantics::StrictFloat
                && key.alignment == KirAlignmentClass::NotApplicable =>
        {
            Some(8)
        }
        KirProfileOperation::Compare | KirProfileOperation::Select
            if key.semantics == KirCostSemantics::NotApplicable
                && key.alignment == KirAlignmentClass::NotApplicable =>
        {
            Some(1)
        }
        KirProfileOperation::ReduceAdd | KirProfileOperation::ReduceMultiply
            if matches!(key.lane, KirLaneType::I32 | KirLaneType::U32)
                && key.semantics == KirCostSemantics::Modular
                && key.alignment == KirAlignmentClass::NotApplicable =>
        {
            // A Wasm horizontal fold costs four lane extracts, three modular
            // operations to combine them, and one operation with the carried
            // scalar accumulator.
            Some(8)
        }
        KirProfileOperation::RuntimePredicate
            if key.lane == KirLaneType::U32
                && key.semantics == KirCostSemantics::NotApplicable
                && key.alignment == KirAlignmentClass::NotApplicable =>
        {
            Some(16)
        }
        KirProfileOperation::Load | KirProfileOperation::Store
            if key.semantics == KirCostSemantics::NotApplicable
                && key.alignment
                    == KirAlignmentClass::Bytes(match key.lane {
                        KirLaneType::F64 => 8,
                        KirLaneType::I32 | KirLaneType::U32 => 4,
                        KirLaneType::I64 | KirLaneType::U64 => return None,
                    }) =>
        {
            Some(2)
        }
        _ => None,
    }
}

fn wasm_simd128_type(lane: KirLaneType, lanes: u8) -> String {
    let lane_name = match lane {
        KirLaneType::I32 => "i32",
        KirLaneType::U32 => "u32",
        KirLaneType::F64 => "f64",
        KirLaneType::I64 => "i64",
        KirLaneType::U64 => "u64",
    };
    format!("{lane_name}x{lanes}")
}

fn cost_universe() -> &'static BTreeSet<KirCostKey> {
    static UNIVERSE: OnceLock<BTreeSet<KirCostKey>> = OnceLock::new();
    UNIVERSE.get_or_init(|| {
        let mut keys = BTreeSet::new();
        for lane in KirLaneType::ALL {
            for lanes in [1, 2, 4, 8, 16] {
                if u16::from(lanes) * lane.bit_width() > 512 {
                    continue;
                }
                for operation in KirProfileOperation::ALL {
                    for semantics in operation_semantics(operation, lane) {
                        if operation.uses_alignment() {
                            let byte_width = u16::from(lanes) * lane.bit_width() / 8;
                            let mut alignment = 1;
                            while alignment <= byte_width {
                                keys.insert(KirCostKey {
                                    operation,
                                    lane,
                                    lanes,
                                    semantics: *semantics,
                                    alignment: KirAlignmentClass::Bytes(alignment),
                                });
                                alignment *= 2;
                            }
                        } else {
                            keys.insert(KirCostKey {
                                operation,
                                lane,
                                lanes,
                                semantics: *semantics,
                                alignment: KirAlignmentClass::NotApplicable,
                            });
                        }
                    }
                }
            }
        }
        keys
    })
}

fn operation_semantics(
    operation: KirProfileOperation,
    lane: KirLaneType,
) -> &'static [KirCostSemantics] {
    const NONE: &[KirCostSemantics] = &[KirCostSemantics::NotApplicable];
    const INTEGER: &[KirCostSemantics] = &[KirCostSemantics::Modular, KirCostSemantics::Checked];
    const FLOAT: &[KirCostSemantics] = &[KirCostSemantics::StrictFloat];
    if matches!(
        operation,
        KirProfileOperation::Add
            | KirProfileOperation::Subtract
            | KirProfileOperation::Multiply
            | KirProfileOperation::Divide
            | KirProfileOperation::Remainder
            | KirProfileOperation::Negate
            | KirProfileOperation::ReduceAdd
            | KirProfileOperation::ReduceMultiply
            | KirProfileOperation::ReduceMin
            | KirProfileOperation::ReduceMax
    ) {
        if lane == KirLaneType::F64 {
            FLOAT
        } else {
            INTEGER
        }
    } else {
        NONE
    }
}

const fn portable_scalar_cost(operation: KirProfileOperation) -> u32 {
    match operation {
        KirProfileOperation::Divide | KirProfileOperation::Remainder => 8,
        KirProfileOperation::Load | KirProfileOperation::Store => 2,
        _ => 1,
    }
}

const fn consumer_tag(consumer: KirConsumer) -> u8 {
    match consumer {
        KirConsumer::C => 1,
        KirConsumer::WebAssembly => 2,
        KirConsumer::NativeLibrary => 3,
        KirConsumer::NativeExecutable => 4,
        KirConsumer::Inspection => 5,
    }
}

fn encode_target(bytes: &mut Vec<u8>, target: &KirTargetIdentity) {
    match target {
        KirTargetIdentity::Inspection => bytes.push(1),
        KirTargetIdentity::PortableC => bytes.push(2),
        KirTargetIdentity::WebAssembly => bytes.push(3),
        KirTargetIdentity::WebAssemblySimd128 => bytes.push(5),
        KirTargetIdentity::Native { triple } => {
            bytes.push(4);
            put_string(bytes, triple);
        }
    }
}

fn encode_layout(bytes: &mut Vec<u8>, layout: KirProfileLayout) {
    match layout {
        KirProfileLayout::PortableUnknown => bytes.push(0),
        KirProfileLayout::Known {
            pointer_width_bits,
            little_endian,
        } => {
            bytes.push(1);
            put_u16(bytes, pointer_width_bits);
            bytes.push(u8::from(little_endian));
        }
    }
}

fn encode_cpu(bytes: &mut Vec<u8>, cpu: &KirCpuIdentity) {
    match cpu {
        KirCpuIdentity::NotApplicable => bytes.push(0),
        KirCpuIdentity::Native {
            policy,
            name,
            features,
        } => {
            bytes.push(1);
            bytes.push(match policy {
                KirNativeCpuPolicy::Baseline => 1,
                KirNativeCpuPolicy::Native => 2,
                KirNativeCpuPolicy::Multiversion => 3,
            });
            put_string(bytes, name);
            put_u32(bytes, features.len() as u32);
            for feature in features {
                put_string(bytes, feature);
            }
        }
    }
}

fn put_optional_string(bytes: &mut Vec<u8>, value: Option<&str>) {
    match value {
        Some(value) => {
            bytes.push(1);
            put_string(bytes, value);
        }
        None => bytes.push(0),
    }
}

fn put_string(bytes: &mut Vec<u8>, value: &str) {
    put_u32(bytes, value.len() as u32);
    bytes.extend_from_slice(value.as_bytes());
}

fn put_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_be_bytes());
}

fn put_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_be_bytes());
}

fn hex_digest(digest: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    let mut text = String::with_capacity(64);
    for byte in digest {
        write!(&mut text, "{byte:02x}").expect("writing to String cannot fail");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_validator_rejects_missing_duplicate_zero_and_stale_entries() {
        let mut missing = KirTargetProfile::inspection();
        let data = Arc::make_mut(&mut missing.0);
        data.costs.pop_first();
        let digest = Sha256::digest(encode_profile_data(data));
        data.digest.copy_from_slice(&digest);
        assert_eq!(
            missing.validate(),
            Err("KIR target profile cost universe is incomplete".to_string())
        );

        let mut entries = portable_cost_entries();
        entries.push(entries[0].clone());
        assert_eq!(
            KirTargetProfile::new(
                KirConsumer::Inspection,
                KirTargetIdentity::Inspection,
                KirProfileLayout::PortableUnknown,
                KirCpuIdentity::NotApplicable,
                BTreeSet::new(),
                BTreeSet::new(),
                1,
                entries,
                None,
                None,
            ),
            Err("KIR target profile contains a duplicate cost key".to_string())
        );

        let mut zero = KirTargetProfile::inspection();
        let data = Arc::make_mut(&mut zero.0);
        let cost = data
            .costs
            .values_mut()
            .find_map(|availability| match availability {
                KirOperationAvailability::Legal(cost) => Some(cost),
                KirOperationAvailability::Unavailable => None,
            })
            .expect("portable scalar cost");
        cost.cost = 0;
        let digest = Sha256::digest(encode_profile_data(data));
        data.digest.copy_from_slice(&digest);
        assert_eq!(
            zero.validate(),
            Err("KIR target profile contains a zero cost for emitted work".to_string())
        );

        let mut stale = KirTargetProfile::inspection();
        Arc::make_mut(&mut stale.0).maximum_interleave_factor = 2;
        assert_eq!(
            stale.validate(),
            Err("KIR target profile digest is stale".to_string())
        );
    }

    #[test]
    fn profile_encoding_changes_for_every_portable_identity_and_layout_mutation() {
        let inspection = KirTargetProfile::inspection();
        let c = KirTargetProfile::portable_c();
        let wasm = KirTargetProfile::webassembly();
        assert_ne!(inspection.canonical_bytes(), c.canonical_bytes());
        assert_ne!(c.canonical_bytes(), wasm.canonical_bytes());

        let mut mutated = inspection.clone();
        Arc::make_mut(&mut mutated.0).layout = KirProfileLayout::Known {
            pointer_width_bits: 64,
            little_endian: true,
        };
        assert_ne!(inspection.canonical_bytes(), mutated.canonical_bytes());
        assert!(Arc::ptr_eq(
            &KirTargetProfile::inspection().0,
            &KirTargetProfile::inspection().0
        ));
    }

    #[test]
    fn wasm_feature_profiles_preserve_the_p5_baseline_identity() {
        let baseline = KirTargetProfile::webassembly_with_features(KirWasmFeatures::Baseline);
        let simd128 = KirTargetProfile::webassembly_with_features(KirWasmFeatures::Simd128);

        assert_eq!(baseline, KirTargetProfile::webassembly());
        assert_eq!(baseline.target_identity(), &KirTargetIdentity::WebAssembly);
        assert_eq!(
            baseline.digest_hex(),
            "ce0ccf53cf133340ba86406a3ba056c53e43411b1ae986eb97847279073f4238"
        );
        assert_eq!(baseline.wasm_features(), Some(KirWasmFeatures::Baseline));
        assert_eq!(simd128.wasm_features(), Some(KirWasmFeatures::Simd128));
        assert_eq!(
            simd128.target_identity(),
            &KirTargetIdentity::WebAssemblySimd128
        );
        assert_ne!(baseline.canonical_bytes(), simd128.canonical_bytes());
        assert!(baseline.validate().is_ok());
        assert!(simd128.validate().is_ok());
        assert!(!baseline.vector_operations_enabled());
        assert!(!baseline.supports_vector_shape(KirLaneType::I32, 4));
        assert!(
            KirTargetProfile::fixed_query_universe()
                .iter()
                .filter(|key| key.lanes > 1)
                .all(|key| matches!(
                    baseline.operation_availability(key),
                    Some(KirOperationAvailability::Unavailable)
                ))
        );
    }

    #[test]
    fn wasm_simd128_profile_exposes_only_the_minimum_loop_vector_subset() {
        let profile = KirTargetProfile::webassembly_with_features(KirWasmFeatures::Simd128);
        assert_eq!(
            profile.digest_hex(),
            "a73e04ad8467cda7a2ac3e9145167a9b5433b21265b0b487261bb525e65417d8"
        );
        assert!(profile.vector_operations_enabled());
        assert_eq!(profile.maximum_interleave_factor(), 1);
        assert!(profile.supports_vector_shape(KirLaneType::F64, 2));
        assert!(profile.supports_vector_shape(KirLaneType::I32, 4));
        assert!(profile.supports_vector_shape(KirLaneType::U32, 4));
        assert!(profile.supports_vector_shape(KirLaneType::I32, 2));
        assert!(profile.supports_vector_shape(KirLaneType::U32, 2));
        assert!(!profile.supports_vector_shape(KirLaneType::F64, 4));
        assert!(!profile.supports_vector_shape(KirLaneType::I64, 2));
        assert!(!profile.supports_vector_shape(KirLaneType::U64, 2));

        let legal_vectors = KirTargetProfile::fixed_query_universe()
            .into_iter()
            .filter(|key| {
                key.lanes > 1
                    && matches!(
                        profile.operation_availability(key),
                        Some(KirOperationAvailability::Legal(_))
                    )
            })
            .collect::<Vec<_>>();
        assert_eq!(legal_vectors.len(), 40);
        assert!(legal_vectors.iter().all(|key| {
            let natural_alignment = match key.lane {
                KirLaneType::F64 => KirAlignmentClass::Bytes(8),
                KirLaneType::I32 | KirLaneType::U32 => KirAlignmentClass::Bytes(4),
                KirLaneType::I64 | KirLaneType::U64 => return false,
            };
            let full_width_shape = matches!(
                (key.lane, key.lanes),
                (KirLaneType::F64, 2) | (KirLaneType::I32, 4) | (KirLaneType::U32, 4)
            );
            let narrow_cast_shape = matches!(
                (key.lane, key.lanes),
                (KirLaneType::I32, 2) | (KirLaneType::U32, 2)
            );
            match key.operation {
                KirProfileOperation::Splat => {
                    (full_width_shape || narrow_cast_shape)
                        && key.semantics == KirCostSemantics::NotApplicable
                        && key.alignment == KirAlignmentClass::NotApplicable
                }
                KirProfileOperation::Cast => {
                    narrow_cast_shape
                        && key.semantics == KirCostSemantics::NotApplicable
                        && key.alignment == KirAlignmentClass::NotApplicable
                }
                KirProfileOperation::Add
                | KirProfileOperation::Subtract
                | KirProfileOperation::Multiply
                | KirProfileOperation::Negate => {
                    full_width_shape
                        && key.semantics
                            == if key.lane == KirLaneType::F64 {
                                KirCostSemantics::StrictFloat
                            } else {
                                KirCostSemantics::Modular
                            }
                        && key.alignment == KirAlignmentClass::NotApplicable
                }
                KirProfileOperation::Divide => {
                    full_width_shape
                        && key.lane == KirLaneType::F64
                        && key.semantics == KirCostSemantics::StrictFloat
                        && key.alignment == KirAlignmentClass::NotApplicable
                }
                KirProfileOperation::Compare | KirProfileOperation::Select => {
                    full_width_shape
                        && key.semantics == KirCostSemantics::NotApplicable
                        && key.alignment == KirAlignmentClass::NotApplicable
                }
                KirProfileOperation::RuntimePredicate => {
                    matches!((key.lane, key.lanes), (KirLaneType::U32, 2 | 4))
                        && key.semantics == KirCostSemantics::NotApplicable
                        && key.alignment == KirAlignmentClass::NotApplicable
                }
                KirProfileOperation::ReduceAdd | KirProfileOperation::ReduceMultiply => {
                    matches!(key.lane, KirLaneType::I32 | KirLaneType::U32)
                        && key.lanes == 4
                        && key.semantics == KirCostSemantics::Modular
                        && key.alignment == KirAlignmentClass::NotApplicable
                }
                KirProfileOperation::Load => {
                    (full_width_shape && key.alignment == natural_alignment
                        || narrow_cast_shape && key.alignment == KirAlignmentClass::Bytes(4))
                        && key.semantics == KirCostSemantics::NotApplicable
                }
                KirProfileOperation::Store => {
                    full_width_shape
                        && key.semantics == KirCostSemantics::NotApplicable
                        && key.alignment == natural_alignment
                }
                _ => false,
            }
        }));
        assert!(legal_vectors.iter().all(|key| {
            let cost = match profile.operation_availability(key) {
                Some(KirOperationAvailability::Legal(cost)) => cost,
                _ => unreachable!("legal vector query was collected above"),
            };
            let expected_cost = match key.operation {
                KirProfileOperation::Divide => 8,
                KirProfileOperation::Load | KirProfileOperation::Store => 2,
                KirProfileOperation::RuntimePredicate => 16,
                KirProfileOperation::ReduceAdd | KirProfileOperation::ReduceMultiply => 8,
                _ => 1,
            };
            cost.cost == expected_cost
                && cost.legalization_parts == 1
                && cost.legalized_type == wasm_simd128_type(key.lane, key.lanes)
        }));
    }

    #[test]
    fn immutable_profile_validation_should_be_memoized_and_copy_on_write_should_invalidate_it() {
        let profile = KirTargetProfile::inspection();
        assert!(profile.validate().is_ok());
        assert!(
            profile.0.validation.get().is_some(),
            "a validated immutable profile should retain its validation result"
        );

        let mut mutated = profile.clone();
        Arc::make_mut(&mut mutated.0).maximum_interleave_factor = 0;
        assert!(
            mutated.0.validation.get().is_none(),
            "copy-on-write mutation must discard the source profile's cached validation"
        );
        assert_eq!(
            mutated.validate(),
            Err("KIR target profile has an invalid interleave factor".to_string())
        );
    }
}
