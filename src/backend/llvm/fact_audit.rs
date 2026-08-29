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
        if ffi::module_has_untracked_strengthening(self.module.shared_handle())? {
            return Err(NativeError::new(
                NativeStage::Module,
                FACT_AUDIT_ERROR,
                "untracked CK-owned strengthening detected before LLVM optimization".to_string(),
            ));
        }
        Ok(AuditedNativeModule {
            report: report(&self.module),
            module: self.module,
        })
    }
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
