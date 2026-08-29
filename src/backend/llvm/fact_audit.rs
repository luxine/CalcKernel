use super::{
    error::{NativeError, NativeStage},
    ffi,
    module::NativeModule,
    verify::VerifiedNativeModule,
};
use crate::{FactId, ProofId};

const FACT_AUDIT_ERROR: i32 = 4;

/// Summary produced at the CK-owned pre-optimization fact-audit boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeStrengtheningKind {
    Range,
    Alignment,
    NoUnsignedWrap,
    NoSignedWrap,
    ReadOnly,
    WriteOnly,
    MemoryEffects,
    AliasScope,
    ParameterNoAlias,
    Assume,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeFactSource {
    Fact(FactId),
    Proof(ProofId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeFactProperty {
    pub kind: NativeStrengtheningKind,
    pub source: NativeFactSource,
    pub function: String,
    pub subject: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeFactAuditReport {
    pub property_count: usize,
    pub fact_sources: usize,
    pub proof_sources: usize,
    pub properties: Vec<NativeFactProperty>,
}

/// An LLVM-verified module whose CK-owned strengthenings have been audited.
#[derive(Debug)]
pub struct AuditedNativeModule<'context> {
    pub(super) module: NativeModule<'context>,
    report: NativeFactAuditReport,
}

impl<'context> VerifiedNativeModule<'context> {
    /// Audits every CK-owned strengthening before LLVM is allowed to optimize.
    pub fn audit(self) -> Result<AuditedNativeModule<'context>, NativeError> {
        let actual = ffi::module_fact_audit_counts(self.module.shared_handle())?;
        let expected = expected_counts(&self.module.fact_properties);
        if actual != expected {
            return Err(NativeError::new(
                NativeStage::Module,
                FACT_AUDIT_ERROR,
                format!(
                    "untracked CK-owned strengthening detected before LLVM optimization: expected {expected:?}, enumerated {actual:?}"
                ),
            ));
        }
        Ok(AuditedNativeModule {
            report: report(&self.module),
            module: self.module,
        })
    }
}

fn expected_counts(properties: &[NativeFactProperty]) -> ffi::CkcLlvmFactAuditCounts {
    let mut counts = ffi::CkcLlvmFactAuditCounts::default();
    for property in properties {
        match property.kind {
            NativeStrengtheningKind::Range => counts.range += 1,
            NativeStrengtheningKind::Alignment => counts.alignment += 1,
            NativeStrengtheningKind::NoUnsignedWrap => counts.no_unsigned_wrap += 1,
            NativeStrengtheningKind::NoSignedWrap => counts.no_signed_wrap += 1,
            NativeStrengtheningKind::ReadOnly => counts.readonly_count += 1,
            NativeStrengtheningKind::WriteOnly => counts.writeonly_count += 1,
            NativeStrengtheningKind::MemoryEffects => counts.memory_effects += 1,
            NativeStrengtheningKind::AliasScope => counts.alias_scope += 1,
            NativeStrengtheningKind::ParameterNoAlias => counts.parameter_noalias += 1,
            NativeStrengtheningKind::Assume => counts.assume_count += 1,
        }
    }
    counts
}

impl AuditedNativeModule<'_> {
    #[must_use]
    pub fn audit_report(&self) -> &NativeFactAuditReport {
        &self.report
    }

    pub fn to_ir_string(&self) -> Result<String, NativeError> {
        ffi::module_print(self.module.shared_handle())
    }
}

fn report(module: &NativeModule<'_>) -> NativeFactAuditReport {
    let fact_sources = module
        .fact_properties
        .iter()
        .filter(|property| matches!(property.source, NativeFactSource::Fact(_)))
        .count();
    let proof_sources = module
        .fact_properties
        .iter()
        .filter(|property| matches!(property.source, NativeFactSource::Proof(_)))
        .count();
    NativeFactAuditReport {
        property_count: module.fact_properties.len(),
        fact_sources,
        proof_sources,
        properties: module.fact_properties.clone(),
    }
}

/// Test-only mutation hook proving the audit rejects an unregistered property.
#[doc(hidden)]
pub fn test_inject_untracked_strengthening(
    module: &VerifiedNativeModule<'_>,
) -> Result<(), NativeError> {
    ffi::module_test_inject_untracked_strengthening(module.module.shared_handle())
}

/// Test-only mutation hook for an unregistered LLVM no-wrap flag.
#[doc(hidden)]
pub fn test_inject_untracked_flag(module: &VerifiedNativeModule<'_>) -> Result<(), NativeError> {
    ffi::module_test_inject_untracked_flag(module.module.shared_handle())
}
