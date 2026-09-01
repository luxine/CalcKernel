use sha2::{Digest, Sha256};

use super::{
    FunctionId, KIR_MASK_COST_LANE, KirConsumer, KirCpuIdentity, KirLegalCost, KirModule,
    KirMultiversionTierId::*, KirNativeCpuPolicy, KirProfileLayout, KirProfileOperation,
    KirTargetIdentity, KirTargetProfile, KirTargetProfileBuilder, print_kir_module,
};

/// Closed compiler-owned CPU target-set schema used by CK 0.13.
pub const KIR_MULTIVERSION_TARGET_SET_SCHEMA: u16 = 1;
pub const KIR_MULTIVERSION_BUNDLE_SCHEMA: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KirTargetArchitecture {
    X86_64,
    AArch64,
}

impl KirTargetArchitecture {
    #[must_use]
    pub const fn stable_name(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::AArch64 => "aarch64",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KirTargetOperatingSystem {
    Linux,
    Darwin,
    Windows,
}

impl KirTargetOperatingSystem {
    #[must_use]
    pub const fn stable_name(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Darwin => "darwin",
            Self::Windows => "windows",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KirMultiversionPlatform {
    pub architecture: KirTargetArchitecture,
    pub operating_system: KirTargetOperatingSystem,
}

impl KirMultiversionPlatform {
    pub fn from_triple(triple: &str) -> Result<Self, String> {
        let architecture = if triple.starts_with("x86_64-") {
            KirTargetArchitecture::X86_64
        } else if triple.starts_with("aarch64-") || triple.starts_with("arm64-") {
            KirTargetArchitecture::AArch64
        } else {
            return Err(format!(
                "unsupported multiversion target architecture: {triple}"
            ));
        };
        let operating_system = if triple.contains("linux") {
            KirTargetOperatingSystem::Linux
        } else if triple.contains("darwin") || triple.contains("apple") {
            KirTargetOperatingSystem::Darwin
        } else if triple.contains("windows") || triple.contains("msvc") {
            KirTargetOperatingSystem::Windows
        } else {
            return Err(format!(
                "unsupported multiversion target operating system: {triple}"
            ));
        };
        Ok(Self {
            architecture,
            operating_system,
        })
    }

    #[must_use]
    pub const fn canonical_triple(self) -> &'static str {
        match (self.architecture, self.operating_system) {
            (KirTargetArchitecture::X86_64, KirTargetOperatingSystem::Linux) => {
                "x86_64-unknown-linux-gnu"
            }
            (KirTargetArchitecture::X86_64, KirTargetOperatingSystem::Darwin) => {
                "x86_64-apple-darwin"
            }
            (KirTargetArchitecture::X86_64, KirTargetOperatingSystem::Windows) => {
                "x86_64-pc-windows-msvc"
            }
            (KirTargetArchitecture::AArch64, KirTargetOperatingSystem::Linux) => {
                "aarch64-unknown-linux-gnu"
            }
            (KirTargetArchitecture::AArch64, KirTargetOperatingSystem::Darwin) => {
                "aarch64-apple-darwin"
            }
            (KirTargetArchitecture::AArch64, KirTargetOperatingSystem::Windows) => {
                "aarch64-pc-windows-msvc"
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KirMultiversionTierId {
    Baseline,
    X86_64V3,
    X86_64V4,
    AArch64Sve,
    AArch64Sve2,
}

impl KirMultiversionTierId {
    #[must_use]
    pub const fn stable_name(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::X86_64V3 => "x86-64-v3",
            Self::X86_64V4 => "x86-64-v4",
            Self::AArch64Sve => "aarch64-sve",
            Self::AArch64Sve2 => "aarch64-sve2",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirRuntimeFeaturePredicate {
    pub detector: String,
    pub hardware_features: Vec<String>,
    pub os_state: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirMultiversionTargetTier {
    pub id: KirMultiversionTierId,
    pub triple: String,
    pub cpu: String,
    pub llvm_features: Vec<String>,
    pub required_features: Vec<String>,
    pub data_layout: String,
    pub profile: KirTargetProfile,
    pub llvm_identity: String,
    pub bridge_identity: String,
    pub predicate: KirRuntimeFeaturePredicate,
    pub digest: [u8; 32],
}

impl KirMultiversionTargetTier {
    #[must_use]
    pub fn digest_hex(&self) -> String {
        hex_digest(&self.digest)
    }

    fn canonical_bytes_without_digest(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"CK-KIR-MULTIVERSION-TIER\0");
        put_string(&mut bytes, self.id.stable_name());
        put_string(&mut bytes, &self.triple);
        put_string(&mut bytes, &self.cpu);
        put_strings(&mut bytes, &self.llvm_features);
        put_strings(&mut bytes, &self.required_features);
        put_string(&mut bytes, &self.data_layout);
        put_bytes(&mut bytes, &self.profile.canonical_bytes());
        put_string(&mut bytes, &self.llvm_identity);
        put_string(&mut bytes, &self.bridge_identity);
        put_string(&mut bytes, &self.predicate.detector);
        put_strings(&mut bytes, &self.predicate.hardware_features);
        put_strings(&mut bytes, &self.predicate.os_state);
        bytes
    }

    fn validate(
        &self,
        platform: KirMultiversionPlatform,
        consumer: KirConsumer,
    ) -> Result<(), String> {
        let spec = tier_spec(platform, self.id)
            .ok_or_else(|| "multiversion tier is outside the closed schema-1 table".to_string())?;
        if self.cpu != spec.cpu
            || self.llvm_features != strings(spec.llvm_features)
            || self.required_features != strings(spec.required_features)
            || self.predicate.detector != spec.detector
            || self.predicate.hardware_features != self.required_features
            || self.predicate.os_state != strings(spec.os_state)
        {
            return Err("multiversion tier feature or predicate identity mismatch".to_string());
        }
        if self.triple.is_empty() || KirMultiversionPlatform::from_triple(&self.triple)? != platform
        {
            return Err("multiversion tier triple does not match target-set platform".to_string());
        }
        if self.data_layout.is_empty() {
            return Err("multiversion tier data layout is empty".to_string());
        }
        self.profile.validate()?;
        if self.profile.consumer() != consumer
            || self.profile.target_identity()
                != &(KirTargetIdentity::Native {
                    triple: self.triple.clone(),
                })
        {
            return Err("multiversion tier profile identity mismatch".to_string());
        }
        let KirCpuIdentity::Native {
            policy,
            name,
            features,
        } = self.profile.cpu_identity()
        else {
            return Err("multiversion tier profile has no Native CPU identity".to_string());
        };
        if *policy != KirNativeCpuPolicy::Multiversion || name != &self.cpu {
            return Err("multiversion tier profile CPU policy mismatch".to_string());
        }
        if !self
            .llvm_features
            .iter()
            .all(|feature| features.contains(feature))
        {
            return Err("multiversion tier profile omits a declared LLVM feature".to_string());
        }
        if self.profile.layout()
            != (KirProfileLayout::Known {
                pointer_width_bits: 64,
                little_endian: true,
            })
        {
            return Err("multiversion schema 1 requires a little-endian 64-bit layout".to_string());
        }
        if self.profile.producer_identity()
            != (
                Some(self.llvm_identity.as_str()),
                Some(self.bridge_identity.as_str()),
            )
        {
            return Err("multiversion tier producer identity mismatch".to_string());
        }
        let expected = Sha256::digest(self.canonical_bytes_without_digest());
        if expected.as_slice() != self.digest {
            return Err("multiversion tier digest is stale".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirMultiversionTargetSet {
    pub schema_version: u16,
    pub platform: KirMultiversionPlatform,
    pub consumer: KirConsumer,
    pub tiers: Vec<KirMultiversionTargetTier>,
    pub digest: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirMultiversionHiddenSymbol {
    pub source_name: String,
    pub hidden_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirMultiversionVariant {
    pub root: FunctionId,
    pub tier: KirMultiversionTierId,
    pub module: KirModule,
    pub logical_pre_state_digest: [u8; 32],
    pub target_profile_digest: String,
    pub required_features: Vec<String>,
    pub predicted_baseline_cost: u64,
    pub predicted_variant_cost: u64,
    pub kir_units: u32,
    pub proof_digest: [u8; 32],
    pub feature_audit_digest: [u8; 32],
    pub codegen_digest: [u8; 32],
    pub hidden_symbols: Vec<KirMultiversionHiddenSymbol>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirMultiversionRootBundle {
    pub root: FunctionId,
    pub public_symbol: String,
    pub variants: Vec<KirMultiversionVariant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirMultiversionDispatchEntry {
    pub root: FunctionId,
    pub public_symbol: String,
    pub ranked_tiers: Vec<KirMultiversionTierId>,
    pub implementation_symbols: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirMultiversionExplanation {
    pub root: FunctionId,
    pub tier: Option<KirMultiversionTierId>,
    pub accepted: bool,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirMultiversionBundle {
    pub schema_version: u16,
    pub target_set: KirMultiversionTargetSet,
    pub logical_pre_state_digest: [u8; 32],
    pub baseline: KirModule,
    pub baseline_kir_units: u32,
    pub shared_growth_consumed_before: u32,
    pub trial_audit_units: u32,
    pub additional_kir_units: u32,
    pub total_kir_units: u32,
    pub roots: Vec<KirMultiversionRootBundle>,
    pub dispatch_plan: Vec<KirMultiversionDispatchEntry>,
    pub explanations: Vec<KirMultiversionExplanation>,
    pub digest: [u8; 32],
}

impl KirMultiversionBundle {
    #[must_use]
    pub fn digest_hex(&self) -> String {
        hex_digest(&self.digest)
    }

    #[must_use]
    pub fn canonical_bytes_without_digest(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"CK-KIR-MULTIVERSION-BUNDLE\0");
        bytes.extend_from_slice(&self.schema_version.to_be_bytes());
        bytes.extend_from_slice(&self.target_set.digest);
        bytes.extend_from_slice(&self.logical_pre_state_digest);
        put_bytes(&mut bytes, print_kir_module(&self.baseline).as_bytes());
        bytes.extend_from_slice(&self.baseline_kir_units.to_be_bytes());
        bytes.extend_from_slice(&self.shared_growth_consumed_before.to_be_bytes());
        bytes.extend_from_slice(&self.trial_audit_units.to_be_bytes());
        bytes.extend_from_slice(&self.additional_kir_units.to_be_bytes());
        bytes.extend_from_slice(&self.total_kir_units.to_be_bytes());
        put_u32(&mut bytes, self.roots.len());
        for root in &self.roots {
            bytes.extend_from_slice(&root.root.index().to_be_bytes());
            put_string(&mut bytes, &root.public_symbol);
            put_u32(&mut bytes, root.variants.len());
            for variant in &root.variants {
                bytes.extend_from_slice(&variant.root.index().to_be_bytes());
                put_string(&mut bytes, variant.tier.stable_name());
                put_bytes(&mut bytes, print_kir_module(&variant.module).as_bytes());
                bytes.extend_from_slice(&variant.logical_pre_state_digest);
                put_string(&mut bytes, &variant.target_profile_digest);
                put_strings(&mut bytes, &variant.required_features);
                bytes.extend_from_slice(&variant.predicted_baseline_cost.to_be_bytes());
                bytes.extend_from_slice(&variant.predicted_variant_cost.to_be_bytes());
                bytes.extend_from_slice(&variant.kir_units.to_be_bytes());
                bytes.extend_from_slice(&variant.proof_digest);
                bytes.extend_from_slice(&variant.feature_audit_digest);
                bytes.extend_from_slice(&variant.codegen_digest);
                put_u32(&mut bytes, variant.hidden_symbols.len());
                for symbol in &variant.hidden_symbols {
                    put_string(&mut bytes, &symbol.source_name);
                    put_string(&mut bytes, &symbol.hidden_name);
                }
            }
        }
        put_u32(&mut bytes, self.dispatch_plan.len());
        for entry in &self.dispatch_plan {
            bytes.extend_from_slice(&entry.root.index().to_be_bytes());
            put_string(&mut bytes, &entry.public_symbol);
            put_u32(&mut bytes, entry.ranked_tiers.len());
            for tier in &entry.ranked_tiers {
                put_string(&mut bytes, tier.stable_name());
            }
            put_strings(&mut bytes, &entry.implementation_symbols);
        }
        put_u32(&mut bytes, self.explanations.len());
        for explanation in &self.explanations {
            bytes.extend_from_slice(&explanation.root.index().to_be_bytes());
            match explanation.tier {
                Some(tier) => {
                    bytes.push(1);
                    put_string(&mut bytes, tier.stable_name());
                }
                None => bytes.push(0),
            }
            bytes.push(u8::from(explanation.accepted));
            put_string(&mut bytes, &explanation.reason);
        }
        bytes
    }
}

#[must_use]
pub fn kir_multiversion_module_digest(module: &KirModule) -> [u8; 32] {
    Sha256::digest(print_kir_module(module).as_bytes()).into()
}

#[must_use]
pub fn print_kir_multiversion_bundle(bundle: &KirMultiversionBundle) -> String {
    let mut output = format!(
        "kir-multiversion-v{} target-set-sha256={} bundle-sha256={} pre-state-sha256={} baseline-units={} shared-growth-before={} trial-audit-units={} additional-units={} total-units={}\n",
        bundle.schema_version,
        bundle.target_set.digest_hex(),
        bundle.digest_hex(),
        hex_digest(&bundle.logical_pre_state_digest),
        bundle.baseline_kir_units,
        bundle.shared_growth_consumed_before,
        bundle.trial_audit_units,
        bundle.additional_kir_units,
        bundle.total_kir_units,
    );
    for tier in &bundle.target_set.tiers {
        output.push_str(&format!(
            "target-tier {} cpu={} llvm-features={} required-features={} predicate={} os-state={} profile-sha256={} tier-sha256={}\n",
            tier.id.stable_name(),
            tier.cpu,
            tier.llvm_features.join(","),
            tier.required_features.join(","),
            tier.predicate.detector,
            tier.predicate.os_state.join(","),
            tier.profile.digest_hex(),
            tier.digest_hex(),
        ));
    }
    output.push_str("\nverified-baseline\n");
    output.push_str(&print_kir_module(&bundle.baseline));
    for root in &bundle.roots {
        output.push_str(&format!(
            "\nmultiversion-root f{} {}\n",
            root.root.index(),
            root.public_symbol
        ));
        for variant in &root.variants {
            output.push_str(&format!(
                "variant tier={} cost={}->{} units={} proof={} feature-audit={} codegen={} hidden={}\n",
                variant.tier.stable_name(),
                variant.predicted_baseline_cost,
                variant.predicted_variant_cost,
                variant.kir_units,
                hex_digest(&variant.proof_digest),
                hex_digest(&variant.feature_audit_digest),
                hex_digest(&variant.codegen_digest),
                variant.hidden_symbols.iter().map(|symbol| symbol.hidden_name.as_str()).collect::<Vec<_>>().join(","),
            ));
            output.push_str(&print_kir_module(&variant.module));
        }
    }
    output.push_str("\ndispatch-plan\n");
    for entry in &bundle.dispatch_plan {
        output.push_str(&format!(
            "dispatch f{} {} tiers={} symbols={}\n",
            entry.root.index(),
            entry.public_symbol,
            entry
                .ranked_tiers
                .iter()
                .map(|tier| tier.stable_name())
                .collect::<Vec<_>>()
                .join(","),
            entry.implementation_symbols.join(","),
        ));
    }
    for explanation in &bundle.explanations {
        output.push_str(&format!(
            "multiversion-explain f{} tier={} accepted={} reason={}\n",
            explanation.root.index(),
            explanation
                .tier
                .map(KirMultiversionTierId::stable_name)
                .unwrap_or("none"),
            explanation.accepted,
            explanation.reason,
        ));
    }
    output
}

impl KirMultiversionTargetSet {
    pub fn schema1_for_triple(triple: &str, consumer: KirConsumer) -> Result<Self, String> {
        let platform = KirMultiversionPlatform::from_triple(triple)?;
        Self::schema1_fixture_with_triple(platform, consumer, triple)
    }

    pub fn schema1_fixture(
        platform: KirMultiversionPlatform,
        consumer: KirConsumer,
    ) -> Result<Self, String> {
        Self::schema1_fixture_with_triple(platform, consumer, platform.canonical_triple())
    }

    fn schema1_fixture_with_triple(
        platform: KirMultiversionPlatform,
        consumer: KirConsumer,
        triple: &str,
    ) -> Result<Self, String> {
        require_native_consumer(consumer)?;
        let mut tiers = Vec::new();
        for id in expected_tiers(platform) {
            let spec = tier_spec(platform, id).expect("expected tier has a schema record");
            let profile = fixture_profile(consumer, triple, spec)?;
            tiers.push(build_tier(
                id,
                triple.to_string(),
                format!(
                    "schema1-layout:{}:{}",
                    platform.architecture.stable_name(),
                    platform.operating_system.stable_name()
                ),
                profile,
                "LLVM 22.1.8 TCK_RecipThroughput".to_string(),
                "ckc-llvm-bridge-abi-4".to_string(),
                spec,
            ));
        }
        Self::from_materialized(platform, consumer, tiers)
    }

    pub fn from_materialized(
        platform: KirMultiversionPlatform,
        consumer: KirConsumer,
        tiers: Vec<KirMultiversionTargetTier>,
    ) -> Result<Self, String> {
        require_native_consumer(consumer)?;
        let mut target_set = Self {
            schema_version: KIR_MULTIVERSION_TARGET_SET_SCHEMA,
            platform,
            consumer,
            tiers,
            digest: [0; 32],
        };
        let digest = Sha256::digest(target_set.canonical_bytes_without_digest());
        target_set.digest.copy_from_slice(&digest);
        target_set.validate()?;
        Ok(target_set)
    }

    #[must_use]
    pub fn tier(&self, id: KirMultiversionTierId) -> Option<&KirMultiversionTargetTier> {
        self.tiers.iter().find(|tier| tier.id == id)
    }

    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        self.canonical_bytes_without_digest()
    }

    #[must_use]
    pub fn digest_hex(&self) -> String {
        hex_digest(&self.digest)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != KIR_MULTIVERSION_TARGET_SET_SCHEMA {
            return Err("unsupported multiversion target-set schema".to_string());
        }
        require_native_consumer(self.consumer)?;
        let expected = expected_tiers(self.platform);
        if self.tiers.iter().map(|tier| tier.id).collect::<Vec<_>>() != expected {
            return Err("multiversion target-set tier order is not canonical".to_string());
        }
        for tier in &self.tiers {
            tier.validate(self.platform, self.consumer)?;
        }
        if self
            .tiers
            .windows(2)
            .any(|pair| pair[0].triple != pair[1].triple)
            || self
                .tiers
                .windows(2)
                .any(|pair| pair[0].data_layout != pair[1].data_layout)
        {
            return Err("multiversion tiers disagree on triple or data layout".to_string());
        }
        let expected = Sha256::digest(self.canonical_bytes_without_digest());
        if expected.as_slice() != self.digest {
            return Err("multiversion target-set digest is stale".to_string());
        }
        Ok(())
    }

    fn canonical_bytes_without_digest(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"CK-KIR-MULTIVERSION-TARGET-SET\0");
        bytes.extend_from_slice(&self.schema_version.to_be_bytes());
        put_string(&mut bytes, self.platform.architecture.stable_name());
        put_string(&mut bytes, self.platform.operating_system.stable_name());
        bytes.push(consumer_tag(self.consumer));
        put_u32(&mut bytes, self.tiers.len());
        for tier in &self.tiers {
            bytes.extend_from_slice(&tier.digest);
        }
        bytes
    }
}

struct TierSpec {
    cpu: &'static str,
    llvm_features: &'static [&'static str],
    required_features: &'static [&'static str],
    detector: &'static str,
    os_state: &'static [&'static str],
    fixture_cost_percent: u32,
}

const X86_V3_FEATURES: &[&str] = &[
    "avx",
    "avx2",
    "bmi1",
    "bmi2",
    "cx16",
    "f16c",
    "fma",
    "lahf-sahf",
    "lzcnt",
    "movbe",
    "popcnt",
    "sse3",
    "sse4.1",
    "sse4.2",
    "ssse3",
    "xsave",
];
const X86_V4_FEATURES: &[&str] = &[
    "avx",
    "avx2",
    "avx512bw",
    "avx512cd",
    "avx512dq",
    "avx512f",
    "avx512vl",
    "bmi1",
    "bmi2",
    "cx16",
    "f16c",
    "fma",
    "lahf-sahf",
    "lzcnt",
    "movbe",
    "popcnt",
    "sse3",
    "sse4.1",
    "sse4.2",
    "ssse3",
    "xsave",
];

fn tier_spec(
    platform: KirMultiversionPlatform,
    id: KirMultiversionTierId,
) -> Option<&'static TierSpec> {
    static X86_BASELINE: TierSpec = TierSpec {
        cpu: "x86-64",
        llvm_features: &[],
        required_features: &[],
        detector: "baseline",
        os_state: &[],
        fixture_cost_percent: 100,
    };
    static X86_V3: TierSpec = TierSpec {
        cpu: "x86-64-v3",
        llvm_features: &["+avx2"],
        required_features: X86_V3_FEATURES,
        detector: "x86-cpuid-xgetbv",
        os_state: &["xcr0.xmm-ymm"],
        fixture_cost_percent: 75,
    };
    static X86_V4: TierSpec = TierSpec {
        cpu: "x86-64-v4",
        llvm_features: &["+avx512f", "+avx512vl"],
        required_features: X86_V4_FEATURES,
        detector: "x86-cpuid-xgetbv",
        os_state: &["xcr0.opmask-zmm", "xcr0.xmm-ymm"],
        fixture_cost_percent: 60,
    };
    static ARM_BASELINE: TierSpec = TierSpec {
        cpu: "generic",
        llvm_features: &[],
        required_features: &[],
        detector: "baseline",
        os_state: &[],
        fixture_cost_percent: 100,
    };
    static ARM_SVE: TierSpec = TierSpec {
        cpu: "generic",
        llvm_features: &["+sve"],
        required_features: &["sve"],
        detector: "linux-auxv-hwcap",
        os_state: &["linux.sve-state"],
        fixture_cost_percent: 75,
    };
    static ARM_SVE2: TierSpec = TierSpec {
        cpu: "generic",
        llvm_features: &["+sve", "+sve2"],
        required_features: &["sve", "sve2"],
        detector: "linux-auxv-hwcap",
        os_state: &["linux.sve-state"],
        fixture_cost_percent: 60,
    };
    match (platform.architecture, platform.operating_system, id) {
        (KirTargetArchitecture::X86_64, _, Baseline) => Some(&X86_BASELINE),
        (KirTargetArchitecture::X86_64, _, X86_64V3) => Some(&X86_V3),
        (KirTargetArchitecture::X86_64, _, X86_64V4) => Some(&X86_V4),
        (KirTargetArchitecture::AArch64, _, Baseline) => Some(&ARM_BASELINE),
        (KirTargetArchitecture::AArch64, KirTargetOperatingSystem::Linux, AArch64Sve) => {
            Some(&ARM_SVE)
        }
        (KirTargetArchitecture::AArch64, KirTargetOperatingSystem::Linux, AArch64Sve2) => {
            Some(&ARM_SVE2)
        }
        _ => None,
    }
}

fn expected_tiers(platform: KirMultiversionPlatform) -> Vec<KirMultiversionTierId> {
    match (platform.architecture, platform.operating_system) {
        (KirTargetArchitecture::X86_64, _) => vec![Baseline, X86_64V3, X86_64V4],
        (KirTargetArchitecture::AArch64, KirTargetOperatingSystem::Linux) => {
            vec![Baseline, AArch64Sve, AArch64Sve2]
        }
        (KirTargetArchitecture::AArch64, _) => vec![Baseline],
    }
}

fn fixture_profile(
    consumer: KirConsumer,
    triple: &str,
    spec: &TierSpec,
) -> Result<KirTargetProfile, String> {
    let mut builder = KirTargetProfileBuilder::native(
        consumer,
        triple,
        64,
        true,
        KirNativeCpuPolicy::Multiversion,
        spec.cpu,
        strings(spec.llvm_features),
    )?;
    for key in KirTargetProfile::fixed_query_universe() {
        if key.operation == KirProfileOperation::MaskNot && key.lane != KIR_MASK_COST_LANE {
            builder.set_unavailable(key)?;
            continue;
        }
        let lanes = u32::from(key.lanes.max(1));
        let raw = if key.lanes > 1 {
            lanes.saturating_mul(4)
        } else {
            4
        };
        let cost = raw
            .saturating_mul(spec.fixture_cost_percent)
            .div_ceil(100)
            .max(1);
        builder.set_legal(
            key,
            KirLegalCost {
                cost,
                legalization_parts: 1,
                legalized_type: format!("fixture-{}", spec.cpu),
            },
        )?;
    }
    builder.set_maximum_interleave_factor(if spec.fixture_cost_percent < 100 {
        4
    } else {
        2
    });
    builder.set_producer_identity("LLVM 22.1.8 TCK_RecipThroughput", "ckc-llvm-bridge-abi-4");
    builder.build()
}

#[cfg(feature = "native-toolchain")]
pub(crate) fn materialized_tier(
    platform: KirMultiversionPlatform,
    id: KirMultiversionTierId,
    triple: String,
    data_layout: String,
    profile: KirTargetProfile,
    llvm_identity: String,
    bridge_identity: String,
) -> Result<KirMultiversionTargetTier, String> {
    let spec = tier_spec(platform, id)
        .ok_or_else(|| "multiversion tier is outside the closed schema-1 table".to_string())?;
    Ok(build_tier(
        id,
        triple,
        data_layout,
        profile,
        llvm_identity,
        bridge_identity,
        spec,
    ))
}

fn build_tier(
    id: KirMultiversionTierId,
    triple: String,
    data_layout: String,
    profile: KirTargetProfile,
    llvm_identity: String,
    bridge_identity: String,
    spec: &TierSpec,
) -> KirMultiversionTargetTier {
    let mut tier = KirMultiversionTargetTier {
        id,
        triple,
        cpu: spec.cpu.to_string(),
        llvm_features: strings(spec.llvm_features),
        required_features: strings(spec.required_features),
        data_layout,
        profile,
        llvm_identity,
        bridge_identity,
        predicate: KirRuntimeFeaturePredicate {
            detector: spec.detector.to_string(),
            hardware_features: strings(spec.required_features),
            os_state: strings(spec.os_state),
        },
        digest: [0; 32],
    };
    let digest = Sha256::digest(tier.canonical_bytes_without_digest());
    tier.digest.copy_from_slice(&digest);
    tier
}

fn require_native_consumer(consumer: KirConsumer) -> Result<(), String> {
    if matches!(
        consumer,
        KirConsumer::NativeLibrary | KirConsumer::NativeExecutable
    ) {
        Ok(())
    } else {
        Err("multiversion target set requires a Native consumer".to_string())
    }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

fn consumer_tag(consumer: KirConsumer) -> u8 {
    match consumer {
        KirConsumer::NativeLibrary => 1,
        KirConsumer::NativeExecutable => 2,
        _ => 0,
    }
}

fn put_strings(bytes: &mut Vec<u8>, values: &[String]) {
    put_u32(bytes, values.len());
    for value in values {
        put_string(bytes, value);
    }
}

fn put_bytes(bytes: &mut Vec<u8>, value: &[u8]) {
    put_u32(bytes, value.len());
    bytes.extend_from_slice(value);
}

fn put_string(bytes: &mut Vec<u8>, value: &str) {
    put_bytes(bytes, value.as_bytes());
}

fn put_u32(bytes: &mut Vec<u8>, value: usize) {
    bytes.extend_from_slice(&u32::try_from(value).unwrap_or(u32::MAX).to_be_bytes());
}

fn hex_digest(digest: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    let mut text = String::with_capacity(64);
    for byte in digest {
        write!(&mut text, "{byte:02x}").expect("writing to String cannot fail");
    }
    text
}
