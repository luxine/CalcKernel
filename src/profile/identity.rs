use sha2::{Digest, Sha256};

use super::{CkProfileError, hex};

pub const CK_PROFILE_FORMAT_SCHEMA: u32 = 1;
pub const CK_PROFILE_CONTRACT_SCHEMA: u32 = 1;
pub const CK_PROFILE_INSPECTION_SCHEMA: u32 = 1;
pub const CK_PROFILE_MAX_SITES: u32 = 1_048_576;
pub const CK_PROFILE_MAX_SHARDS: u32 = 4_096;
pub const CK_PROFILE_HISTOGRAM_BUCKETS: u8 = 16;
pub const CK_PROFILE_MAX_CANDIDATES: u8 = 8;
pub const CK_PROFILE_MAX_BYTES: u64 = 512 * 1024 * 1024;

/// Compiler-owned identities that invalidate workload observations when changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkCompilerProfileIdentity {
    pub package_version: String,
    pub source_identity: [u8; 32],
    pub profile_runtime_identity: [u8; 32],
}

/// Canonical semantic and pre-instrumentation module identities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkModuleProfileIdentity {
    pub semantic_graph_digest: [u8; 32],
    pub pre_profile_kir_digest: [u8; 32],
    pub site_table_digest: [u8; 32],
}

/// Every compiler schema that can change profile interpretation or legality.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkProfileSchemaIdentity {
    pub language: u32,
    pub native_abi: u32,
    pub runtime_abi: u32,
    pub kir: u32,
    pub proof: u32,
    pub cost_model: u32,
    pub target_profile: u32,
    pub llvm_bridge: u32,
    pub cache: u32,
}

/// Byte order of the profiled Native target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CkProfileEndianness {
    Little,
    Big,
}

/// Object format of the profiled Native target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CkProfileObjectFormat {
    Elf,
    MachO,
    Coff,
}

/// Native target and target-set identity used to construct the site topology.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkProfileTargetIdentity {
    pub triple: String,
    pub pointer_width: u8,
    pub endianness: CkProfileEndianness,
    pub object_format: CkProfileObjectFormat,
    pub os_abi: String,
    pub target_set_digest: [u8; 32],
}

/// Semantic Native consumer topology, deliberately independent of packaging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CkProfileTopology {
    NativeExecutable,
    NativeLibrary,
}

/// Optimization family whose canonical profile topology is represented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CkProfileOptimizationFamily {
    O2,
    O3,
}

/// CPU policy bound to a workload profile identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CkProfileCpuPolicy {
    Baseline,
    Native,
    Multiversion,
}

/// Safety, consumer, optimization, and CPU modes that affect compatibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkProfileModes {
    pub overflow_checked: bool,
    pub bounds_checked: bool,
    pub strict_float: bool,
    pub sanitizer: bool,
    pub topology: CkProfileTopology,
    pub optimization_family: CkProfileOptimizationFamily,
    pub cpu_policy: CkProfileCpuPolicy,
}

/// Frozen confidence, profitability, growth, and resource constants for schema 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkProfileContract {
    pub format_schema: u32,
    pub contract_schema: u32,
    pub inspection_schema: u32,
    pub minimum_decision_observations: u64,
    pub branch_dominance_basis_points: u16,
    pub histogram_dominance_basis_points: u16,
    pub cold_basis_points: u16,
    pub hot_work_coverage_basis_points: u16,
    pub minimum_root_work_basis_points: u16,
    pub minimum_variant_benefit_basis_points: u16,
    pub minimum_absolute_cost_units: u32,
    pub maximum_enhanced_variants: u8,
    pub maximum_additional_kir_basis_points: u16,
    pub maximum_sites: u32,
    pub maximum_shards: u32,
    pub histogram_buckets: u8,
    pub maximum_candidate_constants: u8,
    pub maximum_profile_bytes: u64,
}

impl CkProfileContract {
    /// Returns the complete immutable CK 0.13 profile contract.
    #[must_use]
    pub const fn schema1() -> Self {
        Self {
            format_schema: CK_PROFILE_FORMAT_SCHEMA,
            contract_schema: CK_PROFILE_CONTRACT_SCHEMA,
            inspection_schema: CK_PROFILE_INSPECTION_SCHEMA,
            minimum_decision_observations: 128,
            branch_dominance_basis_points: 9_000,
            histogram_dominance_basis_points: 8_500,
            cold_basis_points: 100,
            hot_work_coverage_basis_points: 9_000,
            minimum_root_work_basis_points: 100,
            minimum_variant_benefit_basis_points: 1_000,
            minimum_absolute_cost_units: 2,
            maximum_enhanced_variants: 2,
            maximum_additional_kir_basis_points: 10_000,
            maximum_sites: CK_PROFILE_MAX_SITES,
            maximum_shards: CK_PROFILE_MAX_SHARDS,
            histogram_buckets: CK_PROFILE_HISTOGRAM_BUCKETS,
            maximum_candidate_constants: CK_PROFILE_MAX_CANDIDATES,
            maximum_profile_bytes: CK_PROFILE_MAX_BYTES,
        }
    }
}

/// Complete compatibility identity for one canonical CK workload profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkProfileIdentity {
    pub compiler: CkCompilerProfileIdentity,
    pub module: CkModuleProfileIdentity,
    pub schemas: CkProfileSchemaIdentity,
    pub target: CkProfileTargetIdentity,
    pub modes: CkProfileModes,
    pub contract: CkProfileContract,
}

impl CkProfileIdentity {
    /// Returns canonical schema-1 identity bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when a string or frozen contract field is not canonical.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, CkProfileError> {
        self.validate()?;
        let mut output = Vec::with_capacity(512);
        output.extend_from_slice(b"CKIDENT1");
        push_string(&mut output, &self.compiler.package_version)?;
        output.extend_from_slice(&self.compiler.source_identity);
        output.extend_from_slice(&self.compiler.profile_runtime_identity);
        output.extend_from_slice(&self.module.semantic_graph_digest);
        output.extend_from_slice(&self.module.pre_profile_kir_digest);
        output.extend_from_slice(&self.module.site_table_digest);
        for value in [
            self.schemas.language,
            self.schemas.native_abi,
            self.schemas.runtime_abi,
            self.schemas.kir,
            self.schemas.proof,
            self.schemas.cost_model,
            self.schemas.target_profile,
            self.schemas.llvm_bridge,
            self.schemas.cache,
        ] {
            output.extend_from_slice(&value.to_be_bytes());
        }
        push_string(&mut output, &self.target.triple)?;
        output.push(self.target.pointer_width);
        output.push(endianness_tag(self.target.endianness));
        output.push(object_format_tag(self.target.object_format));
        push_string(&mut output, &self.target.os_abi)?;
        output.extend_from_slice(&self.target.target_set_digest);
        output.push(u8::from(self.modes.overflow_checked));
        output.push(u8::from(self.modes.bounds_checked));
        output.push(u8::from(self.modes.strict_float));
        output.push(u8::from(self.modes.sanitizer));
        output.push(topology_tag(self.modes.topology));
        output.push(optimization_tag(self.modes.optimization_family));
        output.push(cpu_policy_tag(self.modes.cpu_policy));
        encode_contract(&mut output, &self.contract);
        Ok(output)
    }

    /// Returns the full SHA-256 of the canonical identity bytes.
    ///
    /// # Errors
    ///
    /// Returns the same validation errors as [`Self::canonical_bytes`].
    pub fn digest(&self) -> Result<[u8; 32], CkProfileError> {
        let canonical = self.canonical_bytes()?;
        let mut hasher = Sha256::new();
        hasher.update(b"CK-PROFILE-IDENTITY\0");
        hasher.update(canonical);
        Ok(hasher.finalize().into())
    }

    /// Returns the lowercase 64-character full identity digest.
    ///
    /// # Errors
    ///
    /// Returns the same validation errors as [`Self::canonical_bytes`].
    pub fn digest_hex(&self) -> Result<String, CkProfileError> {
        self.digest().map(|digest| hex(&digest))
    }

    pub(crate) fn validate(&self) -> Result<(), CkProfileError> {
        validate_string(&self.compiler.package_version, "compiler.packageVersion")?;
        validate_string(&self.target.triple, "target.triple")?;
        validate_string(&self.target.os_abi, "target.osAbi")?;
        if !matches!(self.target.pointer_width, 32 | 64) {
            return Err(CkProfileError::InvalidValue("target.pointerWidth"));
        }
        if self.modes.sanitizer {
            return Err(CkProfileError::InvalidValue("modes.sanitizer"));
        }
        if self.contract != CkProfileContract::schema1() {
            return Err(CkProfileError::InvalidValue("contract"));
        }
        Ok(())
    }

    pub(crate) fn first_mismatch(&self, observed: &Self) -> Option<(&'static str, String, String)> {
        macro_rules! mismatch {
            ($path:literal, $expected:expr, $actual:expr) => {
                if $expected != $actual {
                    return Some(($path, format!("{:?}", $expected), format!("{:?}", $actual)));
                }
            };
        }
        mismatch!(
            "compiler.packageVersion",
            self.compiler.package_version,
            observed.compiler.package_version
        );
        mismatch!(
            "compiler.sourceIdentity",
            self.compiler.source_identity,
            observed.compiler.source_identity
        );
        mismatch!(
            "compiler.profileRuntimeIdentity",
            self.compiler.profile_runtime_identity,
            observed.compiler.profile_runtime_identity
        );
        mismatch!("module", self.module, observed.module);
        mismatch!("schemas", self.schemas, observed.schemas);
        mismatch!("target", self.target, observed.target);
        mismatch!("modes", self.modes, observed.modes);
        mismatch!("contract", self.contract, observed.contract);
        None
    }
}

pub(crate) fn decode_identity(bytes: &[u8]) -> Result<CkProfileIdentity, CkProfileError> {
    let mut cursor = IdentityCursor::new(bytes);
    if cursor.read_exact(8)? != b"CKIDENT1" {
        return Err(CkProfileError::UnexpectedMagic);
    }
    let compiler = CkCompilerProfileIdentity {
        package_version: cursor.read_string()?,
        source_identity: cursor.read_array()?,
        profile_runtime_identity: cursor.read_array()?,
    };
    let module = CkModuleProfileIdentity {
        semantic_graph_digest: cursor.read_array()?,
        pre_profile_kir_digest: cursor.read_array()?,
        site_table_digest: cursor.read_array()?,
    };
    let schemas = CkProfileSchemaIdentity {
        language: cursor.read_u32()?,
        native_abi: cursor.read_u32()?,
        runtime_abi: cursor.read_u32()?,
        kir: cursor.read_u32()?,
        proof: cursor.read_u32()?,
        cost_model: cursor.read_u32()?,
        target_profile: cursor.read_u32()?,
        llvm_bridge: cursor.read_u32()?,
        cache: cursor.read_u32()?,
    };
    let target = CkProfileTargetIdentity {
        triple: cursor.read_string()?,
        pointer_width: cursor.read_u8()?,
        endianness: decode_endianness(cursor.read_u8()?)?,
        object_format: decode_object_format(cursor.read_u8()?)?,
        os_abi: cursor.read_string()?,
        target_set_digest: cursor.read_array()?,
    };
    let modes = CkProfileModes {
        overflow_checked: cursor.read_bool("modes.overflowChecked")?,
        bounds_checked: cursor.read_bool("modes.boundsChecked")?,
        strict_float: cursor.read_bool("modes.strictFloat")?,
        sanitizer: cursor.read_bool("modes.sanitizer")?,
        topology: decode_topology(cursor.read_u8()?)?,
        optimization_family: decode_optimization(cursor.read_u8()?)?,
        cpu_policy: decode_cpu_policy(cursor.read_u8()?)?,
    };
    let contract = decode_contract(&mut cursor)?;
    if !cursor.is_empty() {
        return Err(CkProfileError::NonCanonicalOrder("identity.trailing"));
    }
    let identity = CkProfileIdentity {
        compiler,
        module,
        schemas,
        target,
        modes,
        contract,
    };
    identity.validate()?;
    Ok(identity)
}

fn validate_string(value: &str, field: &'static str) -> Result<(), CkProfileError> {
    if value.is_empty() || value.len() > 4_096 || value.contains('\0') {
        return Err(CkProfileError::InvalidValue(field));
    }
    Ok(())
}

fn push_string(output: &mut Vec<u8>, value: &str) -> Result<(), CkProfileError> {
    validate_string(value, "identity.string")?;
    let length = u32::try_from(value.len()).map_err(|_| CkProfileError::LengthOverflow)?;
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(value.as_bytes());
    Ok(())
}

fn encode_contract(output: &mut Vec<u8>, contract: &CkProfileContract) {
    for value in [
        contract.format_schema,
        contract.contract_schema,
        contract.inspection_schema,
    ] {
        output.extend_from_slice(&value.to_be_bytes());
    }
    output.extend_from_slice(&contract.minimum_decision_observations.to_be_bytes());
    for value in [
        contract.branch_dominance_basis_points,
        contract.histogram_dominance_basis_points,
        contract.cold_basis_points,
        contract.hot_work_coverage_basis_points,
        contract.minimum_root_work_basis_points,
        contract.minimum_variant_benefit_basis_points,
    ] {
        output.extend_from_slice(&value.to_be_bytes());
    }
    output.extend_from_slice(&contract.minimum_absolute_cost_units.to_be_bytes());
    output.push(contract.maximum_enhanced_variants);
    output.extend_from_slice(&contract.maximum_additional_kir_basis_points.to_be_bytes());
    output.extend_from_slice(&contract.maximum_sites.to_be_bytes());
    output.extend_from_slice(&contract.maximum_shards.to_be_bytes());
    output.push(contract.histogram_buckets);
    output.push(contract.maximum_candidate_constants);
    output.extend_from_slice(&contract.maximum_profile_bytes.to_be_bytes());
}

fn decode_contract(cursor: &mut IdentityCursor<'_>) -> Result<CkProfileContract, CkProfileError> {
    Ok(CkProfileContract {
        format_schema: cursor.read_u32()?,
        contract_schema: cursor.read_u32()?,
        inspection_schema: cursor.read_u32()?,
        minimum_decision_observations: cursor.read_u64()?,
        branch_dominance_basis_points: cursor.read_u16()?,
        histogram_dominance_basis_points: cursor.read_u16()?,
        cold_basis_points: cursor.read_u16()?,
        hot_work_coverage_basis_points: cursor.read_u16()?,
        minimum_root_work_basis_points: cursor.read_u16()?,
        minimum_variant_benefit_basis_points: cursor.read_u16()?,
        minimum_absolute_cost_units: cursor.read_u32()?,
        maximum_enhanced_variants: cursor.read_u8()?,
        maximum_additional_kir_basis_points: cursor.read_u16()?,
        maximum_sites: cursor.read_u32()?,
        maximum_shards: cursor.read_u32()?,
        histogram_buckets: cursor.read_u8()?,
        maximum_candidate_constants: cursor.read_u8()?,
        maximum_profile_bytes: cursor.read_u64()?,
    })
}

const fn endianness_tag(value: CkProfileEndianness) -> u8 {
    match value {
        CkProfileEndianness::Little => 1,
        CkProfileEndianness::Big => 2,
    }
}

fn decode_endianness(value: u8) -> Result<CkProfileEndianness, CkProfileError> {
    match value {
        1 => Ok(CkProfileEndianness::Little),
        2 => Ok(CkProfileEndianness::Big),
        _ => Err(CkProfileError::InvalidValue("target.endianness")),
    }
}

const fn object_format_tag(value: CkProfileObjectFormat) -> u8 {
    match value {
        CkProfileObjectFormat::Elf => 1,
        CkProfileObjectFormat::MachO => 2,
        CkProfileObjectFormat::Coff => 3,
    }
}

fn decode_object_format(value: u8) -> Result<CkProfileObjectFormat, CkProfileError> {
    match value {
        1 => Ok(CkProfileObjectFormat::Elf),
        2 => Ok(CkProfileObjectFormat::MachO),
        3 => Ok(CkProfileObjectFormat::Coff),
        _ => Err(CkProfileError::InvalidValue("target.objectFormat")),
    }
}

const fn topology_tag(value: CkProfileTopology) -> u8 {
    match value {
        CkProfileTopology::NativeExecutable => 1,
        CkProfileTopology::NativeLibrary => 2,
    }
}

fn decode_topology(value: u8) -> Result<CkProfileTopology, CkProfileError> {
    match value {
        1 => Ok(CkProfileTopology::NativeExecutable),
        2 => Ok(CkProfileTopology::NativeLibrary),
        _ => Err(CkProfileError::InvalidValue("modes.topology")),
    }
}

const fn optimization_tag(value: CkProfileOptimizationFamily) -> u8 {
    match value {
        CkProfileOptimizationFamily::O2 => 2,
        CkProfileOptimizationFamily::O3 => 3,
    }
}

fn decode_optimization(value: u8) -> Result<CkProfileOptimizationFamily, CkProfileError> {
    match value {
        2 => Ok(CkProfileOptimizationFamily::O2),
        3 => Ok(CkProfileOptimizationFamily::O3),
        _ => Err(CkProfileError::InvalidValue("modes.optimizationFamily")),
    }
}

const fn cpu_policy_tag(value: CkProfileCpuPolicy) -> u8 {
    match value {
        CkProfileCpuPolicy::Baseline => 1,
        CkProfileCpuPolicy::Native => 2,
        CkProfileCpuPolicy::Multiversion => 3,
    }
}

fn decode_cpu_policy(value: u8) -> Result<CkProfileCpuPolicy, CkProfileError> {
    match value {
        1 => Ok(CkProfileCpuPolicy::Baseline),
        2 => Ok(CkProfileCpuPolicy::Native),
        3 => Ok(CkProfileCpuPolicy::Multiversion),
        _ => Err(CkProfileError::InvalidValue("modes.cpuPolicy")),
    }
}

struct IdentityCursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> IdentityCursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn read_exact(&mut self, length: usize) -> Result<&'a [u8], CkProfileError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(CkProfileError::LengthOverflow)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(CkProfileError::Truncated)?;
        self.offset = end;
        Ok(value)
    }

    fn read_array<const N: usize>(&mut self) -> Result<[u8; N], CkProfileError> {
        self.read_exact(N)?
            .try_into()
            .map_err(|_| CkProfileError::Truncated)
    }

    fn read_u8(&mut self) -> Result<u8, CkProfileError> {
        Ok(self.read_exact(1)?[0])
    }

    fn read_bool(&mut self, field: &'static str) -> Result<bool, CkProfileError> {
        match self.read_u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(CkProfileError::InvalidValue(field)),
        }
    }

    fn read_u16(&mut self) -> Result<u16, CkProfileError> {
        Ok(u16::from_be_bytes(self.read_array()?))
    }

    fn read_u32(&mut self) -> Result<u32, CkProfileError> {
        Ok(u32::from_be_bytes(self.read_array()?))
    }

    fn read_u64(&mut self) -> Result<u64, CkProfileError> {
        Ok(u64::from_be_bytes(self.read_array()?))
    }

    fn read_string(&mut self) -> Result<String, CkProfileError> {
        let length =
            usize::try_from(self.read_u32()?).map_err(|_| CkProfileError::LengthOverflow)?;
        if length > 4_096 {
            return Err(CkProfileError::ResourceLimit("identity string"));
        }
        let bytes = self.read_exact(length)?;
        let value = std::str::from_utf8(bytes).map_err(|_| CkProfileError::InvalidUtf8)?;
        validate_string(value, "identity.string")?;
        Ok(value.to_string())
    }

    fn is_empty(&self) -> bool {
        self.offset == self.bytes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_identity_should_round_trip_canonical_bytes() {
        let identity = fixture_identity();
        let bytes = identity.canonical_bytes().expect("canonical identity");

        assert_eq!(decode_identity(&bytes), Ok(identity));
    }

    #[test]
    fn profile_identity_should_use_complete_lowercase_sha256() {
        let digest = fixture_identity().digest_hex().expect("identity digest");

        assert_eq!(
            digest, "6cda6846a0afda13507aadefbb52f2fb74d485942d997ebbed0a98a5343b229e",
            "schema-1 identity digest changed"
        );
    }

    #[test]
    fn profile_identity_should_reject_mutated_contract_constants() {
        let mut identity = fixture_identity();
        identity.contract.minimum_decision_observations = 127;

        assert_eq!(
            identity.canonical_bytes(),
            Err(CkProfileError::InvalidValue("contract"))
        );
    }

    fn fixture_identity() -> CkProfileIdentity {
        CkProfileIdentity {
            compiler: CkCompilerProfileIdentity {
                package_version: "0.13.0-test".to_string(),
                source_identity: [1; 32],
                profile_runtime_identity: [2; 32],
            },
            module: CkModuleProfileIdentity {
                semantic_graph_digest: [3; 32],
                pre_profile_kir_digest: [4; 32],
                site_table_digest: [5; 32],
            },
            schemas: CkProfileSchemaIdentity {
                language: 1,
                native_abi: 1,
                runtime_abi: 2,
                kir: 3,
                proof: 3,
                cost_model: 3,
                target_profile: 1,
                llvm_bridge: 4,
                cache: 4,
            },
            target: CkProfileTargetIdentity {
                triple: "x86_64-unknown-linux-gnu".to_string(),
                pointer_width: 64,
                endianness: CkProfileEndianness::Little,
                object_format: CkProfileObjectFormat::Elf,
                os_abi: "linux-gnu".to_string(),
                target_set_digest: [6; 32],
            },
            modes: CkProfileModes {
                overflow_checked: false,
                bounds_checked: false,
                strict_float: true,
                sanitizer: false,
                topology: CkProfileTopology::NativeExecutable,
                optimization_family: CkProfileOptimizationFamily::O3,
                cpu_policy: CkProfileCpuPolicy::Baseline,
            },
            contract: CkProfileContract::schema1(),
        }
    }
}
