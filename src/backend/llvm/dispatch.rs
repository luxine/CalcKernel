use std::collections::BTreeSet;
use std::sync::{
    OnceLock,
    atomic::{AtomicUsize, Ordering},
};

/// Closed normalized capability bitset consumed by CK's private dispatcher.
/// A value outside these five normalized states is treated as baseline.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct NativeCapabilitySet(u8);

impl NativeCapabilitySet {
    pub const BASELINE: Self = Self(0);
    pub const X86_V3: Self = Self(1 << 0);
    pub const X86_V4: Self = Self((1 << 0) | (1 << 1));
    pub const ARM_SVE: Self = Self(1 << 2);
    pub const ARM_SVE2: Self = Self((1 << 2) | (1 << 3));

    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    #[must_use]
    pub const fn is_normalized(self) -> bool {
        matches!(
            self,
            Self::BASELINE | Self::X86_V3 | Self::X86_V4 | Self::ARM_SVE | Self::ARM_SVE2
        )
    }

    const fn supports(self, required: Self) -> bool {
        self.is_normalized() && (self.0 & required.0) == required.0
    }
}

/// Raw x86 CPUID/XGETBV snapshot. It is deliberately a value object so every
/// failure and contradictory-state path can be mutation tested without running
/// an unsupported instruction on the test host.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct X86CpuidSnapshot {
    pub query_succeeded: bool,
    pub heterogeneous_uncertainty: bool,
    pub unknown_required_bits: bool,
    pub max_basic_leaf: u32,
    pub max_extended_leaf: u32,
    pub leaf1_ecx: u32,
    pub leaf7_ebx: u32,
    pub extended_leaf1_ecx: u32,
    pub xcr0_query_succeeded: bool,
    pub xcr0: u64,
}

impl X86CpuidSnapshot {
    pub const LEAF1_SSE3: u32 = 1 << 0;
    pub const LEAF1_SSSE3: u32 = 1 << 9;
    pub const LEAF1_FMA: u32 = 1 << 12;
    pub const LEAF1_SSE41: u32 = 1 << 19;
    pub const LEAF1_SSE42: u32 = 1 << 20;
    pub const LEAF1_MOVBE: u32 = 1 << 22;
    pub const LEAF1_POPCNT: u32 = 1 << 23;
    pub const LEAF1_OSXSAVE: u32 = 1 << 27;
    pub const LEAF1_AVX: u32 = 1 << 28;
    pub const LEAF1_F16C: u32 = 1 << 29;

    pub const LEAF7_BMI1: u32 = 1 << 3;
    pub const LEAF7_AVX2: u32 = 1 << 5;
    pub const LEAF7_BMI2: u32 = 1 << 8;
    pub const LEAF7_AVX512F: u32 = 1 << 16;
    pub const LEAF7_AVX512DQ: u32 = 1 << 17;
    pub const LEAF7_AVX512CD: u32 = 1 << 28;
    pub const LEAF7_AVX512BW: u32 = 1 << 30;
    pub const LEAF7_AVX512VL: u32 = 1 << 31;
    pub const EXTENDED_LZCNT: u32 = 1 << 5;

    pub const XCR0_XMM: u64 = 1 << 1;
    pub const XCR0_YMM: u64 = 1 << 2;
    pub const XCR0_OPMASK: u64 = 1 << 5;
    pub const XCR0_ZMM_HI256: u64 = 1 << 6;
    pub const XCR0_HI16_ZMM: u64 = 1 << 7;

    #[must_use]
    pub const fn complete_v4_fixture() -> Self {
        Self {
            query_succeeded: true,
            heterogeneous_uncertainty: false,
            unknown_required_bits: false,
            max_basic_leaf: 7,
            max_extended_leaf: 0x8000_0001,
            leaf1_ecx: Self::LEAF1_SSE3
                | Self::LEAF1_SSSE3
                | Self::LEAF1_FMA
                | Self::LEAF1_SSE41
                | Self::LEAF1_SSE42
                | Self::LEAF1_MOVBE
                | Self::LEAF1_POPCNT
                | Self::LEAF1_OSXSAVE
                | Self::LEAF1_AVX
                | Self::LEAF1_F16C,
            leaf7_ebx: Self::LEAF7_BMI1
                | Self::LEAF7_AVX2
                | Self::LEAF7_BMI2
                | Self::LEAF7_AVX512F
                | Self::LEAF7_AVX512DQ
                | Self::LEAF7_AVX512CD
                | Self::LEAF7_AVX512BW
                | Self::LEAF7_AVX512VL,
            extended_leaf1_ecx: Self::EXTENDED_LZCNT,
            xcr0_query_succeeded: true,
            xcr0: Self::XCR0_XMM
                | Self::XCR0_YMM
                | Self::XCR0_OPMASK
                | Self::XCR0_ZMM_HI256
                | Self::XCR0_HI16_ZMM,
        }
    }
}

/// Normalizes an x86 CPUID/XGETBV snapshot. Missing queries, unknown policy
/// input, or uncertain heterogeneous scheduling all fail closed to baseline.
#[must_use]
pub const fn detect_x86_cpuid(snapshot: X86CpuidSnapshot) -> NativeCapabilitySet {
    if !snapshot.query_succeeded
        || snapshot.heterogeneous_uncertainty
        || snapshot.unknown_required_bits
        || snapshot.max_basic_leaf < 7
        || snapshot.max_extended_leaf < 0x8000_0001
    {
        return NativeCapabilitySet::BASELINE;
    }
    let v3_leaf1 = X86CpuidSnapshot::LEAF1_SSE3
        | X86CpuidSnapshot::LEAF1_SSSE3
        | X86CpuidSnapshot::LEAF1_FMA
        | X86CpuidSnapshot::LEAF1_SSE41
        | X86CpuidSnapshot::LEAF1_SSE42
        | X86CpuidSnapshot::LEAF1_MOVBE
        | X86CpuidSnapshot::LEAF1_POPCNT
        | X86CpuidSnapshot::LEAF1_OSXSAVE
        | X86CpuidSnapshot::LEAF1_AVX
        | X86CpuidSnapshot::LEAF1_F16C;
    let v3_leaf7 =
        X86CpuidSnapshot::LEAF7_BMI1 | X86CpuidSnapshot::LEAF7_AVX2 | X86CpuidSnapshot::LEAF7_BMI2;
    let v3_xcr0 = X86CpuidSnapshot::XCR0_XMM | X86CpuidSnapshot::XCR0_YMM;
    let v3 = snapshot.leaf1_ecx & v3_leaf1 == v3_leaf1
        && snapshot.leaf7_ebx & v3_leaf7 == v3_leaf7
        && snapshot.extended_leaf1_ecx & X86CpuidSnapshot::EXTENDED_LZCNT != 0
        && snapshot.xcr0_query_succeeded
        && snapshot.xcr0 & v3_xcr0 == v3_xcr0;
    if !v3 {
        return NativeCapabilitySet::BASELINE;
    }
    let v4_leaf7 = X86CpuidSnapshot::LEAF7_AVX512F
        | X86CpuidSnapshot::LEAF7_AVX512DQ
        | X86CpuidSnapshot::LEAF7_AVX512CD
        | X86CpuidSnapshot::LEAF7_AVX512BW
        | X86CpuidSnapshot::LEAF7_AVX512VL;
    let v4_xcr0 = X86CpuidSnapshot::XCR0_OPMASK
        | X86CpuidSnapshot::XCR0_ZMM_HI256
        | X86CpuidSnapshot::XCR0_HI16_ZMM;
    if snapshot.leaf7_ebx & v4_leaf7 == v4_leaf7 && snapshot.xcr0 & v4_xcr0 == v4_xcr0 {
        NativeCapabilitySet::X86_V4
    } else {
        NativeCapabilitySet::X86_V3
    }
}

/// Initial Linux AArch64 auxiliary-vector state captured by private startup
/// support before user code can mutate process-visible state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Aarch64AuxvSnapshot {
    pub query_succeeded: bool,
    pub heterogeneous_uncertainty: bool,
    pub hwcap: u64,
    pub hwcap2: u64,
    pub sve_state_usable: bool,
    pub unknown_required_bits: bool,
}

impl Aarch64AuxvSnapshot {
    pub const HWCAP_SVE: u64 = 1 << 22;
    pub const HWCAP2_SVE2: u64 = 1 << 1;
}

/// Normalizes Linux AArch64 HWCAP/HWCAP2 data. SVE2 without SVE, unusable SVE
/// state, failed reads and future policy ambiguity all select baseline.
#[must_use]
pub const fn detect_aarch64_auxv(snapshot: Aarch64AuxvSnapshot) -> NativeCapabilitySet {
    if !snapshot.query_succeeded
        || snapshot.heterogeneous_uncertainty
        || snapshot.unknown_required_bits
        || !snapshot.sve_state_usable
    {
        return NativeCapabilitySet::BASELINE;
    }
    let sve = snapshot.hwcap & Aarch64AuxvSnapshot::HWCAP_SVE != 0;
    let sve2 = snapshot.hwcap2 & Aarch64AuxvSnapshot::HWCAP2_SVE2 != 0;
    match (sve, sve2) {
        (true, true) => NativeCapabilitySet::ARM_SVE2,
        (true, false) => NativeCapabilitySet::ARM_SVE,
        _ => NativeCapabilitySet::BASELINE,
    }
}

/// Executes the real host detector without ever executing an enhanced-tier
/// instruction. Unsupported host adapters deliberately return baseline.
#[must_use]
pub fn detect_host_cpu_capabilities() -> NativeCapabilitySet {
    #[cfg(target_arch = "x86_64")]
    {
        let v3 = std::arch::is_x86_feature_detected!("sse3")
            && std::arch::is_x86_feature_detected!("ssse3")
            && std::arch::is_x86_feature_detected!("sse4.1")
            && std::arch::is_x86_feature_detected!("sse4.2")
            && std::arch::is_x86_feature_detected!("popcnt")
            && std::arch::is_x86_feature_detected!("avx")
            && std::arch::is_x86_feature_detected!("avx2")
            && std::arch::is_x86_feature_detected!("bmi1")
            && std::arch::is_x86_feature_detected!("bmi2")
            && std::arch::is_x86_feature_detected!("f16c")
            && std::arch::is_x86_feature_detected!("fma")
            && std::arch::is_x86_feature_detected!("lzcnt")
            && std::arch::is_x86_feature_detected!("movbe");
        if !v3 {
            return NativeCapabilitySet::BASELINE;
        }
        let v4 = std::arch::is_x86_feature_detected!("avx512f")
            && std::arch::is_x86_feature_detected!("avx512bw")
            && std::arch::is_x86_feature_detected!("avx512cd")
            && std::arch::is_x86_feature_detected!("avx512dq")
            && std::arch::is_x86_feature_detected!("avx512vl");
        return if v4 {
            NativeCapabilitySet::X86_V4
        } else {
            NativeCapabilitySet::X86_V3
        };
    }
    #[cfg(all(target_arch = "aarch64", target_os = "linux"))]
    {
        let sve = std::arch::is_aarch64_feature_detected!("sve");
        let sve2 = std::arch::is_aarch64_feature_detected!("sve2");
        return match (sve, sve2) {
            (true, true) => NativeCapabilitySet::ARM_SVE2,
            (true, false) => NativeCapabilitySet::ARM_SVE,
            _ => NativeCapabilitySet::BASELINE,
        };
    }
    #[allow(unreachable_code)]
    NativeCapabilitySet::BASELINE
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NativeDispatchTier {
    Baseline,
    X86_64V3,
    X86_64V4,
    AArch64Sve,
    AArch64Sve2,
}

impl NativeDispatchTier {
    const fn required_capabilities(self) -> NativeCapabilitySet {
        match self {
            Self::Baseline => NativeCapabilitySet::BASELINE,
            Self::X86_64V3 => NativeCapabilitySet::X86_V3,
            Self::X86_64V4 => NativeCapabilitySet::X86_V4,
            Self::AArch64Sve => NativeCapabilitySet::ARM_SVE,
            Self::AArch64Sve2 => NativeCapabilitySet::ARM_SVE2,
        }
    }

    const fn stable_name(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::X86_64V3 => "x86_64_v3",
            Self::X86_64V4 => "x86_64_v4",
            Self::AArch64Sve => "aarch64_sve",
            Self::AArch64Sve2 => "aarch64_sve2",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeDispatchCandidate {
    pub tier: NativeDispatchTier,
    pub hidden_symbol: String,
    pub address: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeDispatchTable {
    target_set_digest: [u8; 32],
    public_symbol: String,
    baseline_symbol: String,
    support_symbol: String,
    candidates: Vec<NativeDispatchCandidate>,
}

impl NativeDispatchTable {
    pub fn new(
        target_set_digest: [u8; 32],
        public_symbol: impl Into<String>,
        mut candidates: Vec<NativeDispatchCandidate>,
    ) -> Result<Self, String> {
        let public_symbol = public_symbol.into();
        validate_symbol(&public_symbol)?;
        if candidates.is_empty()
            || candidates.last().map(|candidate| candidate.tier)
                != Some(NativeDispatchTier::Baseline)
            || candidates
                .iter()
                .filter(|candidate| candidate.tier == NativeDispatchTier::Baseline)
                .count()
                != 1
        {
            return Err("dispatch table must contain exactly one final baseline".to_string());
        }
        if candidates.iter().any(|candidate| candidate.address == 0) {
            return Err("dispatch table contains a null implementation pointer".to_string());
        }
        let mut tiers = BTreeSet::new();
        let mut addresses = BTreeSet::new();
        if candidates
            .iter()
            .any(|candidate| !tiers.insert(candidate.tier) || !addresses.insert(candidate.address))
        {
            return Err("dispatch table contains a duplicate tier or pointer".to_string());
        }
        let namespace = namespace(&target_set_digest);
        for candidate in &mut candidates {
            candidate.hidden_symbol = format!(
                "__ck_mv_{namespace}_{}_{}",
                public_symbol,
                candidate.tier.stable_name()
            );
        }
        let baseline_symbol = candidates
            .last()
            .expect("validated non-empty candidates")
            .hidden_symbol
            .clone();
        Ok(Self {
            target_set_digest,
            public_symbol: public_symbol.clone(),
            baseline_symbol,
            support_symbol: format!("__ck_mv_{namespace}_{public_symbol}_dispatch"),
            candidates,
        })
    }

    #[must_use]
    pub fn public_symbol(&self) -> &str {
        &self.public_symbol
    }

    #[must_use]
    pub fn baseline_symbol(&self) -> &str {
        &self.baseline_symbol
    }

    #[must_use]
    pub fn support_symbol(&self) -> &str {
        &self.support_symbol
    }

    #[must_use]
    pub const fn target_set_digest(&self) -> &[u8; 32] {
        &self.target_set_digest
    }

    pub fn select(
        &self,
        capabilities: NativeCapabilitySet,
    ) -> Result<&NativeDispatchCandidate, String> {
        let capabilities = if capabilities.is_normalized() {
            capabilities
        } else {
            NativeCapabilitySet::BASELINE
        };
        self.candidates
            .iter()
            .find(|candidate| capabilities.supports(candidate.tier.required_capabilities()))
            .ok_or_else(|| "verified dispatch table has no compatible baseline".to_string())
    }

    #[doc(hidden)]
    pub fn select_for_test(
        &self,
        capabilities: NativeCapabilitySet,
        forced_tier: NativeDispatchTier,
    ) -> Result<&NativeDispatchCandidate, String> {
        if !capabilities.is_normalized()
            || !capabilities.supports(forced_tier.required_capabilities())
        {
            return Err("test seam cannot force an unsupported CPU tier".to_string());
        }
        self.candidates
            .iter()
            .find(|candidate| candidate.tier == forced_tier)
            .ok_or_else(|| "test seam requested a tier absent from the verified table".to_string())
    }

    #[must_use]
    pub fn thunk_contract(&self, abi_signature: impl Into<String>) -> NativeDispatchThunkContract {
        NativeDispatchThunkContract {
            public_symbol: self.public_symbol.clone(),
            baseline_symbol: self.baseline_symbol.clone(),
            support_symbol: self.support_symbol.clone(),
            hidden_symbols: self
                .candidates
                .iter()
                .map(|candidate| candidate.hidden_symbol.clone())
                .chain(std::iter::once(self.support_symbol.clone()))
                .collect(),
            abi_signature: abi_signature.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeDispatchThunkContract {
    pub public_symbol: String,
    pub baseline_symbol: String,
    pub support_symbol: String,
    pub hidden_symbols: Vec<String>,
    pub abi_signature: String,
}

/// One process-local capability value shared by every per-root dispatch cell.
#[derive(Debug, Default)]
pub struct NativeCapabilityCache {
    value: OnceLock<NativeCapabilitySet>,
    initialization_count: AtomicUsize,
}

impl NativeCapabilityCache {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            value: OnceLock::new(),
            initialization_count: AtomicUsize::new(0),
        }
    }

    fn get_or_init(&self, query: impl FnOnce() -> NativeCapabilitySet) -> NativeCapabilitySet {
        *self.value.get_or_init(|| {
            self.initialization_count.fetch_add(1, Ordering::Relaxed);
            let capabilities = query();
            if capabilities.is_normalized() {
                capabilities
            } else {
                NativeCapabilitySet::BASELINE
            }
        })
    }

    #[must_use]
    pub fn initialization_count(&self) -> usize {
        self.initialization_count.load(Ordering::Relaxed)
    }
}

/// Per-public-root pointer slot. The fast path is one acquire load; the winner
/// publishes one verified non-null table pointer with release ordering.
#[derive(Debug, Default)]
pub struct NativeDispatchCell {
    pointer: AtomicUsize,
    resolve_count: AtomicUsize,
    slow_path_count: AtomicUsize,
}

impl NativeDispatchCell {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pointer: AtomicUsize::new(0),
            resolve_count: AtomicUsize::new(0),
            slow_path_count: AtomicUsize::new(0),
        }
    }

    pub fn resolve(
        &self,
        table: &NativeDispatchTable,
        capabilities: &NativeCapabilityCache,
        query: impl FnOnce() -> NativeCapabilitySet,
    ) -> Result<usize, String> {
        self.resolve_count.fetch_add(1, Ordering::Relaxed);
        let published = self.pointer.load(Ordering::Acquire);
        if published != 0 {
            return Ok(published);
        }
        let selected = table.select(capabilities.get_or_init(query))?.address;
        match self
            .pointer
            .compare_exchange(0, selected, Ordering::Release, Ordering::Acquire)
        {
            Ok(_) => {
                self.slow_path_count.fetch_add(1, Ordering::Relaxed);
                Ok(selected)
            }
            Err(winner) if winner != 0 => Ok(winner),
            Err(_) => Err("dispatch pointer publication produced null".to_string()),
        }
    }

    #[must_use]
    pub fn resolve_count(&self) -> usize {
        self.resolve_count.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn slow_path_count(&self) -> usize {
        self.slow_path_count.load(Ordering::Relaxed)
    }
}

fn validate_symbol(symbol: &str) -> Result<(), String> {
    if symbol.is_empty()
        || !symbol.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_alphanumeric() && (index != 0 || !byte.is_ascii_digit())
        })
    {
        return Err(format!("invalid public dispatch symbol `{symbol}`"));
    }
    Ok(())
}

fn namespace(digest: &[u8; 32]) -> String {
    let mut output = String::with_capacity(16);
    for byte in &digest[..8] {
        use std::fmt::Write;
        let _ = write!(output, "{byte:02x}");
    }
    output
}
