use crate::{
    BlockId, CandidateKey, CanonicalLoopDescriptor, FunctionId, KirCostEstimate, KirFunction,
    KirInstructionKind, KirTerminator, LoopCandidateKind, LoopCandidateVariant, LoopId,
    LoopTripCount,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnrollCandidate {
    pub key: CandidateKey,
    pub function: FunctionId,
    pub loop_id: LoopId,
    pub header: BlockId,
    pub factor: u8,
    pub full: bool,
    pub trip_count: u32,
    pub remainder: u8,
    pub body_units: u32,
    pub predicted_cost: KirCostEstimate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnrollFallback {
    pub function: FunctionId,
    pub loop_id: LoopId,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnrollDiscovery {
    pub candidates: Vec<UnrollCandidate>,
    pub fallbacks: Vec<UnrollFallback>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SimpleUnrollShape {
    pub preheader: BlockId,
    pub header: BlockId,
    pub body: BlockId,
    pub exit: BlockId,
}

#[must_use]
pub fn discover_unroll_candidates(
    function: &KirFunction,
    descriptors: &[CanonicalLoopDescriptor],
) -> UnrollDiscovery {
    let mut result = UnrollDiscovery::default();
    for descriptor in descriptors.iter().filter(|item| item.innermost) {
        let Some(shape) = simple_unroll_shape(function, descriptor) else {
            result.fallbacks.push(UnrollFallback {
                function: function.id,
                loop_id: descriptor.id,
                reason: "unsupported-unroll-shape".to_string(),
            });
            continue;
        };
        let Some(trip_count) = exact_trip_count(&descriptor.trip_count) else {
            result.fallbacks.push(UnrollFallback {
                function: function.id,
                loop_id: descriptor.id,
                reason: "non-exact-unroll-trip".to_string(),
            });
            continue;
        };
        let body_units = function
            .blocks
            .iter()
            .find(|block| block.id == shape.body)
            .map_or(0, |block| {
                u32::try_from(block.instructions.len()).unwrap_or(u32::MAX)
            });
        if trip_count <= 8 && body_units <= 16 {
            let cost = unroll_cost(trip_count, body_units, 1, true);
            result.candidates.push(UnrollCandidate {
                key: CandidateKey::LoopFrontier {
                    function: function.id,
                    loop_id: descriptor.id,
                    kind: LoopCandidateKind::FullUnroll,
                    variant: LoopCandidateVariant::Scalar,
                    vf: 1,
                    uf: 1,
                },
                function: function.id,
                loop_id: descriptor.id,
                header: descriptor.header,
                factor: 1,
                full: true,
                trip_count,
                remainder: 0,
                body_units,
                predicted_cost: cost,
            });
        } else if trip_count <= 8 {
            result.fallbacks.push(UnrollFallback {
                function: function.id,
                loop_id: descriptor.id,
                reason: "full-unroll-body-limit".to_string(),
            });
        } else {
            for factor in [2_u8, 4] {
                let remainder = u8::try_from(trip_count % u32::from(factor)).unwrap_or(u8::MAX);
                let cost = unroll_cost(trip_count, body_units, factor, false);
                result.candidates.push(UnrollCandidate {
                    key: CandidateKey::LoopFrontier {
                        function: function.id,
                        loop_id: descriptor.id,
                        kind: LoopCandidateKind::PartialUnroll,
                        variant: LoopCandidateVariant::Scalar,
                        vf: 1,
                        uf: factor,
                    },
                    function: function.id,
                    loop_id: descriptor.id,
                    header: descriptor.header,
                    factor,
                    full: false,
                    trip_count,
                    remainder,
                    body_units,
                    predicted_cost: cost,
                });
            }
        }
    }
    result
        .candidates
        .sort_by(|left, right| left.key.cmp(&right.key));
    result.fallbacks.sort_by(|left, right| {
        (left.function, left.loop_id, &left.reason).cmp(&(
            right.function,
            right.loop_id,
            &right.reason,
        ))
    });
    result
}

#[must_use]
pub(crate) fn simple_unroll_shape(
    function: &KirFunction,
    descriptor: &CanonicalLoopDescriptor,
) -> Option<SimpleUnrollShape> {
    if !descriptor.innermost
        || !descriptor.dedicated_exits
        || !descriptor.lcssa
        || descriptor.blocks.len() != 2
        || descriptor.exits.len() != 1
    {
        return None;
    }
    let preheader = descriptor.preheader?;
    let body = descriptor.latch?;
    if body == descriptor.header || !descriptor.blocks.contains(&body) {
        return None;
    }
    let header_block = function
        .blocks
        .iter()
        .find(|block| block.id == descriptor.header)?;
    let body_block = function.blocks.iter().find(|block| block.id == body)?;
    let preheader_block = function.blocks.iter().find(|block| block.id == preheader)?;
    let KirTerminator::Jump { edge: incoming } = &preheader_block.terminator else {
        return None;
    };
    if incoming.target != descriptor.header {
        return None;
    }
    let KirTerminator::Branch {
        then_edge,
        else_edge,
        ..
    } = &header_block.terminator
    else {
        return None;
    };
    if then_edge.target != body || else_edge.target != descriptor.exits[0] {
        return None;
    }
    let KirTerminator::Jump { edge: backedge } = &body_block.terminator else {
        return None;
    };
    if backedge.target != descriptor.header
        || body_block.instructions.iter().any(|instruction| {
            instruction.memory.is_some()
                || instruction.effect.is_some()
                || matches!(
                    instruction.kind,
                    KirInstructionKind::Binary {
                        op: crate::MirBinaryOp::Mod,
                        ..
                    } | KirInstructionKind::Binary {
                        semantics: crate::KirArithmeticSemantics::Checked,
                        ..
                    } | KirInstructionKind::Unary {
                        semantics: crate::KirArithmeticSemantics::Checked,
                        ..
                    }
                )
                || matches!(
                    instruction.kind,
                    KirInstructionKind::Binary {
                        op: crate::MirBinaryOp::Div,
                        semantics,
                        ..
                    } if semantics != crate::KirArithmeticSemantics::StrictFloat
                )
                || instruction
                    .results
                    .iter()
                    .any(|result| result.type_node.as_scalar().is_none())
                || !matches!(
                    instruction.kind,
                    KirInstructionKind::Undef { .. }
                        | KirInstructionKind::ConstInt { .. }
                        | KirInstructionKind::ConstFloat { .. }
                        | KirInstructionKind::ConstBool { .. }
                        | KirInstructionKind::Copy { .. }
                        | KirInstructionKind::Binary { .. }
                        | KirInstructionKind::Unary { .. }
                        | KirInstructionKind::Compare { .. }
                        | KirInstructionKind::Cast { .. }
                )
        })
    {
        return None;
    }
    Some(SimpleUnrollShape {
        preheader,
        header: descriptor.header,
        body,
        exit: descriptor.exits[0],
    })
}

#[must_use]
pub(crate) fn unroll_cost(
    trip_count: u32,
    body_units: u32,
    factor: u8,
    full: bool,
) -> KirCostEstimate {
    let scalar = if trip_count == 0 {
        2
    } else {
        trip_count
            .saturating_mul(body_units.saturating_add(2))
            .saturating_add(1)
    };
    let transformed = if full {
        trip_count.saturating_mul(body_units)
    } else {
        let groups = trip_count / u32::from(factor);
        trip_count
            .saturating_mul(body_units)
            .saturating_add(groups.saturating_mul(2))
            .saturating_add(2)
    };
    KirCostEstimate::new(scalar, transformed, 0, 0)
}

fn exact_trip_count(trip_count: &LoopTripCount) -> Option<u32> {
    match trip_count {
        LoopTripCount::Zero => Some(0),
        LoopTripCount::Exact { iterations } => u32::try_from(*iterations).ok(),
        LoopTripCount::Runtime { .. } | LoopTripCount::Unknown => None,
    }
}
