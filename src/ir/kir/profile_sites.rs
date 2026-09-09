use std::collections::{BTreeMap, BTreeSet, VecDeque};

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    CkProfileError, CkProfileSiteDescriptor, CkProfileSiteId, CkProfileSiteKind,
    profile_site_table_digest,
};

use super::{
    BlockId, FunctionId, InstructionId, KirBlock, KirConsumer, KirEdge, KirFunction,
    KirInstructionKind, KirModule, KirTerminator, ValueId, compute_kir_dominators,
    print_kir_function, print_kir_module, terminator_successors, validate_kir_module,
};

type CkProfileLoopEvent = (BlockId, Vec<BlockId>, Vec<(BlockId, BlockId)>);

/// Closed workload-profile modes represented around canonical KIR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CkProfileKirMode {
    /// Ordinary compilation: no site table, annotations, or counter operations.
    Off,
    /// Temporary collection artifact with explicit counter operations.
    Generate,
    /// Final compilation with immutable annotations and no counter operations.
    Use,
}

impl CkProfileKirMode {
    pub(super) const fn stable_name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Generate => "generate",
            Self::Use => "use",
        }
    }
}

/// A canonical runtime event to which one profile site is attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CkProfileEvent {
    FunctionEntry {
        function: FunctionId,
        block: BlockId,
    },
    Edge {
        function: FunctionId,
        from: BlockId,
        to: BlockId,
    },
    LoopTrip {
        function: FunctionId,
        header: BlockId,
        latches: Vec<BlockId>,
        exits: Vec<(BlockId, BlockId)>,
    },
    SliceLength {
        function: FunctionId,
        block: BlockId,
        instruction: InstructionId,
        value: ValueId,
    },
    CandidateConstant {
        function: FunctionId,
        block: BlockId,
        instruction: InstructionId,
        observed: ValueId,
    },
}

/// Profile operations are ordered in a domain orthogonal to CK memory/effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CkProfileEffectDomain {
    WorkloadProfile,
}

/// One immutable ordering token for a profile counter operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CkProfileEffect {
    pub domain: CkProfileEffectDomain,
    pub sequence: u32,
}

/// Non-proof association between a canonical site and its KIR event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkProfileSiteAnnotation {
    pub site_id: CkProfileSiteId,
    pub descriptor: CkProfileSiteDescriptor,
    pub event: CkProfileEvent,
}

/// Explicit generation-only counter operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkProfileOperation {
    pub site_id: CkProfileSiteId,
    pub event: CkProfileEvent,
    pub effect: CkProfileEffect,
}

/// One closed one-to-one transfer from a frozen event to an annotation/op.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkProfileMappingEntry {
    pub site_id: CkProfileSiteId,
    pub source: CkProfileEvent,
    pub target: CkProfileEvent,
    pub operation_index: Option<u32>,
}

/// Mapping identity independently checked before profile counts may be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkProfileMapping {
    pub pre_profile_kir_digest: [u8; 32],
    pub site_table_digest: [u8; 32],
    pub entries: Vec<CkProfileMappingEntry>,
}

/// Canonical KIR plus workload-profile data kept outside fact/proof storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkProfileKirPlan {
    pub mode: CkProfileKirMode,
    pub module: KirModule,
    pub pre_profile_kir_digest: [u8; 32],
    pub site_table_digest: [u8; 32],
    pub sites: Vec<CkProfileSiteDescriptor>,
    pub annotations: Vec<CkProfileSiteAnnotation>,
    pub operations: Vec<CkProfileOperation>,
    pub mapping: Option<CkProfileMapping>,
}

/// A stable KIR workload-profile construction or validation error.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CkProfileKirError {
    #[error("profile KIR requires a Native consumer")]
    NonNativeConsumer,
    #[error("profile KIR cannot start from invalid KIR: {0}")]
    InvalidKir(String),
    #[error("profile KIR identity space is exhausted")]
    IdentityExhausted,
    #[error("profile KIR site construction failed: {0}")]
    Site(String),
    #[error("profile KIR mapping is invalid: {0}")]
    Mapping(&'static str),
}

impl From<CkProfileError> for CkProfileKirError {
    fn from(error: CkProfileError) -> Self {
        Self::Site(error.to_string())
    }
}

/// Builds the canonical profile topology and generation/use sidecar.
///
/// # Errors
///
/// Rejects invalid KIR, portable consumers, exhausted identities, site
/// collisions, and every malformed canonical site table.
pub fn prepare_ck_profile_kir(
    module: &KirModule,
    mode: CkProfileKirMode,
) -> Result<CkProfileKirPlan, CkProfileKirError> {
    ensure_valid_module(module)?;
    if mode == CkProfileKirMode::Off {
        return Ok(CkProfileKirPlan {
            mode,
            module: module.clone(),
            pre_profile_kir_digest: kir_digest(module),
            site_table_digest: profile_site_table_digest(&[])?,
            sites: Vec::new(),
            annotations: Vec::new(),
            operations: Vec::new(),
            mapping: None,
        });
    }
    if !matches!(
        module.config.consumer,
        KirConsumer::NativeLibrary | KirConsumer::NativeExecutable
    ) {
        return Err(CkProfileKirError::NonNativeConsumer);
    }

    let canonical = split_critical_edges(module)?;
    ensure_valid_module(&canonical)?;
    let pre_profile_kir_digest = kir_digest(&canonical);
    let annotations = discover_profile_annotations(&canonical)?;
    let sites = annotations
        .iter()
        .map(|annotation| annotation.descriptor.clone())
        .collect::<Vec<_>>();
    let site_table_digest = profile_site_table_digest(&sites)?;
    let operations = if mode == CkProfileKirMode::Generate {
        annotations
            .iter()
            .enumerate()
            .map(|(sequence, annotation)| {
                Ok(CkProfileOperation {
                    site_id: annotation.site_id,
                    event: annotation.event.clone(),
                    effect: CkProfileEffect {
                        domain: CkProfileEffectDomain::WorkloadProfile,
                        sequence: u32::try_from(sequence)
                            .map_err(|_| CkProfileKirError::IdentityExhausted)?,
                    },
                })
            })
            .collect::<Result<Vec<_>, CkProfileKirError>>()?
    } else {
        Vec::new()
    };
    let entries = annotations
        .iter()
        .enumerate()
        .map(|(index, annotation)| {
            Ok(CkProfileMappingEntry {
                site_id: annotation.site_id,
                source: annotation.event.clone(),
                target: annotation.event.clone(),
                operation_index: if mode == CkProfileKirMode::Generate {
                    Some(u32::try_from(index).map_err(|_| CkProfileKirError::IdentityExhausted)?)
                } else {
                    None
                },
            })
        })
        .collect::<Result<Vec<_>, CkProfileKirError>>()?;
    let plan = CkProfileKirPlan {
        mode,
        module: canonical,
        pre_profile_kir_digest,
        site_table_digest,
        sites,
        annotations,
        operations,
        mapping: Some(CkProfileMapping {
            pre_profile_kir_digest,
            site_table_digest,
            entries,
        }),
    };
    super::validate_ck_profile_kir_plan(&plan)?;
    Ok(plan)
}

pub(super) fn independently_rebuild_annotations(
    module: &KirModule,
) -> Result<Vec<CkProfileSiteAnnotation>, CkProfileKirError> {
    discover_profile_annotations(module)
}

pub(super) fn kir_digest(module: &KirModule) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"CK-KIR3-PRE-PROFILE\0");
    hasher.update(print_kir_module(module).as_bytes());
    hasher.finalize().into()
}

fn ensure_valid_module(module: &KirModule) -> Result<(), CkProfileKirError> {
    let validation = validate_kir_module(module);
    if validation.errors.is_empty() {
        Ok(())
    } else {
        Err(CkProfileKirError::InvalidKir(
            validation.errors[0].message.clone(),
        ))
    }
}

fn split_critical_edges(module: &KirModule) -> Result<KirModule, CkProfileKirError> {
    if module
        .functions
        .iter()
        .any(|function| !function.vector_regions.is_empty())
    {
        return Err(CkProfileKirError::InvalidKir(
            "profile topology must be frozen before vector regions".to_string(),
        ));
    }
    let mut result = module.clone();
    let mut next_block = module
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .map(|block| block.id.index())
        .max()
        .map_or(Ok(0), |maximum| {
            maximum
                .checked_add(1)
                .ok_or(CkProfileKirError::IdentityExhausted)
        })?;

    for function in &mut result.functions {
        let mut incoming = BTreeMap::<BlockId, u32>::new();
        for block in &function.blocks {
            for target in terminator_successors(&block.terminator) {
                let count = incoming.entry(target).or_default();
                *count = count
                    .checked_add(1)
                    .ok_or(CkProfileKirError::IdentityExhausted)?;
            }
        }
        let mut splits = Vec::new();
        for block in &function.blocks {
            let edges = edge_snapshots(&block.terminator);
            if edges.len() <= 1 {
                continue;
            }
            for (arm, edge) in edges {
                if incoming.get(&edge.target).copied().unwrap_or(0) > 1 {
                    let split_id = BlockId::from_index(next_block);
                    next_block = next_block
                        .checked_add(1)
                        .ok_or(CkProfileKirError::IdentityExhausted)?;
                    splits.push((block.id, arm, split_id, edge));
                }
            }
        }
        for (source, arm, split_id, original) in splits {
            let source_block = function
                .blocks
                .iter_mut()
                .find(|block| block.id == source)
                .ok_or(CkProfileKirError::Mapping(
                    "critical-edge source is missing",
                ))?;
            replace_edge(
                &mut source_block.terminator,
                arm,
                KirEdge {
                    target: split_id,
                    args: Vec::new(),
                    memory_args: Vec::new(),
                },
            )?;
            function.blocks.push(KirBlock {
                id: split_id,
                label: format!(
                    "profile_edge_f{}_b{}_a{}_to{}",
                    function.id.index(),
                    source.index(),
                    arm,
                    original.target.index()
                ),
                params: Vec::new(),
                memory_params: Vec::new(),
                instructions: Vec::new(),
                terminator: KirTerminator::Jump { edge: original },
            });
        }
    }
    Ok(result)
}

fn edge_snapshots(terminator: &KirTerminator) -> Vec<(u8, KirEdge)> {
    match terminator {
        KirTerminator::Return { .. } => Vec::new(),
        KirTerminator::Jump { edge } => vec![(0, edge.clone())],
        KirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => vec![(0, then_edge.clone()), (1, else_edge.clone())],
    }
}

fn replace_edge(
    terminator: &mut KirTerminator,
    arm: u8,
    replacement: KirEdge,
) -> Result<(), CkProfileKirError> {
    match (terminator, arm) {
        (KirTerminator::Jump { edge }, 0) => *edge = replacement,
        (KirTerminator::Branch { then_edge, .. }, 0) => *then_edge = replacement,
        (KirTerminator::Branch { else_edge, .. }, 1) => *else_edge = replacement,
        _ => return Err(CkProfileKirError::Mapping("critical-edge arm is invalid")),
    }
    Ok(())
}

fn discover_profile_annotations(
    module: &KirModule,
) -> Result<Vec<CkProfileSiteAnnotation>, CkProfileKirError> {
    let mut annotations = Vec::new();
    let mut functions = module.functions.iter().collect::<Vec<_>>();
    functions.sort_by_key(|function| function.id);
    for function in functions {
        let function_digest = function_digest(function);
        let Some(entry) = function.blocks.first().map(|block| block.id) else {
            return Err(CkProfileKirError::InvalidKir(format!(
                "function '{}' has no entry block",
                function.name
            )));
        };
        push_annotation(
            &mut annotations,
            function_digest,
            entry.index(),
            CkProfileSiteKind::FunctionEntry,
            CkProfileEvent::FunctionEntry {
                function: function.id,
                block: entry,
            },
        );

        for (from, to) in selected_cfg_edges(function) {
            push_annotation(
                &mut annotations,
                function_digest,
                from.index(),
                CkProfileSiteKind::Edge {
                    from_block: from.index(),
                    to_block: to.index(),
                    reconstructed: false,
                },
                CkProfileEvent::Edge {
                    function: function.id,
                    from,
                    to,
                },
            );
        }

        for (header, latches, exits) in natural_loop_events(function) {
            push_annotation(
                &mut annotations,
                function_digest,
                header.index(),
                CkProfileSiteKind::LoopTripHistogram {
                    loop_identity: header.index(),
                },
                CkProfileEvent::LoopTrip {
                    function: function.id,
                    header,
                    latches,
                    exits,
                },
            );
        }

        let constants = constant_values(function);
        for block in &function.blocks {
            for instruction in &block.instructions {
                match &instruction.kind {
                    KirInstructionKind::SliceLen { .. } => {
                        let Some(value) = instruction.results.first().map(|result| result.value)
                        else {
                            return Err(CkProfileKirError::InvalidKir(
                                "slice length has no result".to_string(),
                            ));
                        };
                        push_annotation(
                            &mut annotations,
                            function_digest,
                            instruction.id.index(),
                            CkProfileSiteKind::SliceLengthHistogram {
                                decision_identity: instruction.id.index(),
                            },
                            CkProfileEvent::SliceLength {
                                function: function.id,
                                block: block.id,
                                instruction: instruction.id,
                                value,
                            },
                        );
                    }
                    KirInstructionKind::Compare { left, right, .. } => {
                        let candidate = match (constants.get(left), constants.get(right)) {
                            (Some(value), None) => Some((*right, *value)),
                            (None, Some(value)) => Some((*left, *value)),
                            _ => None,
                        };
                        if let Some((observed, candidate)) = candidate {
                            push_annotation(
                                &mut annotations,
                                function_digest,
                                instruction.id.index(),
                                CkProfileSiteKind::CandidateConstant {
                                    decision_identity: instruction.id.index(),
                                    candidates: vec![candidate],
                                },
                                CkProfileEvent::CandidateConstant {
                                    function: function.id,
                                    block: block.id,
                                    instruction: instruction.id,
                                    observed,
                                },
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    annotations.sort_by_key(|annotation| annotation.site_id);
    for pair in annotations.windows(2) {
        if pair[0].site_id == pair[1].site_id {
            return Err(CkProfileKirError::Site(
                "profile site identifier collision".to_string(),
            ));
        }
    }
    Ok(annotations)
}

fn push_annotation(
    annotations: &mut Vec<CkProfileSiteAnnotation>,
    function_digest: [u8; 32],
    location: u32,
    kind: CkProfileSiteKind,
    event: CkProfileEvent,
) {
    let id = site_id(function_digest, location, &kind);
    let descriptor = CkProfileSiteDescriptor {
        id,
        function_digest,
        location,
        kind,
    };
    annotations.push(CkProfileSiteAnnotation {
        site_id: id,
        descriptor,
        event,
    });
}

fn function_digest(function: &KirFunction) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"CK-PROFILE-FUNCTION\0");
    hasher.update(print_kir_function(function).as_bytes());
    hasher.finalize().into()
}

fn site_id(function_digest: [u8; 32], location: u32, kind: &CkProfileSiteKind) -> CkProfileSiteId {
    let mut hasher = Sha256::new();
    hasher.update(b"CK-PROFILE-SITE\0");
    hasher.update(function_digest);
    hasher.update(location.to_be_bytes());
    match kind {
        CkProfileSiteKind::FunctionEntry => hasher.update([1]),
        CkProfileSiteKind::Edge {
            from_block,
            to_block,
            reconstructed,
        } => {
            hasher.update([2]);
            hasher.update(from_block.to_be_bytes());
            hasher.update(to_block.to_be_bytes());
            hasher.update([u8::from(*reconstructed)]);
        }
        CkProfileSiteKind::LoopTripHistogram { loop_identity } => {
            hasher.update([3]);
            hasher.update(loop_identity.to_be_bytes());
        }
        CkProfileSiteKind::SliceLengthHistogram { decision_identity } => {
            hasher.update([4]);
            hasher.update(decision_identity.to_be_bytes());
        }
        CkProfileSiteKind::CandidateConstant {
            decision_identity,
            candidates,
        } => {
            hasher.update([5]);
            hasher.update(decision_identity.to_be_bytes());
            for candidate in candidates {
                hasher.update(candidate.to_be_bytes());
            }
        }
    }
    let digest: [u8; 32] = hasher.finalize().into();
    let mut id = [0; 16];
    id.copy_from_slice(&digest[..16]);
    CkProfileSiteId(id)
}

fn selected_cfg_edges(function: &KirFunction) -> Vec<(BlockId, BlockId)> {
    let mut selected = Vec::new();
    let mut visited = BTreeSet::new();
    let mut roots = function
        .blocks
        .iter()
        .map(|block| block.id)
        .collect::<Vec<_>>();
    roots.sort_unstable();
    for root in roots {
        if !visited.insert(root) {
            continue;
        }
        let mut queue = VecDeque::from([root]);
        while let Some(source) = queue.pop_front() {
            let Some(block) = function.blocks.iter().find(|block| block.id == source) else {
                continue;
            };
            let mut successors = terminator_successors(&block.terminator);
            successors.sort_unstable();
            for target in successors {
                if visited.insert(target) {
                    queue.push_back(target);
                } else {
                    selected.push((source, target));
                }
            }
        }
    }
    selected.sort_unstable();
    selected
}

fn natural_loop_events(function: &KirFunction) -> Vec<CkProfileLoopEvent> {
    let dominators = compute_kir_dominators(function);
    let mut by_header = BTreeMap::<BlockId, Vec<BlockId>>::new();
    for block in &function.blocks {
        for target in terminator_successors(&block.terminator) {
            if dominators.dominates(target, block.id) {
                by_header.entry(target).or_default().push(block.id);
            }
        }
    }
    let predecessors = predecessor_map(function);
    by_header
        .into_iter()
        .map(|(header, mut latches)| {
            latches.sort_unstable();
            latches.dedup();
            let mut members = BTreeSet::from([header]);
            let mut worklist = latches.clone();
            while let Some(block) = worklist.pop() {
                if members.insert(block) {
                    worklist.extend(predecessors.get(&block).into_iter().flatten().copied());
                }
            }
            let mut exits = members
                .iter()
                .flat_map(|source| {
                    function
                        .blocks
                        .iter()
                        .find(|block| block.id == *source)
                        .into_iter()
                        .flat_map(|block| terminator_successors(&block.terminator))
                        .filter(|target| !members.contains(target))
                        .map(|target| (*source, target))
                })
                .collect::<Vec<_>>();
            exits.sort_unstable();
            exits.dedup();
            (header, latches, exits)
        })
        .collect()
}

fn predecessor_map(function: &KirFunction) -> BTreeMap<BlockId, Vec<BlockId>> {
    let mut predecessors = BTreeMap::<BlockId, Vec<BlockId>>::new();
    for block in &function.blocks {
        for target in terminator_successors(&block.terminator) {
            predecessors.entry(target).or_default().push(block.id);
        }
    }
    for sources in predecessors.values_mut() {
        sources.sort_unstable();
        sources.dedup();
    }
    predecessors
}

fn constant_values(function: &KirFunction) -> BTreeMap<ValueId, i64> {
    function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .filter_map(|instruction| {
            let KirInstructionKind::ConstInt { value } = &instruction.kind else {
                return None;
            };
            let result = instruction.results.first()?.value;
            value.parse::<i64>().ok().map(|value| (result, value))
        })
        .collect()
}
