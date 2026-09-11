use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};

use sha2::{Digest, Sha256};

use super::kir_passes::{
    InlineTuningCandidate, check_tuning_inline_independently, discover_tuning_inline_candidates,
    materialize_tuning_inline, run_check_elimination, run_cleanup, run_dead_code_elimination,
    run_dead_store_elimination, run_gvn, run_induction_simplification,
    run_integer_constant_folding, run_licm, run_load_forwarding, run_memory_ssa_refine,
    run_sccp_range,
};
use super::{
    KirOptimizationLevel, KirVerifiedProgramState, SlpCandidate, SpecializationCandidate,
    TransactionCheckError, UnrollCandidate, VectorizationCandidate,
    check_tuned_slp_plan_independently, check_tuned_specialization_plan_independently,
    check_tuned_vectorization_trial_independently, check_unroll_structure_independently,
    discover_slp_candidates, discover_specialization_candidates,
    discover_tuning_vectorization_candidates, discover_unroll_candidates, prepare_slp_trial,
    prepare_specialization_trial, prepare_tuned_vectorization_trial, prepare_unroll_trial,
    run_kir_pass_pipeline,
};
use crate::{
    BlockId, FunctionId, InstructionId, KirTuneFunctionLayout, KirTuneLayoutPlan,
    MirPrimitiveTypeName, MirType, SpecializationFactValue, print_kir_module,
    tune::{TunePlanChoice, TuningPlan, plan_digest},
};

/// The seven finite CK-owned tuning alternative classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum TuneAlternativeClass {
    Inlining = 1,
    Specialization = 2,
    Unrolling = 3,
    LoopSimd = 4,
    Slp = 5,
    ShortSliceVersioning = 6,
    Layout = 7,
}

impl TuneAlternativeClass {
    /// Returns the fixed replay phase, which intentionally differs from the
    /// diversity priority discriminant.
    #[must_use]
    pub const fn replay_phase(self) -> u8 {
        match self {
            Self::Specialization => 1,
            Self::Inlining => 2,
            Self::ShortSliceVersioning => 3,
            Self::LoopSimd => 4,
            Self::Unrolling => 5,
            Self::Slp => 6,
            Self::Layout => 7,
        }
    }
}

/// One stable compiler decision site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuneSite {
    pub site_id: [u8; 32],
    pub class: TuneAlternativeClass,
    pub root_id: [u8; 32],
    pub root_anchor: TuneRootAnchor,
    pub function_symbol: String,
    pub canonical_rank: u32,
    pub pre_state_digest: [u8; 32],
}

/// Canonical source-independent anchor retained in a decision site.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TuneRootAnchor {
    pub function_symbol: String,
    pub kind: u8,
    pub preorder_ordinal: u32,
}

/// Closed wire payload for one currently materializable CK alternative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuneAlternativePayload {
    Inlining {
        callee_symbol: String,
        force_inline: bool,
    },
    Specialization {
        bindings: Vec<TuneSpecializationBinding>,
        guarded: bool,
    },
    Unrolling {
        factor: u32,
    },
    LoopSimd {
        vector_bits: u32,
        interleave: u32,
        break_even_iterations: u32,
    },
    Slp {
        pack_width: u32,
        operand_anchors: Vec<TuneRootAnchor>,
    },
    ShortSliceVersioning {
        maximum_length: u32,
        vector_bits: u32,
        interleave: u32,
    },
    Layout {
        scope: u8,
        root_order: Vec<[u8; 32]>,
    },
}

/// Canonical specialization value encoded without target-endian ambiguity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuneSpecializationBinding {
    pub argument_ordinal: u32,
    pub kind: u8,
    pub bits: u128,
}

/// A checked block-order intent attached to canonical KIR and consumed only at
/// the Native late-layout boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuneLayoutAction {
    pub scope: u8,
    pub function: FunctionId,
    pub blocks: Vec<BlockId>,
}

/// One exact site alternative inside a unit variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuneSiteAlternative {
    pub site_id: [u8; 32],
    pub alternative_id: [u8; 32],
    pub pre_state_digest: [u8; 32],
    pub post_state_digest: [u8; 32],
    pub payload: TuneAlternativePayload,
}

/// One finite nonbaseline variant of a tuning unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuneVariant {
    pub variant_id: [u8; 32],
    pub class: TuneAlternativeClass,
    pub parameter: u32,
    pub isolated_dynamic_estimate: u64,
    pub isolated_static_estimate: u64,
    pub isolated_kir_bytes: u64,
    pub site_alternatives: Vec<TuneSiteAlternative>,
    /// Exact CK-owned transformation reconstructed by the independent checker.
    pub action: TuneVariantAction,
    /// State after applying only this variant to a fresh pre-tune state.
    pub isolated_post_state_digest: [u8; 32],
}

/// Closed set of v0.14 transformations that already have CK legality checkers.
///
/// The wire format stores canonical anchors and payloads rather than trusting
/// these in-memory candidates.  A source-aware replay always re-enumerates this
/// value from the immutable pre-tune state before looking up a variant id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuneVariantAction {
    Inlining {
        candidate: InlineTuningCandidate,
        force_inline: bool,
    },
    Specialization(SpecializationCandidate),
    Unrolling(UnrollCandidate),
    LoopSimd(VectorizationCandidate),
    Slp(SlpCandidate),
    ShortSliceVersioning {
        candidate: VectorizationCandidate,
        maximum_length: u32,
    },
    Layout(TuneLayoutAction),
}

/// One deterministic cluster of overlapping tuning sites.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuneUnit {
    pub unit_id: [u8; 32],
    pub class: TuneAlternativeClass,
    pub site_ids: Vec<[u8; 32]>,
    pub variants: Vec<TuneVariant>,
}

/// Complete bounded candidate space for one immutable pre-tune KIR state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuningSpace {
    pub pre_tune_kir_digest: [u8; 32],
    pub pre_state_digest: [u8; 32],
    pub sites: Vec<TuneSite>,
    pub units: Vec<TuneUnit>,
    pub digest: [u8; 32],
}

/// Source-backed authority for one immutable tuning space. Borrowed proposals
/// are independently reconstructed once; freshly enumerated spaces retain the
/// same authority without repeating discovery for every search expansion.
#[derive(Debug)]
pub struct CheckedTuningSpace<'a> {
    state: &'a KirVerifiedProgramState,
    space: Cow<'a, TuningSpace>,
}

type TuneSelection = ([u8; 32], [u8; 32]);

/// Search-local snapshots never cross the immutable source/space boundary.
/// A prefix is retained before the last-choice suffix: extending a plan must
/// not reuse the different post-state of its previously final choice.
/// Shared results stay immutable; each rewrite constructs and independently
/// checks an owned trial instead of mutating a retained snapshot.
#[derive(Debug, Clone)]
struct ReplayPrefix {
    state: Rc<KirVerifiedProgramState>,
    choices: Vec<TunePlanChoice>,
    previous_key: Option<(u8, [u8; 32])>,
    contains_selected_rewrite: bool,
    entered_late_o3: bool,
}

#[derive(Debug, Default)]
struct ReplayPrefixCache {
    prefixes: BTreeMap<Vec<TuneSelection>, ReplayPrefix>,
    identities: BTreeMap<String, (crate::KirModule, [u8; 32], usize)>,
    analyses: BTreeMap<
        (ReplayPhase, String, String),
        (KirVerifiedProgramState, Rc<KirVerifiedProgramState>),
    >,
    snapshots: usize,
    canonical_bytes: usize,
}

impl ReplayPrefixCache {
    fn admit(&mut self, snapshots: usize, kir_bytes: usize) -> bool {
        // Admission limits bound retained work, never the normative search.
        // Once full, every remaining expansion follows ordinary replay.
        const MAX_SNAPSHOTS: usize = 256;
        const MAX_CANONICAL_BYTES: usize = 4 * 1024 * 1024;
        if snapshots <= MAX_SNAPSHOTS.saturating_sub(self.snapshots)
            && kir_bytes <= MAX_CANONICAL_BYTES.saturating_sub(self.canonical_bytes)
        {
            self.snapshots += snapshots;
            self.canonical_bytes += kir_bytes;
            true
        } else {
            false
        }
    }

    fn retain(&mut self, selections: &[TuneSelection], prefix: ReplayPrefix, kir_bytes: usize) {
        if !self.prefixes.contains_key(selections) && self.admit(1, kir_bytes) {
            self.prefixes.insert(selections.to_vec(), prefix);
        }
    }

    fn identity(
        &mut self,
        state: &KirVerifiedProgramState,
    ) -> Result<([u8; 32], usize), TuningPlanError> {
        let key = state.kir_digest();
        if let Some((module, digest, bytes)) = self.identities.get(&key)
            && module == state.module()
        {
            return Ok((*digest, *bytes));
        }
        let (digest, bytes) = tuning_kir_state_identity(state)?;
        if !self.identities.contains_key(&key) && self.admit(1, bytes) {
            self.identities
                .insert(key, (state.module().clone(), digest, bytes));
        }
        Ok((digest, bytes))
    }

    fn analyze(
        &mut self,
        phase: ReplayPhase,
        state: &KirVerifiedProgramState,
    ) -> Result<Rc<KirVerifiedProgramState>, TuningPlanError> {
        let key = (
            phase,
            state.kir_digest(),
            state.verification_cache().evidence_digest.clone(),
        );
        // Digests select a bucket, but never authorize reuse: compare the
        // complete state, including evidence, allocators and growth accounting.
        if let Some((input, output)) = self.analyses.get(&key)
            && input == state
        {
            return Ok(Rc::clone(output));
        }
        let output = Rc::new(phase.run(state)?);
        let (_, input_bytes) = self.identity(state)?;
        let (_, output_bytes) = self.identity(&output)?;
        if !self.analyses.contains_key(&key) && self.admit(2, input_bytes + output_bytes) {
            self.analyses
                .insert(key, (state.clone(), Rc::clone(&output)));
        }
        Ok(output)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ReplayPhase {
    Ordinary,
    Late,
    Finish,
}

impl ReplayPhase {
    fn run(
        self,
        state: &KirVerifiedProgramState,
    ) -> Result<KirVerifiedProgramState, TuningPlanError> {
        match self {
            Self::Ordinary => finish_ordinary_o3(state),
            Self::Late => advance_to_tunable_late_o3(state),
            Self::Finish => finish_selected_o3(state),
        }
    }
}

fn replay_identity(
    state: &KirVerifiedProgramState,
    cache: &mut Option<&mut ReplayPrefixCache>,
) -> Result<([u8; 32], usize), TuningPlanError> {
    match cache {
        Some(cache) => cache.identity(state),
        None => tuning_kir_state_identity(state),
    }
}

fn replay_phase(
    phase: ReplayPhase,
    state: &KirVerifiedProgramState,
    cache: &mut Option<&mut ReplayPrefixCache>,
) -> Result<Rc<KirVerifiedProgramState>, TuningPlanError> {
    match cache {
        Some(cache) => cache.analyze(phase, state),
        None => phase.run(state).map(Rc::new),
    }
}

pub(crate) struct TuningSearchReplay<'a> {
    state: &'a KirVerifiedProgramState,
    space: &'a TuningSpace,
    prefixes: ReplayPrefixCache,
}

impl TuningSearchReplay<'_> {
    pub(crate) fn derive(
        &mut self,
        selections: &[TuneSelection],
    ) -> Result<(TuningPlan, Rc<KirVerifiedProgramState>), TuningPlanError> {
        replay_selections_with_cache(self.state, self.space, selections, Some(&mut self.prefixes))
    }
}

/// One independently checked plan and its exact replayed state. The plan is
/// immutably borrowed so its identity cannot change before trial construction.
#[derive(Debug)]
pub struct CheckedTuningPlan<'a> {
    plan: &'a TuningPlan,
    state: KirVerifiedProgramState,
}

impl<'a> CheckedTuningSpace<'a> {
    /// Discovers and validates the complete source-backed finite space.
    pub fn enumerate(state: &'a KirVerifiedProgramState) -> Result<Self, TuningPlanError> {
        Ok(Self {
            state,
            space: Cow::Owned(enumerate_tuning_space(state)?),
        })
    }

    /// Independently rejects every field of a stale or forged raw space.
    pub fn check(
        state: &'a KirVerifiedProgramState,
        space: &'a TuningSpace,
    ) -> Result<Self, TuningPlanError> {
        if enumerate_tuning_space(state)? != *space {
            return Err(TuningPlanError::PreStateMismatch);
        }
        Ok(Self {
            state,
            space: Cow::Borrowed(space),
        })
    }

    #[must_use]
    pub fn space(&self) -> &TuningSpace {
        &self.space
    }

    /// Checks canonical choices, their identity, legality, and all pre/post
    /// states, retaining the independent replay instead of materializing twice.
    pub fn apply<'plan>(
        &self,
        plan: &'plan TuningPlan,
    ) -> Result<CheckedTuningPlan<'plan>, TuningPlanError> {
        Ok(CheckedTuningPlan {
            plan,
            state: check_plan_replay(self.state, self.space(), plan)?,
        })
    }

    pub(crate) fn derive(
        &self,
        selections: &[([u8; 32], [u8; 32])],
    ) -> Result<(TuningPlan, KirVerifiedProgramState), TuningPlanError> {
        replay_selections(self.state, self.space(), selections)
    }

    pub(crate) fn search_replay(&self) -> TuningSearchReplay<'_> {
        TuningSearchReplay {
            state: self.state,
            space: self.space(),
            prefixes: ReplayPrefixCache::default(),
        }
    }
}

impl CheckedTuningPlan<'_> {
    #[must_use]
    pub const fn state(&self) -> &KirVerifiedProgramState {
        &self.state
    }

    pub(crate) const fn plan(&self) -> &TuningPlan {
        self.plan
    }
}

/// Source-aware facts for the one Floyd predicated-update choice accepted by
/// the v0.14 performance gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PredicatedUpdateAttestation {
    pub function: String,
    pub header: BlockId,
    pub compare: InstructionId,
    pub load: InstructionId,
    pub store: InstructionId,
    pub unit_id: [u8; 32],
    pub variant_id: [u8; 32],
    pub alternative_id: [u8; 32],
    pub vector_bits: u32,
    pub interleave: u32,
    pub minimum: u32,
    pub pre_state_digest: [u8; 32],
    pub post_state_digest: [u8; 32],
}

impl TuningSpace {
    /// Builds a one-choice plan for a bounded space member.
    pub fn plan_for_variant(
        &self,
        state: &KirVerifiedProgramState,
        unit: usize,
        variant: usize,
    ) -> Result<Option<TuningPlan>, TuningPlanError> {
        let Some(unit) = self.units.get(unit) else {
            return Ok(None);
        };
        let Some(variant) = unit.variants.get(variant) else {
            return Ok(None);
        };
        derive_tuning_plan(state, self, &[(unit.unit_id, variant.variant_id)])
            .map(|(plan, _)| Some(plan))
    }
}

/// Closed deterministic tuning-space failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TuningPlanError {
    #[error("tuning space exceeds schema-1 bounds")]
    ResourceLimit,
    #[error("tuning space does not match immutable pre-state")]
    PreStateMismatch,
    #[error("unknown or forged tuning unit/variant")]
    UnknownChoice,
    #[error("tuning choices are duplicate or out of replay order")]
    NonCanonicalOrder,
    #[error("tuning plan digest mismatch")]
    DigestMismatch,
    #[error("tuning alternative failed independent legality: {0}")]
    IllegalAlternative(String),
    #[error("tuning alternative exceeded the frozen structural growth bound: {0}")]
    GrowthRejected(String),
    #[error("tuning replay failed after legality: {0}")]
    ReplayFailure(String),
}

/// Enumerates a stable finite set of CK-owned choices from verified KIR.
///
/// # Errors
///
/// Fails if stable identifiers or schema bounds cannot be represented.
pub fn enumerate_tuning_space(
    state: &KirVerifiedProgramState,
) -> Result<TuningSpace, TuningPlanError> {
    #[cfg(test)]
    tests::SPACE_ENUMERATIONS.with(|count| count.set(count.get() + 1));
    let pre_tune_kir_digest = tuning_pre_kir_digest(state)?;
    let pre_state_digest = tuning_kir_state_digest(state)?;
    let function_names = state
        .module()
        .functions
        .iter()
        .map(|function| (function.id, function.name.clone()))
        .collect::<BTreeMap<_, _>>();
    let late_state = advance_to_tunable_late_o3(state)?;
    let late_function_names = late_state
        .module()
        .functions
        .iter()
        .map(|function| (function.id, function.name.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut seeds = Vec::new();

    for candidate in discover_tuning_inline_candidates(
        state.module(),
        state.contract_facts(),
        state.eliminated_guards(),
    ) {
        let symbol = function_names
            .get(&candidate.caller)
            .ok_or(TuningPlanError::PreStateMismatch)?
            .clone();
        let callee = function_names
            .get(&candidate.callee)
            .ok_or(TuningPlanError::PreStateMismatch)?;
        let root_anchor = TuneRootAnchor {
            function_symbol: symbol.clone(),
            kind: 6,
            preorder_ordinal: instruction_kind_ordinal(
                state,
                candidate.caller,
                candidate.call,
                true,
            )?,
        };
        for force_inline in [true, false] {
            let action = if force_inline {
                "force-inline"
            } else {
                "keep-out-of-line"
            };
            seeds.push(VariantSeed::new(
                TuneAlternativeClass::Inlining,
                symbol.clone(),
                root_anchor.clone(),
                if force_inline { 1 } else { 2 },
                format!("callee={callee};action={action}"),
                TuneVariantAction::Inlining {
                    candidate,
                    force_inline,
                },
            ));
        }
    }

    for candidate in
        discover_specialization_candidates(state.module(), state.contract_facts()).candidates
    {
        let symbol = function_names
            .get(&candidate.caller)
            .ok_or(TuningPlanError::PreStateMismatch)?
            .clone();
        seeds.push(VariantSeed::new(
            TuneAlternativeClass::Specialization,
            symbol,
            TuneRootAnchor {
                function_symbol: function_names[&candidate.caller].clone(),
                kind: 6,
                preorder_ordinal: instruction_kind_ordinal(
                    state,
                    candidate.caller,
                    candidate.call,
                    true,
                )?,
            },
            1,
            candidate.key.stable_text(),
            TuneVariantAction::Specialization(candidate),
        ));
    }
    for candidate in discover_tuning_vectorization_candidates(&late_state).candidates {
        let symbol = late_function_names
            .get(&candidate.function)
            .ok_or(TuningPlanError::PreStateMismatch)?
            .clone();
        let parameter = u32::from(candidate.vf) * u32::from(candidate.uf);
        seeds.push(VariantSeed::new(
            TuneAlternativeClass::LoopSimd,
            symbol,
            TuneRootAnchor {
                function_symbol: late_function_names[&candidate.function].clone(),
                kind: 3,
                preorder_ordinal: candidate.loop_id.index(),
            },
            parameter,
            vector_candidate_stable_payload(&candidate),
            TuneVariantAction::LoopSimd(candidate),
        ));
        let mut short_candidate = match seeds.last().map(|seed| &seed.action) {
            Some(TuneVariantAction::LoopSimd(candidate)) => candidate.clone(),
            _ => unreachable!("the just-added seed is Loop SIMD"),
        };
        let chunk_width = u32::from(short_candidate.vf) * u32::from(short_candidate.uf);
        if let Some(tuned_minimum) = short_candidate.minimum_trip.checked_mul(2)
            && tuned_minimum <= chunk_width.saturating_mul(1024)
            && let Some(maximum_length) = tuned_minimum.checked_sub(1)
        {
            short_candidate.minimum_trip = tuned_minimum;
            seeds.push(VariantSeed::new(
                TuneAlternativeClass::ShortSliceVersioning,
                late_function_names[&short_candidate.function].clone(),
                TuneRootAnchor {
                    function_symbol: late_function_names[&short_candidate.function].clone(),
                    kind: 3,
                    preorder_ordinal: short_candidate.loop_id.index(),
                },
                maximum_length,
                format!(
                    "{};short-slice-maximum={maximum_length}",
                    vector_candidate_stable_payload(&short_candidate)
                ),
                TuneVariantAction::ShortSliceVersioning {
                    candidate: short_candidate,
                    maximum_length,
                },
            ));
        }
    }
    for function in &late_state.module().functions {
        let loops = super::analyze_canonical_loops_for_discovery(function);
        for candidate in discover_unroll_candidates(function, &loops.loops).candidates {
            seeds.push(VariantSeed::new(
                TuneAlternativeClass::Unrolling,
                function.name.clone(),
                TuneRootAnchor {
                    function_symbol: function.name.clone(),
                    kind: 3,
                    preorder_ordinal: candidate.loop_id.index(),
                },
                if candidate.full {
                    candidate.trip_count
                } else {
                    u32::from(candidate.factor)
                },
                candidate.key.stable_text(),
                TuneVariantAction::Unrolling(candidate),
            ));
        }
        for candidate in
            discover_slp_candidates(function, &late_state.proofs().instruction_dependencies())
                .candidates
        {
            seeds.push(VariantSeed::new(
                TuneAlternativeClass::Slp,
                function.name.clone(),
                TuneRootAnchor {
                    function_symbol: function.name.clone(),
                    kind: 5,
                    preorder_ordinal: instruction_kind_ordinal(
                        &late_state,
                        candidate.function,
                        candidate.root,
                        false,
                    )?,
                },
                u32::from(candidate.lanes),
                candidate.key.stable_text(),
                TuneVariantAction::Slp(candidate),
            ));
        }
        if function.blocks.len() >= 3 {
            let mut blocks = function
                .blocks
                .iter()
                .map(|block| block.id)
                .collect::<Vec<_>>();
            blocks.reverse();
            let mut root_order = Vec::with_capacity(blocks.len());
            for block in &blocks {
                let ordinal = function
                    .blocks
                    .iter()
                    .position(|candidate| candidate.id == *block)
                    .and_then(|index| u32::try_from(index).ok())
                    .ok_or(TuningPlanError::PreStateMismatch)?;
                root_order.push(derive_root_id(
                    pre_tune_kir_digest,
                    &TuneRootAnchor {
                        function_symbol: function.name.clone(),
                        kind: 4,
                        preorder_ordinal: ordinal,
                    },
                ));
            }
            let function_ordinal = state
                .module()
                .functions
                .iter()
                .position(|candidate| candidate.id == function.id)
                .and_then(|index| u32::try_from(index).ok())
                .ok_or(TuningPlanError::PreStateMismatch)?;
            seeds.push(VariantSeed::new(
                TuneAlternativeClass::Layout,
                function.name.clone(),
                TuneRootAnchor {
                    function_symbol: function.name.clone(),
                    kind: 2,
                    preorder_ordinal: function_ordinal,
                },
                u32::try_from(blocks.len()).map_err(|_| TuningPlanError::ResourceLimit)?,
                format!("scope=block;roots={root_order:?}"),
                TuneVariantAction::Layout(TuneLayoutAction {
                    scope: 1,
                    function: function.id,
                    blocks,
                }),
            ));
        }
    }
    seeds.sort_by(|left, right| {
        (
            left.class.replay_phase(),
            left.function_symbol.as_bytes(),
            &left.root_anchor,
            left.payload.as_bytes(),
        )
            .cmp(&(
                right.class.replay_phase(),
                right.function_symbol.as_bytes(),
                &right.root_anchor,
                right.payload.as_bytes(),
            ))
    });

    let mut seed_groups = BTreeMap::<(u8, String, TuneRootAnchor), Vec<VariantSeed>>::new();
    for seed in seeds {
        seed_groups
            .entry((
                seed.class.replay_phase(),
                seed.function_symbol.clone(),
                seed.root_anchor.clone(),
            ))
            .or_default()
            .push(seed);
    }
    let mut sites = Vec::new();
    let mut units = Vec::new();
    for (rank, ((_phase, function_symbol, root_anchor), group)) in
        seed_groups.into_iter().enumerate()
    {
        if sites.len() == 4_096 {
            break;
        }
        let class = group
            .first()
            .ok_or(TuningPlanError::PreStateMismatch)?
            .class;
        let rank = u32::try_from(rank).map_err(|_| TuningPlanError::ResourceLimit)?;
        let root_id = derive_root_id(pre_tune_kir_digest, &root_anchor);
        let site_id = derive_site_id(root_id, class, rank, pre_state_digest);
        let unit_id = derive_unit_id(&[site_id], pre_state_digest);
        let action_state = if matches!(
            class,
            TuneAlternativeClass::ShortSliceVersioning
                | TuneAlternativeClass::LoopSimd
                | TuneAlternativeClass::Unrolling
                | TuneAlternativeClass::Slp
        ) {
            &late_state
        } else {
            state
        };
        let mut variants = Vec::new();
        for seed in group.into_iter().take(4) {
            let Some(payload) = payload_for_action(action_state, &seed.action)? else {
                continue;
            };
            let isolated = match apply_variant_action(action_state, &seed.action) {
                Ok(isolated) => isolated,
                Err(
                    TuningPlanError::IllegalAlternative(_) | TuningPlanError::GrowthRejected(_),
                ) => continue,
                Err(error) => return Err(error),
            };
            let isolated_post_state_digest = tuning_kir_state_digest(&isolated)?;
            let isolated_kir_bytes = u64::try_from(print_kir_module(isolated.module()).len())
                .map_err(|_| TuningPlanError::ResourceLimit)?;
            let instruction_count = isolated
                .module()
                .functions
                .iter()
                .flat_map(|function| &function.blocks)
                .map(|block| block.instructions.len())
                .try_fold(0_u64, |total, count| {
                    total
                        .checked_add(
                            u64::try_from(count).map_err(|_| TuningPlanError::ResourceLimit)?,
                        )
                        .ok_or(TuningPlanError::ResourceLimit)
                })?;
            let alternative_id =
                derive_alternative_id(site_id, &payload, isolated_post_state_digest);
            let site_alternatives = vec![TuneSiteAlternative {
                site_id,
                alternative_id,
                pre_state_digest,
                post_state_digest: isolated_post_state_digest,
                payload,
            }];
            let variant_id = derive_unit_variant_id(
                unit_id,
                class,
                &site_alternatives,
                instruction_count,
                instruction_count,
                isolated_kir_bytes,
                isolated_post_state_digest,
            );
            variants.push(TuneVariant {
                variant_id,
                class,
                parameter: seed.parameter,
                isolated_dynamic_estimate: instruction_count,
                isolated_static_estimate: instruction_count,
                isolated_kir_bytes,
                site_alternatives,
                action: seed.action,
                isolated_post_state_digest,
            });
        }
        if variants.is_empty() {
            continue;
        }
        variants.sort_by_key(|variant| variant.variant_id);
        sites.push(TuneSite {
            site_id,
            class,
            root_id,
            root_anchor,
            function_symbol,
            canonical_rank: rank,
            pre_state_digest,
        });
        units.push(TuneUnit {
            unit_id,
            class,
            site_ids: vec![site_id],
            variants,
        });
        if units.len() == 64 {
            break;
        }
    }
    units.sort_by_key(|unit| (unit.class.replay_phase(), unit.unit_id));
    sites.sort_by_key(|site| site.site_id);
    let digest = derive_space_digest(&sites, &units, pre_state_digest);
    Ok(TuningSpace {
        pre_tune_kir_digest,
        pre_state_digest,
        sites,
        units,
        digest,
    })
}

/// Independently checks a plan against the immutable state and finite space.
///
/// # Errors
///
/// Rejects stale state, unknown choices, duplicates, order changes, and digest
/// forgery without invoking the proposer.
pub fn check_tuning_plan(
    state: &KirVerifiedProgramState,
    space: &TuningSpace,
    plan: &TuningPlan,
) -> Result<(), TuningPlanError> {
    CheckedTuningSpace::check(state, space)?
        .apply(plan)
        .map(|_| ())
}

fn check_plan_replay(
    state: &KirVerifiedProgramState,
    space: &TuningSpace,
    plan: &TuningPlan,
) -> Result<KirVerifiedProgramState, TuningPlanError> {
    if plan.choices.len() > 64 {
        return Err(TuningPlanError::ResourceLimit);
    }
    let mut previous = None;
    let mut seen = BTreeSet::new();
    for choice in &plan.choices {
        let unit = space
            .units
            .iter()
            .find(|unit| unit.unit_id == choice.unit_id)
            .ok_or(TuningPlanError::UnknownChoice)?;
        if unit.class != choice.class
            || !unit.variants.iter().any(|variant| {
                variant.variant_id == choice.variant_id && variant.class == choice.class
            })
        {
            return Err(TuningPlanError::UnknownChoice);
        }
        let key = (choice.class.replay_phase(), choice.unit_id);
        if previous.is_some_and(|prior| prior >= key) || !seen.insert(choice.unit_id) {
            return Err(TuningPlanError::NonCanonicalOrder);
        }
        previous = Some(key);
    }
    if plan.digest != plan_digest(&plan.choices) {
        return Err(TuningPlanError::DigestMismatch);
    }
    let (expected, replayed) = replay_selections(
        state,
        space,
        &plan
            .choices
            .iter()
            .map(|choice| (choice.unit_id, choice.variant_id))
            .collect::<Vec<_>>(),
    )?;
    if expected.choices != plan.choices || expected.digest != plan.digest {
        return Err(TuningPlanError::PreStateMismatch);
    }
    Ok(replayed)
}

/// Replays a checked plan from a fresh immutable verified pre-state.
///
/// # Errors
///
/// Returns only independent-check failures. Phase-specific materialization is
/// deliberately routed through the existing verified optimizer transactions.
pub fn apply_tuning_plan(
    state: &KirVerifiedProgramState,
    space: &TuningSpace,
    plan: &TuningPlan,
) -> Result<KirVerifiedProgramState, TuningPlanError> {
    CheckedTuningSpace::check(state, space)?
        .apply(plan)
        .map(|checked| checked.state)
}

/// Reconstructs and independently checks the exact single predicated Floyd
/// update selected by a tuning plan.
///
/// This is intentionally narrower than ordinary tuning replay: it is the
/// source-aware authorization boundary for the v0.14 performance attestation,
/// not a general description of every legal tuning plan.
///
/// # Errors
///
/// Rejects every unchecked, compound, non-Floyd, non-predicated, structurally
/// ambiguous, or over-threshold selection.
pub fn attest_selected_predicated_update(
    state: &KirVerifiedProgramState,
    space: &TuningSpace,
    plan: &TuningPlan,
) -> Result<PredicatedUpdateAttestation, TuningPlanError> {
    check_tuning_plan(state, space, plan)?;
    let [choice] = plan.choices.as_slice() else {
        return Err(attestation_error(
            "expected exactly one selected tuning choice",
        ));
    };
    if choice.class != TuneAlternativeClass::LoopSimd {
        return Err(attestation_error("selected choice is not Loop SIMD"));
    }
    let unit = space
        .units
        .iter()
        .find(|unit| unit.unit_id == choice.unit_id)
        .ok_or(TuningPlanError::UnknownChoice)?;
    if unit.class != TuneAlternativeClass::LoopSimd || unit.site_ids.len() != 1 {
        return Err(attestation_error(
            "selected Loop SIMD unit does not name exactly one site",
        ));
    }
    let variant = unit
        .variants
        .iter()
        .find(|variant| variant.variant_id == choice.variant_id)
        .ok_or(TuningPlanError::UnknownChoice)?;
    let [alternative] = variant.site_alternatives.as_slice() else {
        return Err(attestation_error(
            "selected Loop SIMD variant does not contain exactly one alternative",
        ));
    };
    if alternative.site_id != unit.site_ids[0]
        || alternative.pre_state_digest != space.pre_state_digest
        || alternative.post_state_digest != variant.isolated_post_state_digest
    {
        return Err(attestation_error(
            "selected alternative has inconsistent source identity",
        ));
    }
    let site = space
        .sites
        .iter()
        .find(|site| site.site_id == alternative.site_id)
        .ok_or_else(|| attestation_error("selected alternative site is absent"))?;
    if site.class != TuneAlternativeClass::LoopSimd
        || site.function_symbol != "floyd"
        || site.root_anchor.function_symbol != site.function_symbol
    {
        return Err(attestation_error(
            "selected alternative is not the target Floyd Loop SIMD site",
        ));
    }
    let TuneVariantAction::LoopSimd(candidate) = &variant.action else {
        return Err(attestation_error(
            "selected variant action is not Loop SIMD",
        ));
    };
    if candidate.minimum_trip == 0 || candidate.minimum_trip > 128 {
        return Err(attestation_error(
            "selected predicated update minimum is outside 1..=128",
        ));
    }
    let update = candidate
        .predicated_update
        .as_ref()
        .ok_or_else(|| attestation_error("selected Loop SIMD action is not predicated"))?;
    if candidate.diamond.is_some() || candidate.reduction.is_some() {
        return Err(attestation_error(
            "selected predicated update contains another vector shape",
        ));
    }
    let lane_bits = candidate
        .operations
        .iter()
        .map(|operation| {
            u32::from(operation.lane_type.bit_width())
                .max(u32::from(operation.result_lane_type.bit_width()))
        })
        .max()
        .ok_or_else(|| attestation_error("selected predicated update has no operations"))?;
    let vector_bits = u32::from(candidate.vf)
        .checked_mul(lane_bits)
        .ok_or(TuningPlanError::ResourceLimit)?;
    let interleave = u32::from(candidate.uf);
    if alternative.payload
        != (TuneAlternativePayload::LoopSimd {
            vector_bits,
            interleave,
            break_even_iterations: candidate.minimum_trip,
        })
    {
        return Err(attestation_error(
            "selected alternative payload does not match the checked vector action",
        ));
    }

    // Run the independent vector checker explicitly at this authorization
    // boundary, even though full plan checking already replays the same action.
    let late_state = advance_to_tunable_late_o3(state)?;
    let prepared = prepare_tuned_vectorization_trial(&late_state, candidate)
        .map_err(map_materialization_error)?;
    check_tuned_vectorization_trial_independently(
        &late_state,
        &prepared.trial,
        &prepared.plan,
        &prepared.charge,
        candidate.minimum_trip,
    )
    .map_err(map_transaction_check_error)?;

    let replayed = apply_tuning_plan(state, space, plan)?;
    if tuning_kir_state_digest(&replayed)? != choice.post_state_digest {
        return Err(attestation_error(
            "selected choice post-state does not match independent replay",
        ));
    }

    Ok(PredicatedUpdateAttestation {
        function: site.function_symbol.clone(),
        header: candidate.header,
        compare: update.condition_instruction,
        load: update.old_load_instruction,
        store: update.store_instruction,
        unit_id: unit.unit_id,
        variant_id: variant.variant_id,
        alternative_id: alternative.alternative_id,
        vector_bits,
        interleave,
        minimum: candidate.minimum_trip,
        pre_state_digest: choice.pre_state_digest,
        post_state_digest: choice.post_state_digest,
    })
}

fn attestation_error(reason: &str) -> TuningPlanError {
    TuningPlanError::IllegalAlternative(format!("predicated-update attestation: {reason}"))
}

fn vector_candidate_stable_payload(candidate: &VectorizationCandidate) -> String {
    let shape = if candidate.predicated_update.is_some() {
        "predicated-same-place-update"
    } else if candidate.diamond.is_some() {
        "diamond"
    } else if candidate.reduction.is_some() {
        "reduction"
    } else {
        "straight-line"
    };
    format!("{};shape={shape}", candidate.key.stable_text())
}

/// Formats the one canonical v0.14 source-aware attestation record.
#[must_use]
pub fn format_predicated_update_attestation(attestation: &PredicatedUpdateAttestation) -> String {
    format!(
        "CKTUNE-ATTEST/1 shape=predicated-same-place-update function={} header={} compare={} load={} store={} unit={} variant={} alternative={} vectorBits={} uf={} minimum={} pre={} post={}",
        attestation.function,
        attestation.header.index(),
        attestation.compare.index(),
        attestation.load.index(),
        attestation.store.index(),
        encode_attestation_digest(&attestation.unit_id),
        encode_attestation_digest(&attestation.variant_id),
        encode_attestation_digest(&attestation.alternative_id),
        attestation.vector_bits,
        attestation.interleave,
        attestation.minimum,
        encode_attestation_digest(&attestation.pre_state_digest),
        encode_attestation_digest(&attestation.post_state_digest),
    )
}

fn encode_attestation_digest(value: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(64);
    for byte in value {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

/// Constructs canonical state-linked choices and returns the exact replayed KIR.
pub(crate) fn derive_tuning_plan(
    state: &KirVerifiedProgramState,
    space: &TuningSpace,
    selections: &[([u8; 32], [u8; 32])],
) -> Result<(TuningPlan, KirVerifiedProgramState), TuningPlanError> {
    CheckedTuningSpace::check(state, space)?.derive(selections)
}

#[derive(Debug)]
struct VariantSeed {
    class: TuneAlternativeClass,
    function_symbol: String,
    root_anchor: TuneRootAnchor,
    parameter: u32,
    payload: String,
    action: TuneVariantAction,
}

impl VariantSeed {
    fn new(
        class: TuneAlternativeClass,
        function_symbol: String,
        root_anchor: TuneRootAnchor,
        parameter: u32,
        payload: String,
        action: TuneVariantAction,
    ) -> Self {
        Self {
            class,
            function_symbol,
            root_anchor,
            parameter,
            payload,
            action,
        }
    }
}

fn replay_selections(
    state: &KirVerifiedProgramState,
    space: &TuningSpace,
    selections: &[([u8; 32], [u8; 32])],
) -> Result<(TuningPlan, KirVerifiedProgramState), TuningPlanError> {
    replay_selections_with_cache(state, space, selections, None)
        .map(|(plan, replayed)| (plan, Rc::unwrap_or_clone(replayed)))
}

fn replay_selections_with_cache(
    state: &KirVerifiedProgramState,
    space: &TuningSpace,
    selections: &[TuneSelection],
    mut cache: Option<&mut ReplayPrefixCache>,
) -> Result<(TuningPlan, Rc<KirVerifiedProgramState>), TuningPlanError> {
    #[cfg(test)]
    tests::PLAN_REPLAYS.with(|count| count.set(count.get() + 1));
    if selections.len() > 64 || replay_identity(state, &mut cache)?.0 != space.pre_state_digest {
        return Err(TuningPlanError::PreStateMismatch);
    }
    let cached = cache.as_ref().and_then(|cache| {
        (1..=selections.len())
            .rev()
            .find_map(|length| cache.prefixes.get(&selections[..length]).cloned())
    });
    let ReplayPrefix {
        state: mut current,
        mut choices,
        mut previous_key,
        mut contains_selected_rewrite,
        mut entered_late_o3,
    } = cached.unwrap_or_else(|| ReplayPrefix {
        state: Rc::new(state.clone()),
        choices: Vec::with_capacity(selections.len()),
        previous_key: None,
        contains_selected_rewrite: false,
        entered_late_o3: false,
    });
    let mut last_finished = false;
    for (index, (unit_id, variant_id)) in selections.iter().enumerate().skip(choices.len()) {
        let unit = space
            .units
            .iter()
            .find(|unit| unit.unit_id == *unit_id)
            .ok_or(TuningPlanError::UnknownChoice)?;
        let variant = unit
            .variants
            .iter()
            .find(|variant| variant.variant_id == *variant_id)
            .ok_or(TuningPlanError::UnknownChoice)?;
        let key = (unit.class.replay_phase(), unit.unit_id);
        if previous_key.is_some_and(|previous| previous >= key) {
            return Err(TuningPlanError::NonCanonicalOrder);
        }
        previous_key = Some(key);
        if matches!(
            unit.class,
            TuneAlternativeClass::ShortSliceVersioning
                | TuneAlternativeClass::LoopSimd
                | TuneAlternativeClass::Unrolling
                | TuneAlternativeClass::Slp
        ) && !entered_late_o3
        {
            current = replay_phase(ReplayPhase::Late, &current, &mut cache)?;
            entered_late_o3 = true;
        }
        let pre_state_digest = replay_identity(&current, &mut cache)?.0;
        if let TuneVariantAction::Layout(layout) = &variant.action {
            current = if contains_selected_rewrite {
                if !entered_late_o3 {
                    current = replay_phase(ReplayPhase::Late, &current, &mut cache)?;
                }
                apply_layout_after_selected_o3(state, &current, layout, &mut cache)?
            } else {
                apply_layout_after_ordinary_o3(&current, layout, &mut cache)?
            };
        } else {
            #[cfg(test)]
            tests::REPLAY_ACTIONS.with(|counts| {
                *counts
                    .borrow_mut()
                    .entry(selections[..=index].to_vec())
                    .or_default() += 1;
            });
            current = Rc::new(apply_variant_action(&current, &variant.action)?);
            contains_selected_rewrite = true;
            if cache.is_none() && index + 1 == selections.len() {
                if !entered_late_o3 {
                    current = Rc::new(advance_to_tunable_late_o3(&current)?);
                }
                current = Rc::new(finish_selected_o3(&current)?);
                last_finished = true;
            }
        }
        let (post_state_digest, kir_bytes) = replay_identity(&current, &mut cache)?;
        choices.push(TunePlanChoice {
            unit_id: *unit_id,
            variant_id: *variant_id,
            class: unit.class,
            pre_state_digest,
            post_state_digest,
        });
        if let Some(cache) = cache.as_mut() {
            cache.retain(
                &selections[..=index],
                ReplayPrefix {
                    state: Rc::clone(&current),
                    choices: choices.clone(),
                    previous_key,
                    contains_selected_rewrite,
                    entered_late_o3,
                },
                kir_bytes,
            );
        }
    }
    if let Some(last) = choices.last_mut() {
        if last.class != TuneAlternativeClass::Layout && !last_finished {
            if !entered_late_o3 {
                current = replay_phase(ReplayPhase::Late, &current, &mut cache)?;
            }
            current = replay_phase(ReplayPhase::Finish, &current, &mut cache)?;
            last.post_state_digest = replay_identity(&current, &mut cache)?.0;
        }
    } else {
        current = replay_phase(ReplayPhase::Ordinary, &current, &mut cache)?;
    }
    let digest = plan_digest(&choices);
    Ok((
        TuningPlan {
            choices,
            predicted_dynamic: 0,
            predicted_static: 0,
            kir_bytes: 0,
            digest,
        },
        current,
    ))
}

fn apply_variant_action(
    state: &KirVerifiedProgramState,
    action: &TuneVariantAction,
) -> Result<KirVerifiedProgramState, TuningPlanError> {
    match action {
        TuneVariantAction::Inlining {
            candidate,
            force_inline,
        } => {
            if *force_inline {
                let trial = materialize_tuning_inline(state, *candidate)
                    .map_err(map_materialization_error)?;
                check_tuning_inline_independently(state, &trial, *candidate)
                    .map_err(TuningPlanError::IllegalAlternative)?;
                Ok(trial)
            } else {
                materialize_tuning_keep_out_of_line(state, *candidate)
            }
        }
        TuneVariantAction::Specialization(candidate) => {
            let prepared = prepare_specialization_trial(state, candidate, 0)
                .map_err(map_materialization_error)?;
            check_tuned_specialization_plan_independently(
                state,
                &prepared.trial,
                &prepared.plan,
                &prepared.charge,
            )
            .map_err(map_transaction_check_error)?;
            Ok(prepared.trial)
        }
        TuneVariantAction::Unrolling(candidate) => {
            let prepared =
                prepare_unroll_trial(state, candidate).map_err(map_materialization_error)?;
            check_unroll_structure_independently(
                state,
                &prepared.trial,
                &prepared.plan,
                &prepared.charge,
            )
            .map_err(map_transaction_check_error)?;
            Ok(prepared.trial)
        }
        TuneVariantAction::LoopSimd(candidate) => {
            let prepared = prepare_tuned_vectorization_trial(state, candidate)
                .map_err(map_materialization_error)?;
            check_tuned_vectorization_trial_independently(
                state,
                &prepared.trial,
                &prepared.plan,
                &prepared.charge,
                candidate.minimum_trip,
            )
            .map_err(map_transaction_check_error)?;
            Ok(prepared.trial)
        }
        TuneVariantAction::Slp(candidate) => {
            let prepared =
                prepare_slp_trial(state, candidate).map_err(map_materialization_error)?;
            check_tuned_slp_plan_independently(
                state,
                &prepared.trial,
                &prepared.plan,
                &prepared.charge,
            )
            .map_err(map_transaction_check_error)?;
            Ok(prepared.trial)
        }
        TuneVariantAction::ShortSliceVersioning {
            candidate,
            maximum_length,
        } => {
            if candidate.minimum_trip != maximum_length.saturating_add(1) {
                return Err(TuningPlanError::IllegalAlternative(
                    "short-slice threshold does not match its vector candidate".to_string(),
                ));
            }
            let prepared = prepare_tuned_vectorization_trial(state, candidate)
                .map_err(map_materialization_error)?;
            check_tuned_vectorization_trial_independently(
                state,
                &prepared.trial,
                &prepared.plan,
                &prepared.charge,
                candidate.minimum_trip,
            )
            .map_err(map_transaction_check_error)?;
            Ok(prepared.trial)
        }
        TuneVariantAction::Layout(layout) => {
            check_layout_action(state, layout)?;
            let mut module = state.module().clone();
            module.tune_layout = Some(KirTuneLayoutPlan {
                functions: vec![KirTuneFunctionLayout {
                    function: layout.function,
                    blocks: layout.blocks.clone(),
                }],
            });
            let trial = KirVerifiedProgramState::from_parts(
                module,
                state.contract_facts().cloned(),
                state.proofs().clone(),
                state.eliminated_guards().to_vec(),
                state.evidence_generation(),
            )
            .map_err(TuningPlanError::IllegalAlternative)?;
            check_layout_trial_independently(state, &trial, layout)?;
            Ok(trial)
        }
    }
}

fn materialize_tuning_keep_out_of_line(
    state: &KirVerifiedProgramState,
    requested: InlineTuningCandidate,
) -> Result<KirVerifiedProgramState, TuningPlanError> {
    if !discover_tuning_inline_candidates(
        state.module(),
        state.contract_facts(),
        state.eliminated_guards(),
    )
    .contains(&requested)
    {
        return Err(TuningPlanError::IllegalAlternative(
            "keep-out-of-line tuning candidate is stale or illegal".to_string(),
        ));
    }
    let mut module = state.module().clone();
    let callee = module
        .functions
        .iter_mut()
        .find(|function| function.id == requested.callee)
        .ok_or(TuningPlanError::PreStateMismatch)?;
    if callee.exported || callee.tune_noinline {
        return Err(TuningPlanError::IllegalAlternative(
            "keep-out-of-line tuning requires one private unmarked callee".to_string(),
        ));
    }
    callee.tune_noinline = true;
    let trial = rebuild_preserving_entry_units(
        state,
        module,
        state.contract_facts().cloned(),
        state.proofs().clone(),
        state.eliminated_guards().to_vec(),
    )?;

    let mut restored = trial.module().clone();
    let restored_callee = restored
        .functions
        .iter_mut()
        .find(|function| function.id == requested.callee)
        .ok_or(TuningPlanError::PreStateMismatch)?;
    restored_callee.tune_noinline = false;
    if restored != *state.module()
        || trial.contract_facts() != state.contract_facts()
        || trial.proofs() != state.proofs()
        || trial.eliminated_guards() != state.eliminated_guards()
    {
        return Err(TuningPlanError::IllegalAlternative(
            "keep-out-of-line trial changed data outside its checked callee attribute".to_string(),
        ));
    }
    Ok(trial)
}

fn map_transaction_check_error(error: TransactionCheckError) -> TuningPlanError {
    match error {
        TransactionCheckError::Reject(reason) if reason.contains("growth") => {
            TuningPlanError::GrowthRejected(reason)
        }
        TransactionCheckError::Reject(reason) => TuningPlanError::IllegalAlternative(reason),
        TransactionCheckError::Compiler(reason) => TuningPlanError::ReplayFailure(reason),
    }
}

fn map_materialization_error(reason: String) -> TuningPlanError {
    if reason.contains("growth") {
        TuningPlanError::GrowthRejected(reason)
    } else {
        TuningPlanError::IllegalAlternative(reason)
    }
}

fn finish_ordinary_o3(
    state: &KirVerifiedProgramState,
) -> Result<KirVerifiedProgramState, TuningPlanError> {
    let result = run_kir_pass_pipeline(
        state.module().clone(),
        KirOptimizationLevel::O3,
        state.contract_facts(),
    );
    if !result.errors.is_empty() {
        return Err(TuningPlanError::ReplayFailure(result.errors.join("; ")));
    }
    let artifact = result.artifact.ok_or_else(|| {
        TuningPlanError::ReplayFailure("verified O3 suffix withheld its artifact".to_string())
    })?;
    KirVerifiedProgramState::from_parts(
        artifact,
        result.contract_facts,
        result.proofs,
        result.eliminated_guards,
        0,
    )
    .map_err(TuningPlanError::ReplayFailure)
}

fn rebuild_preserving_entry_units(
    prior: &KirVerifiedProgramState,
    module: crate::KirModule,
    contract_facts: Option<super::ContractFactSet>,
    proofs: super::ProofArena,
    eliminated_guards: Vec<super::KirGuardElimination>,
) -> Result<KirVerifiedProgramState, TuningPlanError> {
    KirVerifiedProgramState::from_checked_parts_with_entry_units(
        module,
        contract_facts,
        proofs,
        eliminated_guards,
        prior.evidence_generation(),
        prior.optimization_entry_module_units(),
    )
    .map_err(TuningPlanError::ReplayFailure)
}

fn advance_to_tunable_late_o3(
    state: &KirVerifiedProgramState,
) -> Result<KirVerifiedProgramState, TuningPlanError> {
    #[cfg(test)]
    tests::record_analysis("late", state);
    let mut module = state.module().clone();
    let contract_facts = state.contract_facts().cloned();
    let mut proofs = state.proofs().clone();
    let mut eliminated_guards = state.eliminated_guards().to_vec();
    let mut explanations = Vec::new();

    run_memory_ssa_refine(&mut module, contract_facts.as_ref())
        .map_err(TuningPlanError::ReplayFailure)?;
    run_gvn(&mut module, &proofs.instruction_dependencies());
    run_load_forwarding(&mut module);
    run_dead_store_elimination(&mut module);
    run_integer_constant_folding(&mut module, contract_facts.as_ref(), &proofs)
        .map_err(TuningPlanError::ReplayFailure)?;
    let _ = run_sccp_range(&module);
    run_check_elimination(
        &mut module,
        contract_facts.as_ref(),
        &mut proofs,
        &mut eliminated_guards,
        &mut explanations,
        state.evidence_generation(),
        true,
    )
    .map_err(TuningPlanError::ReplayFailure)?;

    let loop_analyses = module
        .functions
        .iter()
        .map(super::analyze_natural_loops)
        .collect::<Vec<_>>();
    run_licm(
        &mut module,
        &proofs.instruction_dependencies(),
        &loop_analyses,
    )
    .map_err(TuningPlanError::ReplayFailure)?;
    run_induction_simplification(&mut module, &proofs, &loop_analyses)
        .map_err(TuningPlanError::ReplayFailure)?;
    run_integer_constant_folding(&mut module, contract_facts.as_ref(), &proofs)
        .map_err(TuningPlanError::ReplayFailure)?;
    let _ = run_sccp_range(&module);
    run_check_elimination(
        &mut module,
        contract_facts.as_ref(),
        &mut proofs,
        &mut eliminated_guards,
        &mut explanations,
        state.evidence_generation(),
        true,
    )
    .map_err(TuningPlanError::ReplayFailure)?;

    rebuild_preserving_entry_units(state, module, contract_facts, proofs, eliminated_guards)
}

fn finish_selected_o3(
    state: &KirVerifiedProgramState,
) -> Result<KirVerifiedProgramState, TuningPlanError> {
    #[cfg(test)]
    tests::record_analysis("finish", state);
    let mut module = state.module().clone();
    run_dead_code_elimination(&mut module, &state.proofs().instruction_dependencies());
    run_cleanup(&mut module);
    rebuild_preserving_entry_units(
        state,
        module,
        state.contract_facts().cloned(),
        state.proofs().clone(),
        state.eliminated_guards().to_vec(),
    )
}

fn apply_layout_after_ordinary_o3(
    state: &KirVerifiedProgramState,
    requested: &TuneLayoutAction,
    cache: &mut Option<&mut ReplayPrefixCache>,
) -> Result<Rc<KirVerifiedProgramState>, TuningPlanError> {
    check_layout_action(state, requested)?;
    let ordinary = replay_phase(ReplayPhase::Ordinary, state, cache)?;
    let function = ordinary
        .module()
        .functions
        .iter()
        .find(|function| function.id == requested.function)
        .ok_or(TuningPlanError::PreStateMismatch)?;
    let live = function
        .blocks
        .iter()
        .map(|block| block.id)
        .collect::<BTreeSet<_>>();
    let mut blocks = requested
        .blocks
        .iter()
        .copied()
        .filter(|block| live.contains(block))
        .collect::<Vec<_>>();
    if blocks.is_empty() {
        return Ok(ordinary);
    }
    let mut seen = blocks.iter().copied().collect::<BTreeSet<_>>();
    blocks.extend(
        function
            .blocks
            .iter()
            .map(|block| block.id)
            .filter(|block| seen.insert(*block)),
    );
    if blocks
        .iter()
        .copied()
        .eq(function.blocks.iter().map(|block| block.id))
    {
        return Ok(ordinary);
    }
    apply_variant_action(
        &ordinary,
        &TuneVariantAction::Layout(TuneLayoutAction {
            scope: requested.scope,
            function: requested.function,
            blocks,
        }),
    )
    .map(Rc::new)
}

fn apply_layout_after_selected_o3(
    original: &KirVerifiedProgramState,
    selected: &KirVerifiedProgramState,
    requested: &TuneLayoutAction,
    cache: &mut Option<&mut ReplayPrefixCache>,
) -> Result<Rc<KirVerifiedProgramState>, TuningPlanError> {
    check_layout_action(original, requested)?;
    let finished = replay_phase(ReplayPhase::Finish, selected, cache)?;
    let Some(function) = finished
        .module()
        .functions
        .iter()
        .find(|function| function.id == requested.function)
    else {
        return Ok(finished);
    };
    let live = function
        .blocks
        .iter()
        .map(|block| block.id)
        .collect::<BTreeSet<_>>();
    let mut blocks = requested
        .blocks
        .iter()
        .copied()
        .filter(|block| live.contains(block))
        .collect::<Vec<_>>();
    if blocks.is_empty() {
        return Ok(finished);
    }
    let mut seen = blocks.iter().copied().collect::<BTreeSet<_>>();
    blocks.extend(
        function
            .blocks
            .iter()
            .map(|block| block.id)
            .filter(|block| seen.insert(*block)),
    );
    if blocks
        .iter()
        .copied()
        .eq(function.blocks.iter().map(|block| block.id))
    {
        return Ok(finished);
    }
    apply_variant_action(
        &finished,
        &TuneVariantAction::Layout(TuneLayoutAction {
            scope: requested.scope,
            function: requested.function,
            blocks,
        }),
    )
    .map(Rc::new)
}

/// Derives the normative identity digest of the canonical pre-tune KIR bytes.
pub fn tuning_pre_kir_digest(state: &KirVerifiedProgramState) -> Result<[u8; 32], TuningPlanError> {
    let kir = print_kir_module(state.module());
    let length = u64::try_from(kir.len()).map_err(|_| TuningPlanError::ResourceLimit)?;
    if length > 32 * 1024 * 1024 {
        return Err(TuningPlanError::ResourceLimit);
    }
    let mut hasher = Sha256::new();
    hasher.update(b"CK-TUNE-PRE-KIR\0");
    hasher.update(1u32.to_be_bytes());
    hasher.update(length.to_be_bytes());
    hasher.update(kir.as_bytes());
    Ok(hasher.finalize().into())
}

/// Derives the normative identity of one complete canonical KIR state.
pub fn tuning_kir_state_digest(
    state: &KirVerifiedProgramState,
) -> Result<[u8; 32], TuningPlanError> {
    tuning_kir_state_identity(state).map(|(digest, _)| digest)
}

fn tuning_kir_state_identity(
    state: &KirVerifiedProgramState,
) -> Result<([u8; 32], usize), TuningPlanError> {
    #[cfg(test)]
    tests::record_analysis("identity", state);
    let kir = print_kir_module(state.module());
    let length = u64::try_from(kir.len()).map_err(|_| TuningPlanError::ResourceLimit)?;
    if length > 32 * 1024 * 1024 {
        return Err(TuningPlanError::ResourceLimit);
    }
    let mut blob = Vec::with_capacity(8 + kir.len());
    blob.extend_from_slice(&length.to_be_bytes());
    blob.extend_from_slice(kir.as_bytes());
    let mut material = Vec::new();
    canonical_field(&mut material, 1, &1u32.to_be_bytes());
    canonical_field(&mut material, 2, &blob);
    Ok((
        hash_canonical_record(b"CK-TUNE-KIR-STATE\0", &material),
        kir.len(),
    ))
}

pub(crate) fn canonical_root_anchor(anchor: &TuneRootAnchor) -> Vec<u8> {
    let mut out = Vec::new();
    canonical_field(&mut out, 1, &canonical_text(&anchor.function_symbol));
    canonical_field(&mut out, 2, &[anchor.kind]);
    canonical_field(&mut out, 3, &anchor.preorder_ordinal.to_be_bytes());
    out
}

pub(crate) fn canonical_alternative_payload(payload: &TuneAlternativePayload) -> Vec<u8> {
    let (class, value) = match payload {
        TuneAlternativePayload::Inlining {
            callee_symbol,
            force_inline,
        } => {
            let mut value = Vec::new();
            canonical_field(&mut value, 1, &canonical_text(callee_symbol));
            canonical_field(&mut value, 2, &[if *force_inline { 1 } else { 2 }]);
            (TuneAlternativeClass::Inlining, value)
        }
        TuneAlternativePayload::Specialization { bindings, guarded } => {
            let items = bindings
                .iter()
                .map(|binding| {
                    let mut item = Vec::new();
                    canonical_field(&mut item, 1, &binding.argument_ordinal.to_be_bytes());
                    canonical_field(&mut item, 2, &[binding.kind]);
                    canonical_field(&mut item, 3, &binding.bits.to_be_bytes());
                    canonical_record(&item)
                })
                .collect::<Vec<_>>();
            let mut value = Vec::new();
            canonical_field(&mut value, 1, &canonical_list(&items));
            canonical_field(&mut value, 2, &[u8::from(*guarded)]);
            (TuneAlternativeClass::Specialization, value)
        }
        TuneAlternativePayload::Unrolling { factor } => {
            let mut value = Vec::new();
            canonical_field(&mut value, 1, &factor.to_be_bytes());
            (TuneAlternativeClass::Unrolling, value)
        }
        TuneAlternativePayload::LoopSimd {
            vector_bits,
            interleave,
            break_even_iterations,
        } => {
            let mut value = Vec::new();
            canonical_field(&mut value, 1, &vector_bits.to_be_bytes());
            canonical_field(&mut value, 2, &interleave.to_be_bytes());
            canonical_field(&mut value, 3, &break_even_iterations.to_be_bytes());
            (TuneAlternativeClass::LoopSimd, value)
        }
        TuneAlternativePayload::Slp {
            pack_width,
            operand_anchors,
        } => {
            let anchors = operand_anchors
                .iter()
                .map(|anchor| canonical_record(&canonical_root_anchor(anchor)))
                .collect::<Vec<_>>();
            let mut value = Vec::new();
            canonical_field(&mut value, 1, &pack_width.to_be_bytes());
            canonical_field(&mut value, 2, &canonical_list(&anchors));
            (TuneAlternativeClass::Slp, value)
        }
        TuneAlternativePayload::ShortSliceVersioning {
            maximum_length,
            vector_bits,
            interleave,
        } => {
            let mut value = Vec::new();
            canonical_field(&mut value, 1, &maximum_length.to_be_bytes());
            canonical_field(&mut value, 2, &vector_bits.to_be_bytes());
            canonical_field(&mut value, 3, &interleave.to_be_bytes());
            (TuneAlternativeClass::ShortSliceVersioning, value)
        }
        TuneAlternativePayload::Layout { scope, root_order } => {
            let roots = root_order
                .iter()
                .map(|root| root.to_vec())
                .collect::<Vec<_>>();
            let mut value = Vec::new();
            canonical_field(&mut value, 1, &[*scope]);
            canonical_field(&mut value, 2, &canonical_list(&roots));
            (TuneAlternativeClass::Layout, value)
        }
    };
    let mut out = Vec::new();
    canonical_field(&mut out, 1, &[class as u8]);
    canonical_field(&mut out, 2, &canonical_record(&value));
    out
}

pub(crate) fn canonical_site(site: &TuneSite) -> Vec<u8> {
    let mut out = Vec::new();
    canonical_field(&mut out, 1, &site.site_id);
    canonical_field(&mut out, 2, &[site.class as u8]);
    canonical_field(&mut out, 3, &site.root_id);
    canonical_field(&mut out, 4, &site.pre_state_digest);
    canonical_field(&mut out, 5, &site.canonical_rank.to_be_bytes());
    canonical_field(
        &mut out,
        6,
        &canonical_record(&canonical_root_anchor(&site.root_anchor)),
    );
    out
}

pub(crate) fn canonical_site_alternative(alternative: &TuneSiteAlternative) -> Vec<u8> {
    let mut out = Vec::new();
    canonical_field(&mut out, 1, &alternative.site_id);
    canonical_field(&mut out, 2, &alternative.alternative_id);
    canonical_field(&mut out, 3, &alternative.pre_state_digest);
    canonical_field(&mut out, 4, &alternative.post_state_digest);
    canonical_field(
        &mut out,
        5,
        &canonical_record(&canonical_alternative_payload(&alternative.payload)),
    );
    out
}

pub(crate) fn canonical_variant(variant: &TuneVariant) -> Vec<u8> {
    let alternatives = variant
        .site_alternatives
        .iter()
        .map(|alternative| canonical_record(&canonical_site_alternative(alternative)))
        .collect::<Vec<_>>();
    let mut out = Vec::new();
    canonical_field(&mut out, 1, &variant.variant_id);
    canonical_field(&mut out, 2, &[variant.class as u8]);
    canonical_field(&mut out, 3, &canonical_list(&alternatives));
    canonical_field(
        &mut out,
        4,
        &variant.isolated_dynamic_estimate.to_be_bytes(),
    );
    canonical_field(&mut out, 5, &variant.isolated_static_estimate.to_be_bytes());
    canonical_field(&mut out, 6, &variant.isolated_kir_bytes.to_be_bytes());
    canonical_field(&mut out, 7, &variant.isolated_post_state_digest);
    out
}

pub(crate) fn canonical_unit(unit: &TuneUnit, baseline: [u8; 32]) -> Vec<u8> {
    let site_ids = unit
        .site_ids
        .iter()
        .map(|id| id.to_vec())
        .collect::<Vec<_>>();
    let variants = unit
        .variants
        .iter()
        .map(|variant| canonical_record(&canonical_variant(variant)))
        .collect::<Vec<_>>();
    let mut out = Vec::new();
    canonical_field(&mut out, 1, &unit.unit_id);
    canonical_field(&mut out, 2, &canonical_list(&site_ids));
    canonical_field(&mut out, 3, &baseline);
    canonical_field(&mut out, 4, &canonical_list(&variants));
    out
}

fn derive_root_id(pre_tune: [u8; 32], anchor: &TuneRootAnchor) -> [u8; 32] {
    let mut material = Vec::new();
    canonical_field(&mut material, 1, &pre_tune);
    canonical_field(
        &mut material,
        2,
        &canonical_record(&canonical_root_anchor(anchor)),
    );
    hash_canonical_record(b"CK-TUNE-ROOT\0", &material)
}

fn derive_site_id(
    root_id: [u8; 32],
    class: TuneAlternativeClass,
    ordinal: u32,
    pre_state: [u8; 32],
) -> [u8; 32] {
    let mut material = Vec::new();
    canonical_field(&mut material, 1, &root_id);
    canonical_field(&mut material, 2, &[class as u8]);
    canonical_field(&mut material, 3, &ordinal.to_be_bytes());
    canonical_field(&mut material, 4, &pre_state);
    hash_canonical_record(b"CK-TUNE-SITE\0", &material)
}

fn derive_alternative_id(
    site_id: [u8; 32],
    payload: &TuneAlternativePayload,
    post_state: [u8; 32],
) -> [u8; 32] {
    let mut material = Vec::new();
    canonical_field(&mut material, 1, &site_id);
    canonical_field(
        &mut material,
        2,
        &canonical_record(&canonical_alternative_payload(payload)),
    );
    canonical_field(&mut material, 3, &post_state);
    hash_canonical_record(b"CK-TUNE-ALTERNATIVE\0", &material)
}

fn derive_unit_id(site_ids: &[[u8; 32]], baseline: [u8; 32]) -> [u8; 32] {
    let ids = site_ids.iter().map(|id| id.to_vec()).collect::<Vec<_>>();
    let mut material = Vec::new();
    canonical_field(&mut material, 1, &canonical_list(&ids));
    canonical_field(&mut material, 2, &baseline);
    hash_canonical_record(b"CK-TUNE-UNIT\0", &material)
}

#[allow(clippy::too_many_arguments)]
fn derive_unit_variant_id(
    unit_id: [u8; 32],
    class: TuneAlternativeClass,
    alternatives: &[TuneSiteAlternative],
    dynamic: u64,
    static_cost: u64,
    kir_bytes: u64,
    post_state: [u8; 32],
) -> [u8; 32] {
    let alternatives = alternatives
        .iter()
        .map(|alternative| canonical_record(&canonical_site_alternative(alternative)))
        .collect::<Vec<_>>();
    let mut material = Vec::new();
    canonical_field(&mut material, 1, &unit_id);
    canonical_field(&mut material, 2, &[class as u8]);
    canonical_field(&mut material, 3, &canonical_list(&alternatives));
    canonical_field(&mut material, 4, &dynamic.to_be_bytes());
    canonical_field(&mut material, 5, &static_cost.to_be_bytes());
    canonical_field(&mut material, 6, &kir_bytes.to_be_bytes());
    canonical_field(&mut material, 7, &post_state);
    hash_canonical_record(b"CK-TUNE-UNIT-VARIANT\0", &material)
}

fn derive_space_digest(sites: &[TuneSite], units: &[TuneUnit], baseline: [u8; 32]) -> [u8; 32] {
    let sites = sites
        .iter()
        .map(|site| canonical_record(&canonical_site(site)))
        .collect::<Vec<_>>();
    let units = units
        .iter()
        .map(|unit| canonical_record(&canonical_unit(unit, baseline)))
        .collect::<Vec<_>>();
    let mut material = Vec::new();
    canonical_field(&mut material, 1, &canonical_list(&sites));
    canonical_field(&mut material, 2, &canonical_list(&units));
    hash_canonical_record(b"CK-TUNE-CANDIDATE-SPACE\0", &material)
}

fn payload_for_action(
    state: &KirVerifiedProgramState,
    action: &TuneVariantAction,
) -> Result<Option<TuneAlternativePayload>, TuningPlanError> {
    match action {
        TuneVariantAction::Inlining {
            candidate,
            force_inline,
        } => {
            let callee = state
                .module()
                .functions
                .iter()
                .find(|function| function.id == candidate.callee)
                .ok_or(TuningPlanError::PreStateMismatch)?;
            Ok(Some(TuneAlternativePayload::Inlining {
                callee_symbol: callee.name.clone(),
                force_inline: *force_inline,
            }))
        }
        TuneVariantAction::Specialization(candidate) => {
            let callee = state
                .module()
                .functions
                .iter()
                .find(|function| function.id == candidate.callee)
                .ok_or(TuningPlanError::PreStateMismatch)?;
            let mut bindings = Vec::new();
            for fact in &candidate.facts {
                let parameter = callee
                    .params
                    .get(
                        usize::try_from(fact.parameter_index)
                            .map_err(|_| TuningPlanError::ResourceLimit)?,
                    )
                    .ok_or(TuningPlanError::PreStateMismatch)?;
                let Some((kind, bits)) = specialization_bits(&parameter.type_node, &fact.value)?
                else {
                    return Ok(None);
                };
                bindings.push(TuneSpecializationBinding {
                    argument_ordinal: fact.parameter_index,
                    kind,
                    bits,
                });
            }
            bindings.sort_by_key(|binding| binding.argument_ordinal);
            if bindings.is_empty()
                || bindings.len() > 16
                || bindings
                    .windows(2)
                    .any(|pair| pair[0].argument_ordinal == pair[1].argument_ordinal)
            {
                return Ok(None);
            }
            Ok(Some(TuneAlternativePayload::Specialization {
                bindings,
                guarded: false,
            }))
        }
        TuneVariantAction::Unrolling(candidate) => {
            let factor = if candidate.full {
                candidate.trip_count
            } else {
                u32::from(candidate.factor)
            };
            Ok((factor.is_power_of_two() && (2..=64).contains(&factor))
                .then_some(TuneAlternativePayload::Unrolling { factor }))
        }
        TuneVariantAction::LoopSimd(candidate) => {
            let lane_bits = candidate
                .operations
                .iter()
                .map(|operation| {
                    u32::from(operation.lane_type.bit_width())
                        .max(u32::from(operation.result_lane_type.bit_width()))
                })
                .max()
                .unwrap_or(64);
            let vector_bits = u32::from(candidate.vf)
                .checked_mul(lane_bits)
                .ok_or(TuningPlanError::ResourceLimit)?;
            let interleave = u32::from(candidate.uf);
            Ok((vector_bits.is_power_of_two()
                && (64..=2_048).contains(&vector_bits)
                && (1..=8).contains(&interleave),)
                .0
                .then_some(TuneAlternativePayload::LoopSimd {
                    vector_bits,
                    interleave,
                    break_even_iterations: candidate.minimum_trip,
                }))
        }
        TuneVariantAction::Slp(candidate) => {
            let mut operand_anchors = Vec::with_capacity(candidate.scalar_instructions.len());
            let function = state
                .module()
                .functions
                .iter()
                .find(|function| function.id == candidate.function)
                .ok_or(TuningPlanError::PreStateMismatch)?;
            for instruction in &candidate.scalar_instructions {
                operand_anchors.push(TuneRootAnchor {
                    function_symbol: function.name.clone(),
                    kind: 5,
                    preorder_ordinal: instruction_kind_ordinal(
                        state,
                        candidate.function,
                        *instruction,
                        false,
                    )?,
                });
            }
            let pack_width = u32::from(candidate.lanes);
            Ok(
                ((2..=64).contains(&pack_width) && operand_anchors.len() <= 64).then_some(
                    TuneAlternativePayload::Slp {
                        pack_width,
                        operand_anchors,
                    },
                ),
            )
        }
        TuneVariantAction::ShortSliceVersioning {
            candidate,
            maximum_length,
        } => {
            let lane_bits = candidate
                .operations
                .iter()
                .map(|operation| {
                    u32::from(operation.lane_type.bit_width())
                        .max(u32::from(operation.result_lane_type.bit_width()))
                })
                .max()
                .unwrap_or(64);
            let vector_bits = u32::from(candidate.vf)
                .checked_mul(lane_bits)
                .ok_or(TuningPlanError::ResourceLimit)?;
            let interleave = u32::from(candidate.uf);
            Ok((candidate.minimum_trip == maximum_length.saturating_add(1)
                && vector_bits.is_power_of_two()
                && (64..=2_048).contains(&vector_bits)
                && (1..=8).contains(&interleave))
            .then_some(TuneAlternativePayload::ShortSliceVersioning {
                maximum_length: *maximum_length,
                vector_bits,
                interleave,
            }))
        }
        TuneVariantAction::Layout(layout) => {
            check_layout_action(state, layout)?;
            let function = state
                .module()
                .functions
                .iter()
                .find(|function| function.id == layout.function)
                .ok_or(TuningPlanError::PreStateMismatch)?;
            let mut roots = Vec::with_capacity(layout.blocks.len());
            let pre_tune = tuning_pre_kir_digest(state)?;
            for block in &layout.blocks {
                let ordinal = function
                    .blocks
                    .iter()
                    .position(|candidate| candidate.id == *block)
                    .and_then(|index| u32::try_from(index).ok())
                    .ok_or(TuningPlanError::PreStateMismatch)?;
                roots.push(derive_root_id(
                    pre_tune,
                    &TuneRootAnchor {
                        function_symbol: function.name.clone(),
                        kind: 4,
                        preorder_ordinal: ordinal,
                    },
                ));
            }
            Ok(Some(TuneAlternativePayload::Layout {
                scope: layout.scope,
                root_order: roots,
            }))
        }
    }
}

fn check_layout_action(
    state: &KirVerifiedProgramState,
    layout: &TuneLayoutAction,
) -> Result<(), TuningPlanError> {
    if layout.scope != 1 || state.module().tune_layout.is_some() {
        return Err(TuningPlanError::IllegalAlternative(
            "layout scope or pre-state metadata is outside schema 1".to_string(),
        ));
    }
    let function = state
        .module()
        .functions
        .iter()
        .find(|function| function.id == layout.function)
        .ok_or(TuningPlanError::PreStateMismatch)?;
    let expected = function
        .blocks
        .iter()
        .map(|block| block.id)
        .collect::<BTreeSet<_>>();
    let actual = layout.blocks.iter().copied().collect::<BTreeSet<_>>();
    if layout.blocks.len() < 3
        || layout.blocks.len() != function.blocks.len()
        || actual.len() != layout.blocks.len()
        || actual != expected
        || layout
            .blocks
            .iter()
            .copied()
            .eq(function.blocks.iter().map(|block| block.id))
    {
        return Err(TuningPlanError::IllegalAlternative(
            "layout action is not a distinct complete block permutation".to_string(),
        ));
    }
    Ok(())
}

fn check_layout_trial_independently(
    pre_state: &KirVerifiedProgramState,
    trial: &KirVerifiedProgramState,
    layout: &TuneLayoutAction,
) -> Result<(), TuningPlanError> {
    check_layout_action(pre_state, layout)?;
    let mut without_layout = trial.module().clone();
    let attached = without_layout.tune_layout.take();
    if without_layout != *pre_state.module()
        || attached
            != Some(KirTuneLayoutPlan {
                functions: vec![KirTuneFunctionLayout {
                    function: layout.function,
                    blocks: layout.blocks.clone(),
                }],
            })
        || trial.contract_facts() != pre_state.contract_facts()
        || trial.proofs() != pre_state.proofs()
        || trial.eliminated_guards() != pre_state.eliminated_guards()
    {
        return Err(TuningPlanError::IllegalAlternative(
            "layout trial changed data outside canonical layout metadata".to_string(),
        ));
    }
    Ok(())
}

fn specialization_bits(
    parameter: &MirType,
    value: &SpecializationFactValue,
) -> Result<Option<(u8, u128)>, TuningPlanError> {
    let invalid =
        || TuningPlanError::IllegalAlternative("noncanonical specialization value".into());
    match (parameter, value) {
        (
            MirType::Primitive(MirPrimitiveTypeName::U32),
            SpecializationFactValue::Integer { value },
        ) => Ok(Some((
            1,
            u128::from(value.parse::<u32>().map_err(|_| invalid())?),
        ))),
        (
            MirType::Primitive(MirPrimitiveTypeName::U64),
            SpecializationFactValue::Integer { value },
        ) => Ok(Some((
            2,
            u128::from(value.parse::<u64>().map_err(|_| invalid())?),
        ))),
        (
            MirType::Primitive(MirPrimitiveTypeName::I32),
            SpecializationFactValue::Integer { value },
        ) => {
            let value = value.parse::<i32>().map_err(|_| invalid())?;
            Ok(Some((3, u128::from(value as u32))))
        }
        (
            MirType::Primitive(MirPrimitiveTypeName::I64),
            SpecializationFactValue::Integer { value },
        ) => {
            let value = value.parse::<i64>().map_err(|_| invalid())?;
            Ok(Some((4, u128::from(value as u64))))
        }
        (
            MirType::Primitive(MirPrimitiveTypeName::F64),
            SpecializationFactValue::Float { value },
        ) => {
            let value = value.parse::<f64>().map_err(|_| invalid())?;
            Ok(Some((6, u128::from(value.to_bits()))))
        }
        (MirType::Slice(_), SpecializationFactValue::SliceLength { length }) => {
            Ok(Some((7, u128::from(*length))))
        }
        (_, SpecializationFactValue::Boolean { .. }) => Ok(None),
        _ => Err(invalid()),
    }
}

fn instruction_kind_ordinal(
    state: &KirVerifiedProgramState,
    function_id: crate::FunctionId,
    target: crate::InstructionId,
    calls_only: bool,
) -> Result<u32, TuningPlanError> {
    let function = state
        .module()
        .functions
        .iter()
        .find(|function| function.id == function_id)
        .ok_or(TuningPlanError::PreStateMismatch)?;
    let mut ordinal = 0_u32;
    for instruction in function.blocks.iter().flat_map(|block| &block.instructions) {
        let included =
            !calls_only || matches!(instruction.kind, crate::KirInstructionKind::Call { .. });
        if included {
            if instruction.id == target {
                return Ok(ordinal);
            }
            ordinal = ordinal
                .checked_add(1)
                .ok_or(TuningPlanError::ResourceLimit)?;
        }
    }
    Err(TuningPlanError::PreStateMismatch)
}

fn canonical_field(out: &mut Vec<u8>, tag: u16, value: &[u8]) {
    out.extend_from_slice(&tag.to_be_bytes());
    out.extend_from_slice(&u32::try_from(value.len()).unwrap_or(u32::MAX).to_be_bytes());
    out.extend_from_slice(value);
}

fn canonical_record(value: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + value.len());
    out.extend_from_slice(&u32::try_from(value.len()).unwrap_or(u32::MAX).to_be_bytes());
    out.extend_from_slice(value);
    out
}

fn canonical_list(values: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(
        &u32::try_from(values.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );
    for value in values {
        out.extend_from_slice(value);
    }
    out
}

fn canonical_text(value: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + value.len());
    out.extend_from_slice(&u32::try_from(value.len()).unwrap_or(u32::MAX).to_be_bytes());
    out.extend_from_slice(value.as_bytes());
    out
}

fn hash_canonical_record(domain: &[u8], material: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(canonical_record(material));
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    type ReplayActionCounts = BTreeMap<Vec<TuneSelection>, usize>;

    thread_local! {
        pub(super) static SPACE_ENUMERATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        pub(super) static PLAN_REPLAYS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        pub(super) static REPLAY_ACTIONS: std::cell::RefCell<ReplayActionCounts> = const { std::cell::RefCell::new(BTreeMap::new()) };
        static REPLAY_ANALYSES: std::cell::RefCell<BTreeMap<(&'static str, String, String), usize>> = const { std::cell::RefCell::new(BTreeMap::new()) };
    }

    pub(super) fn record_analysis(phase: &'static str, state: &KirVerifiedProgramState) {
        REPLAY_ANALYSES.with(|counts| {
            *counts
                .borrow_mut()
                .entry((
                    phase,
                    state.kir_digest(),
                    state.verification_cache().evidence_digest.clone(),
                ))
                .or_default() += 1;
        });
    }

    fn replay_test_state() -> KirVerifiedProgramState {
        let checked = crate::check(&crate::SourceFile::new(
            "replay-work.ck",
            "export fn kernel() -> u32 { let i: u32 = 0; let total: u32 = 0; while i < 12 { total = total + i; i = i + 1; } return total; }",
        ));
        assert!(checked.diagnostics.is_empty());
        let mir = crate::lower_to_mir(&checked.checked_program).expect("MIR");
        let module = crate::build_kir_module(
            &mir,
            crate::KirBuildConfig {
                consumer: crate::KirConsumer::C,
                overflow_mode: crate::KirOverflowMode::Unchecked,
                bounds_mode: crate::KirBoundsMode::Unchecked,
                sanitizer_mode: crate::KirSanitizerMode::Disabled,
            },
        )
        .expect("KIR");
        crate::prepare_kir_pre_tune_state(module, None).expect("verified pre-tune state")
    }

    #[test]
    fn tune_search_should_revalidate_the_immutable_space_only_once() {
        let state = replay_test_state();
        let space = enumerate_tuning_space(&state).expect("space");
        SPACE_ENUMERATIONS.with(|count| count.set(0));
        let search = crate::run_deterministic_search(&state, &space, crate::TuneBudget::Quick)
            .expect("search");
        assert!(!search.expansions.is_empty());
        assert_eq!(SPACE_ENUMERATIONS.with(std::cell::Cell::get), 1);
    }

    #[test]
    fn tune_search_should_materialize_each_shared_prefix_only_once() {
        let checked = crate::check(&crate::SourceFile::new(
            "shared-prefix.ck",
            include_str!("../../benches/fixtures/pgo/call_constant_length.ck"),
        ));
        assert!(checked.diagnostics.is_empty());
        let mir = crate::lower_to_mir(&checked.checked_program).expect("MIR");
        let module = crate::build_kir_module(
            &mir,
            crate::KirBuildConfig {
                consumer: crate::KirConsumer::C,
                overflow_mode: crate::KirOverflowMode::Unchecked,
                bounds_mode: crate::KirBoundsMode::Unchecked,
                sanitizer_mode: crate::KirSanitizerMode::Disabled,
            },
        )
        .expect("KIR");
        let facts =
            crate::import_contract_facts(&module, &checked.checked_program, 0).expect("facts");
        let state = crate::prepare_kir_pre_tune_state(module, Some(&facts)).expect("pre-tune");
        let space = CheckedTuningSpace::enumerate(&state).expect("space");
        REPLAY_ACTIONS.with(|counts| counts.borrow_mut().clear());
        REPLAY_ANALYSES.with(|counts| counts.borrow_mut().clear());

        let search = crate::run_checked_tuning_search(&space, crate::TuneBudget::Standard)
            .expect("complete search");

        assert!(search.expansions.len() > 8, "exercise a branching search");
        REPLAY_ACTIONS.with(|counts| {
            let counts = counts.borrow();
            assert!(!counts.is_empty());
            assert_eq!(
                counts.values().copied().max(),
                Some(1),
                "shared prefixes were replayed: {counts:?}"
            );
        });
        REPLAY_ANALYSES.with(|counts| {
            let counts = counts.borrow();
            for phase in ["late", "finish", "identity"] {
                let values = counts
                    .iter()
                    .filter(|((kind, _, _), _)| *kind == phase)
                    .map(|(_, count)| *count)
                    .collect::<Vec<_>>();
                eprintln!(
                    "phase={phase} calls={} unique={} max={:?}",
                    values.iter().sum::<usize>(),
                    values.len(),
                    values.iter().max()
                );
            }
            assert_eq!(
                counts.values().copied().max(),
                Some(1),
                "the same immutable state was analyzed again"
            );
        });
    }

    #[test]
    fn tune_plan_application_should_retain_the_independently_checked_replay() {
        let state = replay_test_state();
        let space = enumerate_tuning_space(&state).expect("space");
        PLAN_REPLAYS.with(|count| count.set(0));
        apply_tuning_plan(&state, &space, &TuningPlan::baseline()).expect("baseline replay");
        assert_eq!(PLAN_REPLAYS.with(std::cell::Cell::get), 1);
    }

    #[test]
    fn exhausted_replay_retention_should_fall_back_without_dropping_work() {
        let state = replay_test_state();
        let checked = CheckedTuningSpace::enumerate(&state).expect("space");
        let search = crate::run_checked_tuning_search(&checked, crate::TuneBudget::Thorough)
            .expect("search");
        let mut replay = checked.search_replay();
        replay.prefixes.snapshots = 256;
        replay.prefixes.canonical_bytes = 4 * 1024 * 1024;
        for plan in std::iter::once(TuningPlan::baseline()).chain(search.compile_selection) {
            let selections = plan
                .choices
                .iter()
                .map(|choice| (choice.unit_id, choice.variant_id))
                .collect::<Vec<_>>();
            assert_eq!(
                replay
                    .derive(&selections)
                    .map(|(plan, state)| (plan, Rc::unwrap_or_clone(state))),
                checked.derive(&selections)
            );
        }
        assert!(replay.prefixes.prefixes.is_empty());
        assert!(replay.prefixes.identities.is_empty());
        assert!(replay.prefixes.analyses.is_empty());
    }

    #[test]
    fn replay_identity_reuse_should_compare_the_module_not_just_its_digest_bucket() {
        let state = replay_test_state();
        let mut different = state.module().clone();
        different.functions[0].name.push_str("_different");
        let mut cache = ReplayPrefixCache::default();
        cache
            .identities
            .insert(state.kir_digest(), (different, [0; 32], 0));
        assert_eq!(cache.identity(&state), tuning_kir_state_identity(&state));
    }

    #[test]
    fn replay_analysis_reuse_should_compare_the_complete_input_state() {
        let state = replay_test_state();
        let mut different = state.clone();
        different.module_mut().functions[0]
            .name
            .push_str("_different");
        let mut cache = ReplayPrefixCache::default();
        cache.analyses.insert(
            (
                ReplayPhase::Late,
                state.kir_digest(),
                state.verification_cache().evidence_digest.clone(),
            ),
            (different.clone(), Rc::new(different)),
        );
        assert_eq!(
            cache.analyze(ReplayPhase::Late, &state).as_deref(),
            advance_to_tunable_late_o3(&state).as_ref()
        );
    }

    #[test]
    fn replay_analysis_hit_should_share_the_complete_immutable_snapshot() {
        let state = replay_test_state();
        for phase in [
            ReplayPhase::Ordinary,
            ReplayPhase::Late,
            ReplayPhase::Finish,
        ] {
            let mut cache = ReplayPrefixCache::default();
            let first = cache.analyze(phase, &state).expect("first analysis");
            let second = cache.analyze(phase, &state).expect("retained analysis");
            assert_eq!(first, second, "complete state remains identical: {phase:?}");
            assert!(
                std::ptr::eq(first.module(), second.module()),
                "{phase:?} deep-copied an immutable retained analysis"
            );
        }
    }

    #[test]
    fn repeated_search_replay_should_share_its_final_immutable_snapshot() {
        let state = replay_test_state();
        let checked = CheckedTuningSpace::enumerate(&state).expect("space");
        let mut replay = checked.search_replay();
        let (first_plan, first) = replay.derive(&[]).expect("first baseline");
        let (second_plan, second) = replay.derive(&[]).expect("retained baseline");
        assert_eq!(first_plan, second_plan);
        assert_eq!(first, second);
        assert!(
            std::ptr::eq(first.module(), second.module()),
            "search copied its read-only final snapshot before computing metrics"
        );
    }

    #[test]
    fn checked_tune_space_should_reject_mutated_discovery_before_search() {
        let state = replay_test_state();
        let space = enumerate_tuning_space(&state).expect("space");
        let mut changed = space.clone();
        changed.digest[0] ^= 1;
        assert!(matches!(
            CheckedTuningSpace::check(&state, &changed),
            Err(TuningPlanError::PreStateMismatch)
        ));
        let mut changed = space;
        changed.units[0].variants[0].isolated_kir_bytes += 1;
        assert!(matches!(
            CheckedTuningSpace::check(&state, &changed),
            Err(TuningPlanError::PreStateMismatch)
        ));
    }

    #[test]
    fn checked_tune_replays_should_equal_independent_raw_derivation() {
        let state = replay_test_state();
        let checked = CheckedTuningSpace::enumerate(&state).expect("checked space");
        for budget in [
            crate::TuneBudget::Quick,
            crate::TuneBudget::Standard,
            crate::TuneBudget::Thorough,
        ] {
            let retained =
                crate::run_checked_tuning_search(&checked, budget).expect("checked search");
            let raw = crate::run_deterministic_search(&state, checked.space(), budget)
                .expect("raw search");
            assert_eq!(retained, raw);
            for plan in std::iter::once(TuningPlan::baseline()).chain(retained.compile_selection) {
                let selections = plan
                    .choices
                    .iter()
                    .map(|choice| (choice.unit_id, choice.variant_id))
                    .collect::<Vec<_>>();
                let (independent, replayed) =
                    derive_tuning_plan(&state, checked.space(), &selections)
                        .expect("independent replay");
                let applied = checked.apply(&plan).expect("retained checked replay");
                assert_eq!(independent.choices, plan.choices);
                assert_eq!(independent.digest, plan.digest);
                assert_eq!(
                    tuning_kir_state_digest(&replayed).expect("independent digest"),
                    tuning_kir_state_digest(applied.state()).expect("checked digest")
                );
                let mut forged = plan;
                forged.digest[0] ^= 1;
                assert!(matches!(
                    checked.apply(&forged),
                    Err(TuningPlanError::DigestMismatch)
                ));
            }
        }
    }

    #[test]
    fn transaction_rejections_keep_growth_distinct_and_internal_failures_fatal() {
        assert!(matches!(
            map_materialization_error("vector-code-growth-budget-not-met".to_string()),
            TuningPlanError::GrowthRejected(_)
        ));
        assert!(matches!(
            map_transaction_check_error(TransactionCheckError::reject(
                "vector-code-growth-budget-not-met"
            )),
            TuningPlanError::GrowthRejected(_)
        ));
        assert!(matches!(
            map_transaction_check_error(TransactionCheckError::reject(
                "profitability-threshold-not-met"
            )),
            TuningPlanError::IllegalAlternative(_)
        ));
        assert!(matches!(
            map_transaction_check_error(TransactionCheckError::compiler(
                "checker invariant failed"
            )),
            TuningPlanError::ReplayFailure(_)
        ));
    }
}
