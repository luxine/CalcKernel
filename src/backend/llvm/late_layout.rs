use std::collections::{BTreeMap, BTreeSet};

use crate::{
    CkProfileAnalysis, CkProfileKirPlan, CkProfileObservation, CkProfileSiteKind, KirConsumer,
};

use super::{
    NativeError, NativeOptimizationLevel, NativeStage, NativeTarget, OptimizedNativeModule, ffi,
};

/// One LLVM function and the non-entry block order requested at the late boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkLateProfileFunctionLayout {
    pub llvm_function: String,
    pub blocks: Vec<String>,
}

/// Closed profile-derived layout plan. It cannot represent instruction changes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CkLateProfileLayoutPlan {
    pub functions: Vec<CkLateProfileFunctionLayout>,
}

/// Target emission repairs authorized after an accepted permutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CkLateProfileRepair {
    FallthroughTerminator,
    BranchRelaxation,
    BranchFixup,
    AlignmentPadding,
}

/// Independently checked pre/post late-layout evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkLateProfileLayoutReport {
    pub accepted: bool,
    pub changed: bool,
    pub pre_layout_digest: [u8; 32],
    pub post_layout_digest: [u8; 32],
    pub pre_structural_digest: [u8; 32],
    pub post_structural_digest: [u8; 32],
    pub repairs: Vec<CkLateProfileRepair>,
    pub reason: String,
}

impl CkLateProfileLayoutPlan {
    fn encode(&self) -> Result<Vec<u8>, NativeError> {
        let mut output = String::from("CKLAYOUT1\n");
        let mut functions = BTreeSet::new();
        for function in &self.functions {
            validate_name(&function.llvm_function)?;
            if !functions.insert(function.llvm_function.as_str()) {
                return Err(layout_error("late layout function is duplicated"));
            }
            let mut blocks = BTreeSet::new();
            for block in &function.blocks {
                validate_name(block)?;
                if !blocks.insert(block.as_str()) {
                    return Err(layout_error("late layout block is duplicated"));
                }
                output.push_str("B\t");
                output.push_str(&function.llvm_function);
                output.push('\t');
                output.push_str(block);
                output.push('\n');
            }
        }
        Ok(output.into_bytes())
    }
}

/// Builds one deterministic hot-successor-first plan from a mapping-verified
/// KIR profile sidecar. Unknown blocks retain their ordinary relative order.
#[must_use]
pub fn build_late_profile_layout_plan(
    plan: &CkProfileKirPlan,
    analysis: &CkProfileAnalysis,
) -> CkLateProfileLayoutPlan {
    let observations = analysis
        .sites
        .iter()
        .map(|site| (site.descriptor.id, &site.observation))
        .collect::<BTreeMap<_, _>>();
    let mut function_digests = BTreeMap::new();
    for annotation in &plan.annotations {
        if let crate::CkProfileEvent::FunctionEntry { function, .. } = annotation.event {
            function_digests.insert(function, annotation.descriptor.function_digest);
        }
    }
    let mut output = Vec::new();
    for function in &plan.module.functions {
        let Some(function_digest) = function_digests.get(&function.id) else {
            continue;
        };
        let mut weights = BTreeMap::<u32, u64>::new();
        for annotation in &plan.annotations {
            if annotation.descriptor.function_digest != *function_digest {
                continue;
            }
            let CkProfileSiteKind::Edge { to_block, .. } = annotation.descriptor.kind else {
                continue;
            };
            if let Some(CkProfileObservation::Scalar(count)) = observations.get(&annotation.site_id)
            {
                let entry = weights.entry(to_block).or_default();
                *entry = entry.saturating_add(*count);
            }
        }
        if weights.is_empty() {
            continue;
        }
        let mut blocks = function
            .blocks
            .iter()
            .map(|block| (block.id.index(), weights.get(&block.id.index()).copied()))
            .collect::<Vec<_>>();
        blocks.sort_by(|left, right| {
            match (left.1, right.1) {
                (Some(left), Some(right)) => right.cmp(&left),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
            .then_with(|| left.0.cmp(&right.0))
        });
        output.push(CkLateProfileFunctionLayout {
            llvm_function: llvm_implementation_name(plan, function),
            blocks: blocks
                .into_iter()
                .map(|(block, _)| format!("kir.bb{block}"))
                .collect(),
        });
    }
    CkLateProfileLayoutPlan { functions: output }
}

/// Converts canonical tuning metadata into the same closed late-layout bridge
/// representation used by PGO, without consulting timings or LLVM heuristics.
pub fn build_tune_layout_plan(
    module: &crate::KirModule,
) -> Result<Option<CkLateProfileLayoutPlan>, NativeError> {
    let Some(layout) = &module.tune_layout else {
        return Ok(None);
    };
    let mut functions = Vec::with_capacity(layout.functions.len());
    for requested in &layout.functions {
        let function = module
            .functions
            .iter()
            .find(|function| function.id == requested.function)
            .ok_or_else(|| layout_error("tune layout function is missing"))?;
        let llvm_function = if module.config.consumer == KirConsumer::NativeExecutable
            && module
                .entry
                .as_ref()
                .is_some_and(|entry| entry.function_name == function.name)
        {
            "__ck_user_main".to_string()
        } else if function.exported {
            format!("__ck_impl_{}", function.name)
        } else {
            function.name.clone()
        };
        functions.push(CkLateProfileFunctionLayout {
            llvm_function,
            blocks: requested
                .blocks
                .iter()
                .map(|block| format!("kir.bb{}", block.index()))
                .collect(),
        });
    }
    Ok(Some(CkLateProfileLayoutPlan { functions }))
}

impl<'context> OptimizedNativeModule<'context> {
    /// Applies the closed layout plan after the complete ordinary O2 or O3 IR pipeline.
    ///
    /// # Errors
    ///
    /// Rejects non-O2 use, malformed/forged plans, missing KIR-to-LLVM names,
    /// bridge failures, and any independently detected structural mutation.
    pub fn apply_late_profile_layout(
        self,
        target: &NativeTarget,
        plan: &CkLateProfileLayoutPlan,
    ) -> Result<(Self, CkLateProfileLayoutReport), NativeError> {
        if !matches!(
            self.level,
            NativeOptimizationLevel::O2 | NativeOptimizationLevel::O3
        ) {
            return Err(layout_error("late layout requires O2 or O3"));
        }
        let encoded = plan.encode()?;
        let report =
            ffi::module_apply_late_layout(self.module.shared_handle(), target.handle(), &encoded)?;
        if report.pre_structural_digest != report.post_structural_digest {
            return Err(layout_error(
                "late profile layout changed structural content",
            ));
        }
        Ok((self, public_report(report)))
    }

    /// Captures the frozen ordinary boundary without applying a plan.
    pub fn late_layout_snapshot(
        &self,
        target: &NativeTarget,
    ) -> Result<CkLateProfileLayoutReport, NativeError> {
        let report = ffi::module_apply_late_layout(
            self.module.shared_handle(),
            target.handle(),
            b"CKLAYOUT1\n",
        )?;
        Ok(public_report(report))
    }
}

/// Test-only raw bridge seam for malformed-plan coverage.
#[doc(hidden)]
pub fn test_apply_late_layout_bytes(
    module: &OptimizedNativeModule<'_>,
    target: &NativeTarget,
    bytes: &[u8],
) -> Result<(), NativeError> {
    ffi::module_apply_late_layout(module.module.shared_handle(), target.handle(), bytes).map(|_| ())
}

fn public_report(report: ffi::BridgeLateLayoutReport) -> CkLateProfileLayoutReport {
    let mut repairs = Vec::new();
    if report.repair_mask & 1 != 0 {
        repairs.push(CkLateProfileRepair::FallthroughTerminator);
    }
    if report.repair_mask & 2 != 0 {
        repairs.push(CkLateProfileRepair::BranchRelaxation);
    }
    if report.repair_mask & 4 != 0 {
        repairs.push(CkLateProfileRepair::BranchFixup);
    }
    if report.repair_mask & 8 != 0 {
        repairs.push(CkLateProfileRepair::AlignmentPadding);
    }
    CkLateProfileLayoutReport {
        accepted: report.accepted,
        changed: report.changed,
        pre_layout_digest: report.pre_layout_digest,
        post_layout_digest: report.post_layout_digest,
        pre_structural_digest: report.pre_structural_digest,
        post_structural_digest: report.post_structural_digest,
        repairs,
        reason: report.reason,
    }
}

fn llvm_implementation_name(plan: &CkProfileKirPlan, function: &crate::KirFunction) -> String {
    if plan.module.config.consumer == KirConsumer::NativeExecutable
        && plan
            .module
            .entry
            .as_ref()
            .is_some_and(|entry| entry.function_name == function.name)
    {
        "__ck_user_main".to_string()
    } else {
        function.name.clone()
    }
}

fn validate_name(name: &str) -> Result<(), NativeError> {
    if name.is_empty()
        || name
            .bytes()
            .any(|byte| byte == 0 || byte == b'\n' || byte == b'\r' || byte == b'\t')
    {
        return Err(layout_error("late layout name is malformed"));
    }
    Ok(())
}

fn layout_error(message: impl Into<String>) -> NativeError {
    NativeError::new(NativeStage::Module, 5, message.into())
}
