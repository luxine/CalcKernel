//! Conservative late physical value placement over the backend's typed Wasm IR.

use std::collections::{BTreeMap, HashMap, HashSet};

use super::final_ir::{
    FinalInstruction, FinalInstructionKind, FinalLocal, FinalWasmFunction, FinalWasmModule,
    WasmValueType,
};

const MAX_FUNCTION_LINES: usize = 16_384;
const MAX_FUNCTION_BYTES: usize = 1_048_576;
const MAX_FUNCTION_LOCALS: usize = 4_096;
const MAX_STACKIFY_CANDIDATES: usize = 512;
const MAX_STACKIFY_SCAN_STEPS: usize = 1_000_000;
const MAX_CROSSING_CHECKS: usize = 250_000;

/// Applies the O3 local placement pass to the owned typed module. Functions
/// outside the pass's scalar envelope or work limits remain unchanged.
pub(super) fn optimize_final_module(module: &mut FinalWasmModule, opt_level: u8) {
    if opt_level < 3 {
        return;
    }
    for function in &mut module.functions {
        if let Some(optimized) = optimize_final_function(function) {
            *function = optimized;
        }
    }
}

#[derive(Debug, Clone)]
struct PlacementLine {
    kind: LineKind,
    removed: bool,
    typed_instruction: Option<FinalInstruction>,
}

#[derive(Debug, Clone)]
enum LineKind {
    Local { name: String, type_name: String },
    Instruction(InstructionKind),
}

#[derive(Debug, Clone)]
enum InstructionKind {
    Get(String),
    Set(String),
    Tee(String),
    Stack { pops: u8, pushes: u8 },
    Fence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValueType {
    I32,
    I64,
    F64,
    V128,
}

impl ValueType {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "i32" => Some(Self::I32),
            "i64" => Some(Self::I64),
            "f64" => Some(Self::F64),
            "v128" => Some(Self::V128),
            _ => None,
        }
    }
}

#[derive(Debug, Default, Clone)]
struct LocalReferences {
    reads: Vec<usize>,
    writes: Vec<usize>,
    tees: usize,
}

#[derive(Debug, Clone)]
struct Candidate {
    name: String,
    type_name: String,
    segment: usize,
    start: usize,
    end: usize,
}

fn optimize_final_function(function: &FinalWasmFunction) -> Option<FinalWasmFunction> {
    if !within_typed_function_limits(function)
        || function.results.iter().any(|ty| value_type(*ty).is_none())
        || !is_wasm_identifier(&function.name)
        || function
            .export_name
            .as_ref()
            .is_some_and(|name| name != &function.name || !is_wasm_identifier(name))
    {
        return None;
    }

    let mut declarations = BTreeMap::<String, (ValueType, bool)>::new();
    for param in &function.params {
        if !is_wasm_identifier(&param.name) {
            return None;
        }
        let ty = value_type(param.ty)?;
        if declarations
            .insert(param.name.clone(), (ty, true))
            .is_some()
        {
            return None;
        }
    }
    let mut lines =
        Vec::<PlacementLine>::with_capacity(function.locals.len() + function.body.len());
    for local in &function.locals {
        if !is_wasm_identifier(&local.name) {
            return None;
        }
        let ty = value_type(local.ty)?;
        if declarations
            .insert(local.name.clone(), (ty, false))
            .is_some()
        {
            return None;
        }
        lines.push(PlacementLine {
            kind: LineKind::Local {
                name: local.name.clone(),
                type_name: local.ty.wat().to_string(),
            },
            removed: false,
            typed_instruction: None,
        });
    }
    if declarations.len() > MAX_FUNCTION_LOCALS {
        return None;
    }

    for instruction in &function.body {
        if !final_instruction_identifiers_are_valid(&instruction.kind) {
            return None;
        }
        let kind = classify_final_instruction(&instruction.kind)?;
        lines.push(PlacementLine {
            kind: LineKind::Instruction(kind),
            removed: false,
            typed_instruction: Some(instruction.clone()),
        });
    }
    for line in &lines {
        if let LineKind::Instruction(
            InstructionKind::Get(name) | InstructionKind::Set(name) | InstructionKind::Tee(name),
        ) = &line.kind
            && !declarations.contains_key(name)
        {
            return None;
        }
    }

    convert_adjacent_reads_to_tee(&mut lines, &declarations);
    stackify_single_use_values(&mut lines, &declarations);
    coalesce_disjoint_locals(&mut lines, &declarations);
    remove_unreferenced_locals(&mut lines);

    let mut optimized = function.clone();
    optimized.locals = lines
        .iter()
        .filter_map(|line| {
            if line.removed {
                return None;
            }
            let LineKind::Local { name, type_name } = &line.kind else {
                return None;
            };
            Some(FinalLocal {
                name: name.clone(),
                ty: match ValueType::parse(type_name)? {
                    ValueType::I32 => WasmValueType::I32,
                    ValueType::I64 => WasmValueType::I64,
                    ValueType::F64 => WasmValueType::F64,
                    ValueType::V128 => WasmValueType::V128,
                },
            })
        })
        .collect();
    optimized.body = lines
        .into_iter()
        .filter_map(|line| {
            if line.removed || !matches!(line.kind, LineKind::Instruction(_)) {
                return None;
            }
            line.typed_instruction
        })
        .collect();
    Some(optimized)
}

fn value_type(ty: WasmValueType) -> Option<ValueType> {
    ValueType::parse(ty.wat())
}

fn is_wasm_identifier(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
}

fn final_instruction_identifiers_are_valid(instruction: &FinalInstructionKind) -> bool {
    match instruction {
        FinalInstructionKind::LocalGet(name)
        | FinalInstructionKind::LocalSet(name)
        | FinalInstructionKind::LocalTee(name)
        | FinalInstructionKind::Call(name)
        | FinalInstructionKind::Br(name)
        | FinalInstructionKind::BrIf(name) => is_wasm_identifier(name),
        FinalInstructionKind::Block { label, .. }
        | FinalInstructionKind::Loop { label, .. }
        | FinalInstructionKind::If { label, .. } => {
            label.as_ref().is_none_or(|name| is_wasm_identifier(name))
        }
        FinalInstructionKind::BrTable { targets, default } => {
            is_wasm_identifier(default) && targets.iter().all(|name| is_wasm_identifier(name))
        }
        _ => true,
    }
}

fn within_typed_function_limits(function: &FinalWasmFunction) -> bool {
    let line_count = 2usize
        .saturating_add(function.params.len())
        .saturating_add(usize::from(!function.results.is_empty()))
        .saturating_add(function.locals.len())
        .saturating_add(function.body.len());
    if line_count > MAX_FUNCTION_LINES
        || function.params.len().saturating_add(function.locals.len()) > MAX_FUNCTION_LOCALS
    {
        return false;
    }

    let mut estimated_bytes = 32usize.saturating_add(function.name.len()).saturating_add(
        function
            .export_name
            .as_ref()
            .map_or(0, |name| name.len() + 16),
    );
    for local in function.params.iter().chain(&function.locals) {
        estimated_bytes = estimated_bytes
            .saturating_add(local.name.len())
            .saturating_add(local.ty.wat().len())
            .saturating_add(16);
    }
    for ty in &function.results {
        estimated_bytes = estimated_bytes.saturating_add(ty.wat().len() + 1);
    }
    for instruction in &function.body {
        estimated_bytes = estimated_bytes.saturating_add(estimated_instruction_bytes(instruction));
        if estimated_bytes > MAX_FUNCTION_BYTES {
            return false;
        }
    }
    estimated_bytes <= MAX_FUNCTION_BYTES
}

fn estimated_instruction_bytes(instruction: &FinalInstruction) -> usize {
    let operands = match &instruction.kind {
        FinalInstructionKind::Simple(opcode) => opcode.wat().len(),
        FinalInstructionKind::I32Const(value) => value.to_string().len() + 10,
        FinalInstructionKind::I64Const(value) => value.to_string().len() + 10,
        FinalInstructionKind::F64Const(_) => 40,
        FinalInstructionKind::V128Const(bytes) => {
            16usize.saturating_add(bytes.len().saturating_mul(5))
        }
        FinalInstructionKind::LocalGet(name)
        | FinalInstructionKind::LocalSet(name)
        | FinalInstructionKind::LocalTee(name)
        | FinalInstructionKind::Call(name)
        | FinalInstructionKind::Br(name)
        | FinalInstructionKind::BrIf(name) => name.len() + 12,
        FinalInstructionKind::Block { label, results }
        | FinalInstructionKind::Loop { label, results }
        | FinalInstructionKind::If { label, results } => {
            label.as_ref().map_or(0, String::len)
                + results.iter().map(|ty| ty.wat().len() + 8).sum::<usize>()
                + 16
        }
        FinalInstructionKind::Else
        | FinalInstructionKind::End
        | FinalInstructionKind::MemorySize
        | FinalInstructionKind::MemoryGrow
        | FinalInstructionKind::MemoryCopy
        | FinalInstructionKind::MemoryFill => 16,
        FinalInstructionKind::BrTable { targets, default } => {
            targets.iter().map(String::len).sum::<usize>() + default.len() + 16
        }
        FinalInstructionKind::Load { opcode, offset, .. }
        | FinalInstructionKind::Store { opcode, offset, .. } => {
            opcode.wat().len() + offset.to_string().len() + 24
        }
        FinalInstructionKind::Lane { opcode, .. } => opcode.wat().len() + 12,
        FinalInstructionKind::Shuffle(_) => 96,
    };
    instruction
        .indent
        .saturating_add(operands)
        .saturating_add(8)
}

fn classify_final_instruction(instruction: &FinalInstructionKind) -> Option<InstructionKind> {
    Some(match instruction {
        FinalInstructionKind::Simple(opcode) => classify_final_simple(opcode.wat())?,
        FinalInstructionKind::I32Const(_)
        | FinalInstructionKind::I64Const(_)
        | FinalInstructionKind::F64Const(_) => InstructionKind::Stack { pops: 0, pushes: 1 },
        FinalInstructionKind::LocalGet(name) => InstructionKind::Get(name.clone()),
        FinalInstructionKind::LocalSet(name) => InstructionKind::Set(name.clone()),
        FinalInstructionKind::LocalTee(name) => InstructionKind::Tee(name.clone()),
        FinalInstructionKind::Call(_)
        | FinalInstructionKind::Block { .. }
        | FinalInstructionKind::Loop { .. }
        | FinalInstructionKind::If { .. }
        | FinalInstructionKind::Else
        | FinalInstructionKind::End
        | FinalInstructionKind::Br(_)
        | FinalInstructionKind::BrIf(_)
        | FinalInstructionKind::BrTable { .. }
        | FinalInstructionKind::Load { .. }
        | FinalInstructionKind::Store { .. }
        | FinalInstructionKind::MemorySize
        | FinalInstructionKind::MemoryGrow
        | FinalInstructionKind::MemoryCopy
        | FinalInstructionKind::MemoryFill => InstructionKind::Fence,
        FinalInstructionKind::V128Const(_) => InstructionKind::Stack { pops: 0, pushes: 1 },
        FinalInstructionKind::Lane { opcode, .. } => {
            let pops = if opcode.wat().ends_with(".replace_lane") {
                2
            } else {
                1
            };
            InstructionKind::Stack { pops, pushes: 1 }
        }
        FinalInstructionKind::Shuffle(_) => InstructionKind::Stack { pops: 2, pushes: 1 },
    })
}

fn classify_final_simple(opcode: &str) -> Option<InstructionKind> {
    if let Some((pops, pushes)) = classify_simd_stack_effect(opcode) {
        return Some(InstructionKind::Stack { pops, pushes });
    }
    match opcode {
        "drop" => Some(InstructionKind::Stack { pops: 1, pushes: 0 }),
        "i32.eqz" | "i64.eqz" | "f64.neg" | "f64.convert_i32_s" | "f64.convert_i32_u" => {
            Some(InstructionKind::Stack { pops: 1, pushes: 1 })
        }
        "i32.add" | "i32.sub" | "i32.mul" | "i32.and" | "i32.or" | "i32.xor" | "i32.shl"
        | "i32.shr_s" | "i32.shr_u" | "i32.rotl" | "i32.rotr" | "i64.add" | "i64.sub"
        | "i64.mul" | "i64.and" | "i64.or" | "i64.xor" | "i64.shl" | "i64.shr_s" | "i64.shr_u"
        | "i64.rotl" | "i64.rotr" | "f64.add" | "f64.sub" | "f64.mul" | "f64.div" | "f64.min"
        | "f64.max" | "f64.copysign" | "i32.eq" | "i32.ne" | "i32.lt_s" | "i32.lt_u"
        | "i32.gt_s" | "i32.gt_u" | "i32.le_s" | "i32.le_u" | "i32.ge_s" | "i32.ge_u"
        | "i64.eq" | "i64.ne" | "i64.lt_s" | "i64.lt_u" | "i64.gt_s" | "i64.gt_u" | "i64.le_s"
        | "i64.le_u" | "i64.ge_s" | "i64.ge_u" | "f64.eq" | "f64.ne" | "f64.lt" | "f64.gt"
        | "f64.le" | "f64.ge" => Some(InstructionKind::Stack { pops: 2, pushes: 1 }),
        // These instructions trap and therefore remain effect boundaries.
        "i32.div_s" | "i32.div_u" | "i32.rem_s" | "i32.rem_u" | "i64.div_s" | "i64.div_u"
        | "i64.rem_s" | "i64.rem_u" | "unreachable" | "return" => Some(InstructionKind::Fence),
        _ => None,
    }
}

/// Returns operand-stack arity for every SIMD `SimpleOpcode` emitted by this
/// backend. All SIMD operations produce one value; some reductions produce a
/// scalar, which still has the same stack height. Lane immediates are handled
/// separately because extracts and replacements have different arities.
fn classify_simd_stack_effect(opcode: &str) -> Option<(u8, u8)> {
    let (prefix, operation) = opcode.split_once('.')?;
    if !matches!(
        prefix,
        "v128" | "i8x16" | "i16x8" | "i32x4" | "i64x2" | "f32x4" | "f64x2"
    ) {
        return None;
    }

    let arity = match (prefix, operation) {
        ("v128", "bitselect") => (3, 1),
        ("v128", "not" | "any_true") => (1, 1),
        ("v128", "and" | "andnot" | "or" | "xor") => (2, 1),
        (_, "splat") => (1, 1),
        (_, "abs" | "neg" | "popcnt" | "all_true" | "bitmask") => (1, 1),
        ("f32x4", "ceil" | "floor" | "trunc" | "nearest" | "sqrt") => (1, 1),
        ("f64x2", "sqrt") => (1, 1),
        ("f64x2", "convert_low_i32x4_s" | "convert_low_i32x4_u") => (1, 1),
        (_, "swizzle" | "q15mulr_sat_s") => (2, 1),
        (
            _,
            "add" | "sub" | "mul" | "div" | "min" | "max" | "pmin" | "pmax" | "eq" | "ne" | "lt"
            | "lt_s" | "lt_u" | "gt" | "gt_s" | "gt_u" | "le" | "le_s" | "le_u" | "ge" | "ge_s"
            | "ge_u",
        ) => (2, 1),
        _ => return None,
    };
    Some(arity)
}

fn is_pinned_name(name: &str, declarations: &BTreeMap<String, (ValueType, bool)>) -> bool {
    declarations.get(name).is_none_or(|(_, is_param)| *is_param)
        || name.starts_with("edge_")
        || name.starts_with("ik_")
        || is_slice_pair_component(name)
}

fn is_slice_pair_component(name: &str) -> bool {
    if name.ends_with("_data") || name.ends_with("_len") {
        return true;
    }
    ["_data_", "_len_"].iter().any(|marker| {
        name.rsplit_once(marker).is_some_and(|(_, suffix)| {
            !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit())
        })
    })
}

fn collect_references(lines: &[PlacementLine]) -> HashMap<String, LocalReferences> {
    let mut references = HashMap::<String, LocalReferences>::new();
    for (index, line) in lines.iter().enumerate() {
        if line.removed {
            continue;
        }
        let LineKind::Instruction(kind) = &line.kind else {
            continue;
        };
        match kind {
            InstructionKind::Get(name) => references
                .entry(name.clone())
                .or_default()
                .reads
                .push(index),
            InstructionKind::Set(name) => references
                .entry(name.clone())
                .or_default()
                .writes
                .push(index),
            InstructionKind::Tee(name) => {
                let reference = references.entry(name.clone()).or_default();
                reference.reads.push(index);
                reference.writes.push(index);
                reference.tees += 1;
            }
            InstructionKind::Stack { .. } | InstructionKind::Fence => {}
        }
    }
    references
}

fn convert_adjacent_reads_to_tee(
    lines: &mut [PlacementLine],
    declarations: &BTreeMap<String, (ValueType, bool)>,
) {
    let references = collect_references(lines);
    let mut index = 0;
    while index + 1 < lines.len() {
        let next_index = index + 1;
        if lines[index].removed || lines[next_index].removed {
            index += 1;
            continue;
        }
        let pair_name = match (&lines[index].kind, &lines[next_index].kind) {
            (
                LineKind::Instruction(InstructionKind::Set(set)),
                LineKind::Instruction(InstructionKind::Get(get)),
            ) if set == get => Some(set.clone()),
            _ => None,
        };
        let Some(name) = pair_name else {
            index += 1;
            continue;
        };
        if is_pinned_name(&name, declarations)
            || references
                .get(&name)
                .is_none_or(|reference| reference.reads.len() < 2)
        {
            index += 1;
            continue;
        }
        lines[index].kind = LineKind::Instruction(InstructionKind::Tee(name));
        sync_typed_instruction(&mut lines[index]);
        lines[next_index].removed = true;
        index += 2;
    }
}

fn stackify_single_use_values(
    lines: &mut [PlacementLine],
    declarations: &BTreeMap<String, (ValueType, bool)>,
) {
    let references = collect_references(lines);
    let mut candidates = Vec::<Candidate>::new();
    let segments = segment_ids(lines);
    for (name, references) in references {
        if references.writes.len() != 1
            || references.reads.len() != 1
            || references.tees != 0
            || is_pinned_name(&name, declarations)
        {
            continue;
        }
        let set_index = references.writes[0];
        let get_index = references.reads[0];
        if set_index >= get_index {
            continue;
        }
        let (Some(set_segment), Some(get_segment)) = (segments[set_index], segments[get_index])
        else {
            continue;
        };
        if set_segment != get_segment {
            continue;
        }
        let Some((_, false)) = declarations.get(&name) else {
            continue;
        };
        let type_name = match lines.iter().find_map(|line| match &line.kind {
            LineKind::Local {
                name: local,
                type_name,
            } if local == &name => Some(type_name.clone()),
            _ => None,
        }) {
            Some(type_name) => type_name,
            None => continue,
        };
        candidates.push(Candidate {
            name,
            type_name,
            segment: set_segment,
            start: set_index,
            end: get_index,
        });
    }
    if candidates.len() > MAX_STACKIFY_CANDIDATES {
        return;
    }
    // Stable outer-before-inner order makes nested stackification independent
    // of the HashMap traversal above. It also lets one outer interval prove
    // safety against the original inner local roundtrip before either pair is
    // removed.
    candidates.sort_by(|a, b| {
        (a.segment, a.start, std::cmp::Reverse(a.end), &a.name).cmp(&(
            b.segment,
            b.start,
            std::cmp::Reverse(b.end),
            &b.name,
        ))
    });

    // Test every candidate against the original post-tee sequence before
    // removing any pair. A candidate whose interval is unbalanced cannot be
    // stackified, and therefore must not prevent a safe interval from being
    // considered merely because their lexical ranges cross.
    let mut scan_steps = 0;
    let mut safe_candidates = Vec::new();
    for candidate in candidates {
        let mut stack_depth = 0i32;
        let mut safe = true;
        for line in &lines[candidate.start + 1..candidate.end] {
            if line.removed {
                continue;
            }
            scan_steps += 1;
            if scan_steps > MAX_STACKIFY_SCAN_STEPS {
                return;
            }
            let LineKind::Instruction(kind) = &line.kind else {
                safe = false;
                break;
            };
            if matches!(kind, InstructionKind::Fence) {
                safe = false;
                break;
            }
            let (pops, pushes) = match kind {
                InstructionKind::Get(_) => (0, 1),
                InstructionKind::Set(_) => (1, 0),
                InstructionKind::Tee(_) => (1, 1),
                InstructionKind::Stack { pops, pushes } => (*pops, *pushes),
                InstructionKind::Fence => unreachable!(),
            };
            if stack_depth < i32::from(pops) {
                safe = false;
                break;
            }
            stack_depth = stack_depth - i32::from(pops) + i32::from(pushes);
        }
        if safe && stack_depth == 0 {
            safe_candidates.push(candidate);
        }
    }

    let mut crossing = HashSet::<String>::new();
    let mut crossing_checks = 0;
    for left in 0..safe_candidates.len() {
        for right in left + 1..safe_candidates.len() {
            crossing_checks += 1;
            if crossing_checks > MAX_CROSSING_CHECKS {
                return;
            }
            let a = &safe_candidates[left];
            let b = &safe_candidates[right];
            if a.segment == b.segment
                && ((a.start < b.start && b.start < a.end && a.end < b.end)
                    || (b.start < a.start && a.start < b.end && b.end < a.end))
            {
                crossing.insert(a.name.clone());
                crossing.insert(b.name.clone());
            }
        }
    }

    for candidate in safe_candidates {
        if crossing.contains(&candidate.name) {
            continue;
        }
        lines[candidate.start].removed = true;
        lines[candidate.end].removed = true;
    }
}

fn segment_ids(lines: &[PlacementLine]) -> Vec<Option<usize>> {
    let mut segment = 0usize;
    let mut active = false;
    let mut ids = vec![None; lines.len()];
    for (index, line) in lines.iter().enumerate() {
        if line.removed {
            continue;
        }
        let LineKind::Instruction(kind) = &line.kind else {
            continue;
        };
        if matches!(kind, InstructionKind::Fence) {
            active = false;
            segment += 1;
        } else {
            if !active {
                segment += 1;
                active = true;
            }
            ids[index] = Some(segment);
        }
    }
    ids
}

fn coalesce_disjoint_locals(
    lines: &mut [PlacementLine],
    declarations: &BTreeMap<String, (ValueType, bool)>,
) {
    let references = collect_references(lines);
    let segments = segment_ids(lines);
    let mut candidates = Vec::<Candidate>::new();
    for (name, reference) in references {
        if reference.writes.len() != 1 || is_pinned_name(&name, declarations) {
            continue;
        }
        let start = reference.writes[0];
        // `local.tee`'s input is on the operand stack. It writes the local but
        // does not read the previous local value, so exclude that synthetic
        // read when computing the local's interval.
        let reads = reference
            .reads
            .iter()
            .copied()
            .filter(|index| {
                !matches!(
                    &lines[*index].kind,
                    LineKind::Instruction(InstructionKind::Tee(tee_name)) if tee_name == &name
                )
            })
            .collect::<Vec<_>>();
        if reads.is_empty() {
            continue;
        }
        let end = *reads.iter().max().expect("nonempty reads checked above");
        if reads.iter().any(|read| *read <= start) {
            continue;
        }
        let (Some(start_segment), Some(end_segment)) = (segments[start], segments[end]) else {
            continue;
        };
        if start_segment != end_segment
            || reads
                .iter()
                .any(|read| segments[*read] != Some(start_segment))
        {
            continue;
        }
        let Some((type_name, false)) = declarations
            .get(&name)
            .map(|(type_name, is_param)| (type_name, *is_param))
        else {
            continue;
        };
        candidates.push(Candidate {
            name,
            type_name: match type_name {
                ValueType::I32 => "i32".to_string(),
                ValueType::I64 => "i64".to_string(),
                ValueType::F64 => "f64".to_string(),
                ValueType::V128 => "v128".to_string(),
            },
            segment: start_segment,
            start,
            end,
        });
    }
    candidates.sort_by(|a, b| {
        (a.segment, &a.type_name, a.start, a.end, &a.name).cmp(&(
            b.segment,
            &b.type_name,
            b.start,
            b.end,
            &b.name,
        ))
    });

    let mut active_slots = BTreeMap::<(usize, String), Vec<(usize, String)>>::new();
    let mut aliases = HashMap::<String, String>::new();
    for candidate in candidates {
        let slots = active_slots
            .entry((candidate.segment, candidate.type_name.clone()))
            .or_default();
        if let Some((slot_end, slot_name)) =
            slots.iter_mut().find(|(end, _)| *end < candidate.start)
        {
            *slot_end = candidate.end;
            aliases.insert(candidate.name, slot_name.clone());
        } else {
            slots.push((candidate.end, candidate.name.clone()));
        }
    }
    if aliases.is_empty() {
        return;
    }

    for line in lines.iter_mut() {
        match &mut line.kind {
            LineKind::Instruction(
                InstructionKind::Get(name)
                | InstructionKind::Set(name)
                | InstructionKind::Tee(name),
            ) => {
                if let Some(alias) = aliases.get(name) {
                    *name = alias.clone();
                }
            }
            LineKind::Local { name, .. } => {
                if let Some(alias) = aliases.get(name) {
                    line.removed = true;
                    *name = alias.clone();
                }
            }
            LineKind::Instruction(_) => {}
        }
        sync_typed_instruction(line);
    }
}

fn sync_typed_instruction(line: &mut PlacementLine) {
    let (Some(instruction), LineKind::Instruction(kind)) =
        (&mut line.typed_instruction, &line.kind)
    else {
        return;
    };
    match kind {
        InstructionKind::Get(name) => {
            instruction.kind = FinalInstructionKind::LocalGet(name.clone())
        }
        InstructionKind::Set(name) => {
            instruction.kind = FinalInstructionKind::LocalSet(name.clone())
        }
        InstructionKind::Tee(name) => {
            instruction.kind = FinalInstructionKind::LocalTee(name.clone())
        }
        InstructionKind::Stack { .. } | InstructionKind::Fence => {}
    }
}

fn remove_unreferenced_locals(lines: &mut [PlacementLine]) {
    let references = collect_references(lines);
    for line in lines {
        if let LineKind::Local { name, .. } = &line.kind
            && !references.contains_key(name)
        {
            line.removed = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_FUNCTION_BYTES, optimize_final_module};
    use crate::backend::wasm::final_ir::{
        FinalInstruction, FinalInstructionKind, FinalLocal, FinalWasmFunction, FinalWasmModule,
        MemoryOpcode, SimpleOpcode, WasmValueType,
    };

    fn instruction(kind: FinalInstructionKind) -> FinalInstruction {
        FinalInstruction { indent: 4, kind }
    }

    fn function(
        locals: &[(&str, WasmValueType)],
        body: Vec<FinalInstruction>,
    ) -> FinalWasmFunction {
        FinalWasmFunction {
            name: "f".into(),
            export_name: None,
            params: Vec::new(),
            results: Vec::new(),
            locals: locals
                .iter()
                .map(|(name, ty)| FinalLocal {
                    name: (*name).into(),
                    ty: *ty,
                })
                .collect(),
            body,
        }
    }

    fn module(function: FinalWasmFunction) -> FinalWasmModule {
        FinalWasmModule {
            functions: vec![function],
            memory_minimum: 1,
            target_metadata: None,
        }
    }

    fn optimize(function: FinalWasmFunction, level: u8) -> FinalWasmFunction {
        let mut module = module(function);
        optimize_final_module(&mut module, level);
        module.functions.remove(0)
    }

    fn validate_wat(module: &FinalWasmModule) {
        let wasm = wat::parse_str(module.to_wat()).expect("placement output WAT parses");
        let features = wasmparser::WasmFeatures::MVP
            | wasmparser::WasmFeatures::MULTI_VALUE
            | wasmparser::WasmFeatures::BULK_MEMORY
            | wasmparser::WasmFeatures::SIMD;
        wasmparser::Validator::new_with_features(features)
            .validate_all(&wasm)
            .expect("placement output is well-typed WebAssembly");
    }

    #[test]
    fn stackifies_a_single_use_local_roundtrip() {
        let optimized = optimize(
            function(
                &[("v0", WasmValueType::I32), ("v1", WasmValueType::I32)],
                vec![
                    instruction(FinalInstructionKind::I32Const(3)),
                    instruction(FinalInstructionKind::LocalSet("v0".into())),
                    instruction(FinalInstructionKind::LocalGet("v0".into())),
                    instruction(FinalInstructionKind::I32Const(1)),
                    instruction(FinalInstructionKind::Simple(SimpleOpcode::I32Add)),
                    instruction(FinalInstructionKind::LocalSet("v1".into())),
                ],
            ),
            3,
        );

        assert!(optimized.locals.iter().all(|local| local.name != "v0"));
        assert!(optimized.body.iter().all(|instruction| !matches!(
            &instruction.kind,
            FinalInstructionKind::LocalGet(name) | FinalInstructionKind::LocalSet(name) if name == "v0"
        )));
        assert!(optimized.body.iter().any(|instruction| matches!(
            instruction.kind,
            FinalInstructionKind::Simple(SimpleOpcode::I32Add)
        )));
    }

    #[test]
    fn places_mixed_scalar_and_vector_locals_across_lane_and_shuffle_ops() {
        use crate::backend::wasm::final_ir::LaneOpcode;

        let mut original = function(
            &[
                ("scalar", WasmValueType::I32),
                ("vector", WasmValueType::V128),
            ],
            vec![
                instruction(FinalInstructionKind::I32Const(17)),
                instruction(FinalInstructionKind::LocalSet("scalar".into())),
                instruction(FinalInstructionKind::V128Const(vec![0; 16])),
                instruction(FinalInstructionKind::Lane {
                    opcode: LaneOpcode::I32x4ExtractLane,
                    lane: 2,
                }),
                instruction(FinalInstructionKind::Simple(SimpleOpcode::Drop)),
                instruction(FinalInstructionKind::V128Const(vec![1; 16])),
                instruction(FinalInstructionKind::V128Const(vec![2; 16])),
                instruction(FinalInstructionKind::Shuffle(std::array::from_fn(|i| {
                    i as u8
                }))),
                instruction(FinalInstructionKind::Simple(SimpleOpcode::Drop)),
                instruction(FinalInstructionKind::V128Const(vec![6; 16])),
                instruction(FinalInstructionKind::Simple(SimpleOpcode::V128Not)),
                instruction(FinalInstructionKind::Simple(SimpleOpcode::Drop)),
                instruction(FinalInstructionKind::V128Const(vec![11; 16])),
                instruction(FinalInstructionKind::I32Const(11)),
                instruction(FinalInstructionKind::Lane {
                    opcode: LaneOpcode::I32x4ReplaceLane,
                    lane: 3,
                }),
                instruction(FinalInstructionKind::Simple(SimpleOpcode::Drop)),
                instruction(FinalInstructionKind::V128Const(vec![7; 16])),
                instruction(FinalInstructionKind::V128Const(vec![8; 16])),
                instruction(FinalInstructionKind::V128Const(vec![9; 16])),
                instruction(FinalInstructionKind::Simple(SimpleOpcode::V128Bitselect)),
                instruction(FinalInstructionKind::Simple(SimpleOpcode::Drop)),
                instruction(FinalInstructionKind::V128Const(vec![12; 16])),
                instruction(FinalInstructionKind::V128Const(vec![13; 16])),
                instruction(FinalInstructionKind::Simple(SimpleOpcode::V128And)),
                instruction(FinalInstructionKind::Simple(SimpleOpcode::Drop)),
                instruction(FinalInstructionKind::I32Const(10)),
                instruction(FinalInstructionKind::Simple(SimpleOpcode::I32x4Splat)),
                instruction(FinalInstructionKind::Simple(SimpleOpcode::V128AnyTrue)),
                instruction(FinalInstructionKind::Simple(SimpleOpcode::Drop)),
                instruction(FinalInstructionKind::V128Const(vec![3; 16])),
                instruction(FinalInstructionKind::LocalSet("vector".into())),
                instruction(FinalInstructionKind::V128Const(vec![4; 16])),
                instruction(FinalInstructionKind::V128Const(vec![5; 16])),
                instruction(FinalInstructionKind::Shuffle(std::array::from_fn(|i| {
                    i as u8
                }))),
                instruction(FinalInstructionKind::Simple(SimpleOpcode::Drop)),
                instruction(FinalInstructionKind::LocalGet("vector".into())),
                instruction(FinalInstructionKind::Simple(SimpleOpcode::Drop)),
                instruction(FinalInstructionKind::LocalGet("scalar".into())),
            ],
        );
        original.results = vec![WasmValueType::I32];
        let optimized = optimize(original.clone(), 3);

        assert!(optimized.locals.is_empty());
        assert!(optimized.body.iter().all(|instruction| !matches!(
            &instruction.kind,
            FinalInstructionKind::LocalGet(name) | FinalInstructionKind::LocalSet(name)
                if name == "scalar" || name == "vector"
        )));
        assert!(optimized.body.iter().any(|instruction| matches!(
            instruction.kind,
            FinalInstructionKind::Lane {
                opcode: LaneOpcode::I32x4ExtractLane,
                lane: 2
            }
        )));
        assert_eq!(
            optimized
                .body
                .iter()
                .filter(|instruction| matches!(instruction.kind, FinalInstructionKind::Shuffle(_)))
                .count(),
            2
        );
        assert!(
            optimized
                .body
                .iter()
                .any(|instruction| matches!(instruction.kind, FinalInstructionKind::I32Const(17)))
        );
        assert_eq!(optimized.results, [WasmValueType::I32]);
        validate_wat(&module(original));
        validate_wat(&module(optimized.clone()));
    }

    #[test]
    fn keeps_loop_spanning_locals_and_edge_copies_materialized() {
        let mut original = FinalWasmFunction {
            name: "f".into(),
            export_name: None,
            params: vec![FinalLocal {
                name: "p".into(),
                ty: WasmValueType::I32,
            }],
            results: Vec::new(),
            locals: ["ordinary", "edge_1_2_0_0"]
                .into_iter()
                .map(|name| FinalLocal {
                    name: name.into(),
                    ty: WasmValueType::I32,
                })
                .collect(),
            body: vec![
                instruction(FinalInstructionKind::LocalGet("p".into())),
                instruction(FinalInstructionKind::LocalSet("edge_1_2_0_0".into())),
                instruction(FinalInstructionKind::I32Const(17)),
                instruction(FinalInstructionKind::LocalSet("ordinary".into())),
                instruction(FinalInstructionKind::Block {
                    label: Some("exit".into()),
                    results: Vec::new(),
                }),
                instruction(FinalInstructionKind::Loop {
                    label: Some("loop".into()),
                    results: Vec::new(),
                }),
                instruction(FinalInstructionKind::End),
                instruction(FinalInstructionKind::End),
                instruction(FinalInstructionKind::LocalGet("ordinary".into())),
                instruction(FinalInstructionKind::Simple(SimpleOpcode::Drop)),
                instruction(FinalInstructionKind::LocalGet("edge_1_2_0_0".into())),
                instruction(FinalInstructionKind::Simple(SimpleOpcode::Drop)),
            ],
        };
        // Include an O3-eligible v128 declaration in this structured function
        // to verify the new type support keeps control-flow fences intact.
        original.locals.push(FinalLocal {
            name: "vector".into(),
            ty: WasmValueType::V128,
        });

        let optimized = optimize(original.clone(), 3);
        assert!(
            optimized
                .locals
                .iter()
                .any(|local| local.name == "ordinary")
        );
        assert!(
            optimized
                .locals
                .iter()
                .any(|local| local.name == "edge_1_2_0_0")
        );
        assert!(optimized.locals.iter().all(|local| local.name != "vector"));
        assert_eq!(optimized.body, original.body);
        validate_wat(&module(optimized));
    }

    #[test]
    fn stackifies_nested_intervals_deterministically() {
        let original = function(
            &[("a", WasmValueType::I32), ("b", WasmValueType::I32)],
            vec![
                instruction(FinalInstructionKind::I32Const(1)),
                instruction(FinalInstructionKind::LocalSet("a".into())),
                instruction(FinalInstructionKind::I32Const(2)),
                instruction(FinalInstructionKind::LocalSet("b".into())),
                instruction(FinalInstructionKind::LocalGet("b".into())),
                instruction(FinalInstructionKind::Simple(SimpleOpcode::Drop)),
                instruction(FinalInstructionKind::LocalGet("a".into())),
                instruction(FinalInstructionKind::Simple(SimpleOpcode::Drop)),
            ],
        );
        let first = optimize(original.clone(), 3);
        let second = optimize(original, 3);

        assert_eq!(first, second);
        assert!(first.locals.is_empty());
        assert!(first.body.iter().all(|instruction| !matches!(
            instruction.kind,
            FinalInstructionKind::LocalGet(_) | FinalInstructionKind::LocalSet(_)
        )));
    }

    #[test]
    fn keeps_a_single_use_local_when_intervening_stack_is_unbalanced() {
        let optimized = optimize(
            function(
                &[("value", WasmValueType::I32)],
                vec![
                    instruction(FinalInstructionKind::I32Const(8)),
                    instruction(FinalInstructionKind::LocalSet("value".into())),
                    instruction(FinalInstructionKind::I32Const(1)),
                    instruction(FinalInstructionKind::LocalGet("value".into())),
                    instruction(FinalInstructionKind::Simple(SimpleOpcode::I32Add)),
                    instruction(FinalInstructionKind::Simple(SimpleOpcode::Drop)),
                ],
            ),
            3,
        );

        assert!(optimized.locals.iter().any(|local| local.name == "value"));
        assert!(optimized.body.iter().any(|instruction| matches!(
            &instruction.kind,
            FinalInstructionKind::LocalSet(name) if name == "value"
        )));
        assert!(optimized.body.iter().any(|instruction| matches!(
            &instruction.kind,
            FinalInstructionKind::LocalGet(name) if name == "value"
        )));
    }

    #[test]
    fn leaves_crossing_single_use_intervals_materialized() {
        let optimized = optimize(
            function(
                &[("a", WasmValueType::I32), ("b", WasmValueType::I32)],
                vec![
                    instruction(FinalInstructionKind::I32Const(1)),
                    instruction(FinalInstructionKind::LocalSet("a".into())),
                    instruction(FinalInstructionKind::I32Const(2)),
                    instruction(FinalInstructionKind::LocalSet("b".into())),
                    instruction(FinalInstructionKind::LocalGet("a".into())),
                    instruction(FinalInstructionKind::Simple(SimpleOpcode::Drop)),
                    instruction(FinalInstructionKind::LocalGet("b".into())),
                    instruction(FinalInstructionKind::Simple(SimpleOpcode::Drop)),
                ],
            ),
            3,
        );

        for name in ["a", "b"] {
            assert!(optimized.locals.iter().any(|local| local.name == name));
            assert!(optimized.body.iter().any(|instruction| matches!(
                &instruction.kind,
                FinalInstructionKind::LocalSet(local) if local == name
            )));
            assert!(optimized.body.iter().any(|instruction| matches!(
                &instruction.kind,
                FinalInstructionKind::LocalGet(local) if local == name
            )));
        }
    }

    #[test]
    fn tees_adjacent_reads_and_coalesces_disjoint_matching_locals() {
        let optimized = optimize(
            function(
                &[
                    ("a", WasmValueType::I32),
                    ("b", WasmValueType::I32),
                    ("sink", WasmValueType::I32),
                ],
                vec![
                    instruction(FinalInstructionKind::I32Const(3)),
                    instruction(FinalInstructionKind::LocalSet("a".into())),
                    instruction(FinalInstructionKind::LocalGet("a".into())),
                    instruction(FinalInstructionKind::I32Const(1)),
                    instruction(FinalInstructionKind::Simple(SimpleOpcode::I32Add)),
                    instruction(FinalInstructionKind::LocalSet("sink".into())),
                    instruction(FinalInstructionKind::LocalGet("a".into())),
                    instruction(FinalInstructionKind::I32Const(2)),
                    instruction(FinalInstructionKind::Simple(SimpleOpcode::I32Add)),
                    instruction(FinalInstructionKind::LocalSet("sink".into())),
                    instruction(FinalInstructionKind::I32Const(5)),
                    instruction(FinalInstructionKind::LocalSet("b".into())),
                    instruction(FinalInstructionKind::LocalGet("b".into())),
                    instruction(FinalInstructionKind::LocalGet("b".into())),
                    instruction(FinalInstructionKind::Simple(SimpleOpcode::I32Add)),
                    instruction(FinalInstructionKind::LocalSet("sink".into())),
                ],
            ),
            3,
        );

        assert!(optimized.locals.iter().all(|local| local.name != "b"));
        assert!(optimized.body.iter().any(|instruction| matches!(
            &instruction.kind,
            FinalInstructionKind::LocalTee(name) if name == "a"
        )));
        assert!(optimized.body.iter().all(|instruction| !matches!(
            &instruction.kind,
            FinalInstructionKind::LocalGet(name) | FinalInstructionKind::LocalSet(name) if name == "b"
        )));
    }

    #[test]
    fn coalesces_disjoint_v128_locals_only_with_matching_vector_types() {
        let original = function(
            &[
                ("a", WasmValueType::V128),
                ("b", WasmValueType::V128),
                ("vector_sink", WasmValueType::V128),
                ("scalar", WasmValueType::I32),
                ("scalar_sink", WasmValueType::I32),
            ],
            vec![
                instruction(FinalInstructionKind::V128Const(vec![1; 16])),
                instruction(FinalInstructionKind::LocalSet("a".into())),
                instruction(FinalInstructionKind::LocalGet("a".into())),
                instruction(FinalInstructionKind::LocalGet("a".into())),
                instruction(FinalInstructionKind::Simple(SimpleOpcode::V128And)),
                instruction(FinalInstructionKind::LocalSet("vector_sink".into())),
                instruction(FinalInstructionKind::V128Const(vec![2; 16])),
                instruction(FinalInstructionKind::LocalSet("b".into())),
                instruction(FinalInstructionKind::LocalGet("b".into())),
                instruction(FinalInstructionKind::LocalGet("b".into())),
                instruction(FinalInstructionKind::Simple(SimpleOpcode::V128And)),
                instruction(FinalInstructionKind::LocalSet("vector_sink".into())),
                instruction(FinalInstructionKind::I32Const(7)),
                instruction(FinalInstructionKind::LocalSet("scalar".into())),
                instruction(FinalInstructionKind::LocalGet("scalar".into())),
                instruction(FinalInstructionKind::LocalGet("scalar".into())),
                instruction(FinalInstructionKind::Simple(SimpleOpcode::I32Add)),
                instruction(FinalInstructionKind::LocalSet("scalar_sink".into())),
            ],
        );

        let optimized = optimize(original.clone(), 3);
        assert!(
            optimized
                .locals
                .iter()
                .any(|local| { local.name == "a" && local.ty == WasmValueType::V128 })
        );
        assert!(optimized.locals.iter().all(|local| local.name != "b"));
        assert!(
            optimized
                .locals
                .iter()
                .any(|local| { local.name == "scalar" && local.ty == WasmValueType::I32 })
        );
        assert!(optimized.body.iter().all(|instruction| !matches!(
            &instruction.kind,
            FinalInstructionKind::LocalGet(name) | FinalInstructionKind::LocalSet(name)
                if name == "b"
        )));
        validate_wat(&module(original));
        validate_wat(&module(optimized));
    }

    #[test]
    fn does_not_coalesce_overlapping_intervals_or_different_types() {
        let optimized = optimize(
            function(
                &[
                    ("a", WasmValueType::I32),
                    ("b", WasmValueType::I32),
                    ("c", WasmValueType::I64),
                ],
                vec![
                    instruction(FinalInstructionKind::I32Const(3)),
                    instruction(FinalInstructionKind::LocalSet("a".into())),
                    instruction(FinalInstructionKind::I32Const(5)),
                    instruction(FinalInstructionKind::LocalSet("b".into())),
                    instruction(FinalInstructionKind::LocalGet("a".into())),
                    instruction(FinalInstructionKind::LocalGet("a".into())),
                    instruction(FinalInstructionKind::Simple(SimpleOpcode::I32Add)),
                    instruction(FinalInstructionKind::Simple(SimpleOpcode::Drop)),
                    instruction(FinalInstructionKind::LocalGet("b".into())),
                    instruction(FinalInstructionKind::LocalGet("b".into())),
                    instruction(FinalInstructionKind::Simple(SimpleOpcode::I32Add)),
                    instruction(FinalInstructionKind::Simple(SimpleOpcode::Drop)),
                    instruction(FinalInstructionKind::I64Const(7)),
                    instruction(FinalInstructionKind::LocalSet("c".into())),
                    instruction(FinalInstructionKind::LocalGet("c".into())),
                    instruction(FinalInstructionKind::LocalGet("c".into())),
                    instruction(FinalInstructionKind::Simple(SimpleOpcode::I64Add)),
                    instruction(FinalInstructionKind::Simple(SimpleOpcode::Drop)),
                ],
            ),
            3,
        );

        for name in ["a", "b", "c"] {
            assert!(optimized.locals.iter().any(|local| local.name == name));
        }
    }

    #[test]
    fn respects_call_memory_trap_and_bulk_memory_fences() {
        let fences = [
            vec![instruction(FinalInstructionKind::Call("callee".into()))],
            vec![
                instruction(FinalInstructionKind::I32Const(0)),
                instruction(FinalInstructionKind::Load {
                    opcode: MemoryOpcode::I32Load,
                    offset: 0,
                    align: 2,
                }),
                instruction(FinalInstructionKind::Simple(SimpleOpcode::Drop)),
            ],
            vec![
                instruction(FinalInstructionKind::I32Const(8)),
                instruction(FinalInstructionKind::I32Const(0)),
                instruction(FinalInstructionKind::Simple(SimpleOpcode::I32DivS)),
                instruction(FinalInstructionKind::Simple(SimpleOpcode::Drop)),
            ],
            vec![
                instruction(FinalInstructionKind::I32Const(0)),
                instruction(FinalInstructionKind::I32Const(0)),
                instruction(FinalInstructionKind::I32Const(4)),
                instruction(FinalInstructionKind::MemoryCopy),
            ],
            vec![
                instruction(FinalInstructionKind::I32Const(0)),
                instruction(FinalInstructionKind::I32Const(0)),
                instruction(FinalInstructionKind::I32Const(4)),
                instruction(FinalInstructionKind::MemoryFill),
            ],
        ];

        for fence in fences {
            let mut body = vec![
                instruction(FinalInstructionKind::I32Const(8)),
                instruction(FinalInstructionKind::LocalSet("v".into())),
            ];
            body.extend(fence);
            body.extend([
                instruction(FinalInstructionKind::LocalGet("v".into())),
                instruction(FinalInstructionKind::Simple(SimpleOpcode::Drop)),
            ]);
            let optimized = optimize(function(&[("v", WasmValueType::I32)], body), 3);
            assert!(optimized.locals.iter().any(|local| local.name == "v"));
            assert!(optimized.body.iter().any(|instruction| matches!(
                &instruction.kind,
                FinalInstructionKind::LocalSet(name) if name == "v"
            )));
            assert!(optimized.body.iter().any(|instruction| matches!(
                &instruction.kind,
                FinalInstructionKind::LocalGet(name) if name == "v"
            )));
        }
    }

    #[test]
    fn falls_back_for_unknown_instruction_or_non_scalar_signature() {
        let original = function(
            &[("v", WasmValueType::I32)],
            vec![
                instruction(FinalInstructionKind::I32Const(3)),
                instruction(FinalInstructionKind::LocalSet("v".into())),
                instruction(FinalInstructionKind::LocalGet("v".into())),
                instruction(FinalInstructionKind::Simple(SimpleOpcode::Drop)),
                instruction(FinalInstructionKind::Simple(SimpleOpcode::I32Clz)),
            ],
        );
        assert_eq!(optimize(original.clone(), 3), original);

        let original = FinalWasmFunction {
            name: "f".into(),
            export_name: None,
            params: vec![FinalLocal {
                name: "v".into(),
                ty: WasmValueType::F32,
            }],
            results: Vec::new(),
            locals: Vec::new(),
            body: vec![
                instruction(FinalInstructionKind::LocalGet("v".into())),
                instruction(FinalInstructionKind::Simple(SimpleOpcode::Drop)),
            ],
        };
        assert_eq!(optimize(original.clone(), 3), original);
    }

    #[test]
    fn pins_parameters_edge_snapshots_and_slice_components() {
        let original = FinalWasmFunction {
            name: "f".into(),
            export_name: None,
            params: vec![FinalLocal {
                name: "p".into(),
                ty: WasmValueType::I32,
            }],
            results: Vec::new(),
            locals: ["edge_1_2_0_0", "slice_data", "slice_len"]
                .into_iter()
                .map(|name| FinalLocal {
                    name: name.into(),
                    ty: WasmValueType::I32,
                })
                .collect(),
            body: vec![
                instruction(FinalInstructionKind::LocalGet("p".into())),
                instruction(FinalInstructionKind::LocalSet("edge_1_2_0_0".into())),
                instruction(FinalInstructionKind::LocalGet("edge_1_2_0_0".into())),
                instruction(FinalInstructionKind::LocalSet("slice_data".into())),
                instruction(FinalInstructionKind::LocalGet("slice_data".into())),
                instruction(FinalInstructionKind::LocalSet("slice_len".into())),
                instruction(FinalInstructionKind::LocalGet("p".into())),
                instruction(FinalInstructionKind::Simple(SimpleOpcode::Drop)),
            ],
        };

        assert_eq!(optimize(original.clone(), 3), original);
    }

    #[test]
    fn pins_slice_components_with_numeric_collision_suffixes() {
        let optimized = optimize(
            function(
                &[
                    ("ordinary", WasmValueType::I32),
                    ("slice_data_1", WasmValueType::I32),
                    ("slice_len_2", WasmValueType::I32),
                    ("sink0", WasmValueType::I32),
                    ("sink1", WasmValueType::I32),
                    ("sink2", WasmValueType::I32),
                ],
                vec![
                    instruction(FinalInstructionKind::I32Const(1)),
                    instruction(FinalInstructionKind::LocalSet("ordinary".into())),
                    instruction(FinalInstructionKind::LocalGet("ordinary".into())),
                    instruction(FinalInstructionKind::LocalGet("ordinary".into())),
                    instruction(FinalInstructionKind::Simple(SimpleOpcode::I32Add)),
                    instruction(FinalInstructionKind::LocalSet("sink0".into())),
                    instruction(FinalInstructionKind::I32Const(2)),
                    instruction(FinalInstructionKind::LocalSet("slice_data_1".into())),
                    instruction(FinalInstructionKind::LocalGet("slice_data_1".into())),
                    instruction(FinalInstructionKind::LocalGet("slice_data_1".into())),
                    instruction(FinalInstructionKind::Simple(SimpleOpcode::I32Add)),
                    instruction(FinalInstructionKind::LocalSet("sink1".into())),
                    instruction(FinalInstructionKind::I32Const(3)),
                    instruction(FinalInstructionKind::LocalSet("slice_len_2".into())),
                    instruction(FinalInstructionKind::LocalGet("slice_len_2".into())),
                    instruction(FinalInstructionKind::LocalGet("slice_len_2".into())),
                    instruction(FinalInstructionKind::Simple(SimpleOpcode::I32Add)),
                    instruction(FinalInstructionKind::LocalSet("sink2".into())),
                ],
            ),
            3,
        );

        for name in ["slice_data_1", "slice_len_2"] {
            assert!(optimized.locals.iter().any(|local| local.name == name));
            assert!(optimized.body.iter().any(|instruction| matches!(
                &instruction.kind,
                FinalInstructionKind::LocalTee(local) | FinalInstructionKind::LocalSet(local)
                    if local == name
            )));
        }
    }

    #[test]
    fn leaves_o0_through_o2_unchanged_and_is_deterministic() {
        let original = function(
            &[("v", WasmValueType::I32)],
            vec![
                instruction(FinalInstructionKind::I32Const(1)),
                instruction(FinalInstructionKind::LocalSet("v".into())),
                instruction(FinalInstructionKind::LocalGet("v".into())),
                instruction(FinalInstructionKind::Simple(SimpleOpcode::Drop)),
            ],
        );
        for level in 0..=2 {
            assert_eq!(optimize(original.clone(), level), original);
        }
        assert_eq!(optimize(original.clone(), 3), optimize(original, 3));
    }

    #[test]
    fn falls_back_past_function_work_limits() {
        let mut too_many_lines = module(function(
            &[("unused", WasmValueType::I32)],
            (0..8_191)
                .flat_map(|_| {
                    [
                        instruction(FinalInstructionKind::I32Const(0)),
                        instruction(FinalInstructionKind::Simple(SimpleOpcode::Drop)),
                    ]
                })
                .collect(),
        ));
        let line_limited = too_many_lines.functions[0].clone();
        optimize_final_module(&mut too_many_lines, 3);
        assert_eq!(too_many_lines.functions[0], line_limited);

        let mut too_many_bytes = module(function(
            &[("v", WasmValueType::I32)],
            vec![
                instruction(FinalInstructionKind::I32Const(3)),
                instruction(FinalInstructionKind::LocalSet("v".into())),
                instruction(FinalInstructionKind::LocalGet("v".into())),
                instruction(FinalInstructionKind::Simple(SimpleOpcode::Drop)),
            ],
        ));
        too_many_bytes.functions[0].body[0].indent = MAX_FUNCTION_BYTES + 1;
        let byte_limited = too_many_bytes.functions[0].clone();
        optimize_final_module(&mut too_many_bytes, 3);
        assert_eq!(too_many_bytes.functions[0], byte_limited);
    }
}
