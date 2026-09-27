//! Conservative late physical value placement for generated WebAssembly text.
//!
//! This pass deliberately works on the backend's flat function WAT. It accepts
//! only the syntax and instructions emitted by this backend; an unrecognized
//! function is returned byte-for-byte unchanged.

use std::collections::{BTreeMap, HashMap, HashSet};

const MAX_FUNCTION_LINES: usize = 16_384;
const MAX_FUNCTION_BYTES: usize = 1_048_576;
const MAX_FUNCTION_LOCALS: usize = 4_096;
const MAX_STACKIFY_CANDIDATES: usize = 512;
const MAX_STACKIFY_SCAN_STEPS: usize = 1_000_000;
const MAX_CROSSING_CHECKS: usize = 250_000;

pub(super) fn optimize_wat_module(wat: &str, opt_level: u8) -> String {
    if opt_level < 3 {
        return wat.to_string();
    }

    let lines = wat.split_inclusive('\n').collect::<Vec<_>>();
    let mut output = String::with_capacity(wat.len());
    let mut cursor = 0;
    while cursor < lines.len() {
        if !lines[cursor].trim_start().starts_with("(func $") {
            output.push_str(lines[cursor]);
            cursor += 1;
            continue;
        }

        let start = cursor;
        cursor += 1;
        while cursor < lines.len() && lines[cursor].trim() != ")" {
            cursor += 1;
        }
        if cursor == lines.len() {
            // A malformed or unfamiliar function boundary makes the remainder
            // ambiguous. Preserve the complete original suffix.
            output.push_str(&lines[start..].concat());
            return output;
        }
        cursor += 1;
        let function = lines[start..cursor].concat();
        output.push_str(&optimize_function(&function).unwrap_or(function));
    }
    output
}

#[derive(Debug, Clone)]
struct WatLine {
    text: String,
    kind: LineKind,
    removed: bool,
}

#[derive(Debug, Clone)]
enum LineKind {
    Header,
    Param,
    Result,
    Local { name: String, type_name: String },
    Blank,
    Instruction(InstructionKind),
    Close,
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
}

impl ValueType {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "i32" => Some(Self::I32),
            "i64" => Some(Self::I64),
            "f64" => Some(Self::F64),
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

fn optimize_function(function: &str) -> Option<String> {
    if function.len() > MAX_FUNCTION_BYTES {
        return None;
    }
    let raw_lines = function.split_inclusive('\n').collect::<Vec<_>>();
    if raw_lines.len() > MAX_FUNCTION_LINES || raw_lines.len() < 2 {
        return None;
    }

    let mut lines = raw_lines
        .iter()
        .map(|line| WatLine {
            text: (*line).to_string(),
            kind: LineKind::Blank,
            removed: false,
        })
        .collect::<Vec<_>>();
    let mut declarations = BTreeMap::<String, (ValueType, bool)>::new();
    let mut header_seen = false;
    let mut close_seen = false;

    for (index, line) in raw_lines.iter().enumerate() {
        let trimmed = line.trim();
        if index == 0 {
            if !is_function_header(trimmed) {
                return None;
            }
            lines[index].kind = LineKind::Header;
            header_seen = true;
            continue;
        }
        if trimmed == ")" {
            if index + 1 != raw_lines.len() {
                return None;
            }
            lines[index].kind = LineKind::Close;
            close_seen = true;
            continue;
        }
        if trimmed.is_empty() {
            lines[index].kind = LineKind::Blank;
            continue;
        }
        if trimmed.starts_with("(param ") {
            let (name, type_name) = parse_declaration(trimmed, "param")?;
            let parsed_type = ValueType::parse(&type_name)?;
            if declarations
                .insert(name.clone(), (parsed_type, true))
                .is_some()
            {
                return None;
            }
            lines[index].kind = LineKind::Param;
            continue;
        }
        if trimmed.starts_with("(result ") {
            parse_result(trimmed)?;
            lines[index].kind = LineKind::Result;
            continue;
        }
        if trimmed.starts_with("(local ") {
            let (name, type_name) = parse_declaration(trimmed, "local")?;
            let parsed_type = ValueType::parse(&type_name)?;
            if declarations
                .insert(name.clone(), (parsed_type, false))
                .is_some()
            {
                return None;
            }
            if declarations.len() > MAX_FUNCTION_LOCALS {
                return None;
            }
            lines[index].kind = LineKind::Local { name, type_name };
            continue;
        }
        lines[index].kind = LineKind::Instruction(classify_instruction(trimmed)?);
    }

    if !header_seen || !close_seen || declarations.len() > MAX_FUNCTION_LOCALS {
        return None;
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

    Some(
        lines
            .into_iter()
            .filter(|line| !line.removed)
            .map(|line| line.text)
            .collect(),
    )
}

fn is_function_header(line: &str) -> bool {
    let Some(after_prefix) = line.strip_prefix("(func $") else {
        return false;
    };
    let name_end = after_prefix
        .find(|character: char| character.is_ascii_whitespace() || character == ')')
        .unwrap_or(after_prefix.len());
    let name = &after_prefix[..name_end];
    if !is_wat_identifier(name) {
        return false;
    }
    let suffix = &after_prefix[name_end..];
    suffix.is_empty()
        || suffix
            .strip_prefix(" (export \"")
            .and_then(|export| export.strip_suffix("\")"))
            .is_some_and(|export| export == name && is_wat_identifier(export))
}

fn parse_declaration(line: &str, kind: &str) -> Option<(String, String)> {
    let inner = line.strip_prefix(&format!("({kind} "))?.strip_suffix(')')?;
    let mut parts = inner.split_whitespace();
    let name = parts.next()?.strip_prefix('$')?;
    let type_name = parts.next()?;
    if parts.next().is_some() || !is_wat_identifier(name) {
        return None;
    }
    Some((name.to_string(), type_name.to_string()))
}

fn parse_result(line: &str) -> Option<()> {
    let inner = line.strip_prefix("(result ")?.strip_suffix(')')?;
    let types = inner.split_whitespace().collect::<Vec<_>>();
    if types.is_empty() || types.iter().any(|ty| ValueType::parse(ty).is_none()) {
        return None;
    }
    Some(())
}

fn is_wat_identifier(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '-')
        })
}

fn classify_instruction(line: &str) -> Option<InstructionKind> {
    let parts = line.split_whitespace().collect::<Vec<_>>();
    let opcode = *parts.first()?;
    match opcode {
        "local.get" | "local.set" | "local.tee" => {
            if parts.len() != 2 {
                return None;
            }
            let name = parts[1].strip_prefix('$')?;
            if !is_wat_identifier(name) {
                return None;
            }
            Some(match opcode {
                "local.get" => InstructionKind::Get(name.to_string()),
                "local.set" => InstructionKind::Set(name.to_string()),
                _ => InstructionKind::Tee(name.to_string()),
            })
        }
        "i32.const" | "i64.const" | "f64.const" => {
            if parts.len() != 2 {
                return None;
            }
            Some(InstructionKind::Stack { pops: 0, pushes: 1 })
        }
        "drop" => (parts.len() == 1).then_some(InstructionKind::Stack { pops: 1, pushes: 0 }),
        "i32.eqz" | "i64.eqz" | "f64.neg" | "f64.convert_i32_s" | "f64.convert_i32_u" => {
            (parts.len() == 1).then_some(InstructionKind::Stack { pops: 1, pushes: 1 })
        }
        "i32.add" | "i32.sub" | "i32.mul" | "i32.and" | "i32.or" | "i32.xor" | "i32.shl"
        | "i32.shr_s" | "i32.shr_u" | "i32.rotl" | "i32.rotr" | "i64.add" | "i64.sub"
        | "i64.mul" | "i64.and" | "i64.or" | "i64.xor" | "i64.shl" | "i64.shr_s" | "i64.shr_u"
        | "i64.rotl" | "i64.rotr" | "f64.add" | "f64.sub" | "f64.mul" | "f64.div" | "f64.min"
        | "f64.max" | "f64.copysign" | "i32.eq" | "i32.ne" | "i32.lt_s" | "i32.lt_u"
        | "i32.gt_s" | "i32.gt_u" | "i32.le_s" | "i32.le_u" | "i32.ge_s" | "i32.ge_u"
        | "i64.eq" | "i64.ne" | "i64.lt_s" | "i64.lt_u" | "i64.gt_s" | "i64.gt_u" | "i64.le_s"
        | "i64.le_u" | "i64.ge_s" | "i64.ge_u" | "f64.eq" | "f64.ne" | "f64.lt" | "f64.gt"
        | "f64.le" | "f64.ge" => {
            (parts.len() == 1).then_some(InstructionKind::Stack { pops: 2, pushes: 1 })
        }
        // Integer division and remainder trap on zero and signed overflow.
        // They are known syntax, but remain hard effect boundaries.
        "i32.div_s" | "i32.div_u" | "i32.rem_s" | "i32.rem_u" | "i64.div_s" | "i64.div_u"
        | "i64.rem_s" | "i64.rem_u" => (parts.len() == 1).then_some(InstructionKind::Fence),
        "i32.load" | "i64.load" | "f64.load" | "i32.store" | "i64.store" | "f64.store" => {
            Some(InstructionKind::Fence)
        }
        "call" => (parts.len() == 2 && parts[1].strip_prefix('$').is_some_and(is_wat_identifier))
            .then_some(InstructionKind::Fence),
        "block" | "loop" => (parts.len() == 2
            && parts[1].strip_prefix('$').is_some_and(is_wat_identifier))
        .then_some(InstructionKind::Fence),
        "if" | "else" | "end" | "return" | "unreachable" => {
            (parts.len() == 1).then_some(InstructionKind::Fence)
        }
        "br" | "br_if" => (parts.len() == 2
            && parts[1].strip_prefix('$').is_some_and(is_wat_identifier))
        .then_some(InstructionKind::Fence),
        "br_table" => (parts.len() >= 2
            && parts[1..]
                .iter()
                .all(|label| label.strip_prefix('$').is_some_and(is_wat_identifier)))
        .then_some(InstructionKind::Fence),
        _ => None,
    }
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

fn collect_references(lines: &[WatLine]) -> HashMap<String, LocalReferences> {
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
    lines: &mut [WatLine],
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
        lines[index].text = lines[index].text.replacen("local.set", "local.tee", 1);
        lines[index].kind = LineKind::Instruction(InstructionKind::Tee(name));
        lines[next_index].removed = true;
        index += 2;
    }
}

fn stackify_single_use_values(
    lines: &mut [WatLine],
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

fn segment_ids(lines: &[WatLine]) -> Vec<Option<usize>> {
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
    lines: &mut [WatLine],
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
                    line.text = replace_local_name(&line.text, alias);
                }
            }
            LineKind::Local { name, .. } => {
                if let Some(alias) = aliases.get(name) {
                    line.removed = true;
                    *name = alias.clone();
                }
            }
            LineKind::Header
            | LineKind::Param
            | LineKind::Result
            | LineKind::Blank
            | LineKind::Instruction(_)
            | LineKind::Close => {}
        }
    }
}

fn replace_local_name(line: &str, name: &str) -> String {
    let Some(start) = line.find('$') else {
        return line.to_string();
    };
    let end = line[start + 1..]
        .find(|character: char| character.is_ascii_whitespace())
        .map_or(line.len(), |offset| start + 1 + offset);
    format!("{}${name}{}", &line[..start], &line[end..])
}

fn remove_unreferenced_locals(lines: &mut [WatLine]) {
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
    use super::optimize_wat_module;

    fn optimize(function: &str) -> String {
        let module = format!("(module\n{function}\n)\n");
        optimize_wat_module(&module, 3)
    }

    #[test]
    fn stackifies_a_single_use_local_roundtrip() {
        let output = optimize(
            "  (func $f\n    (local $v0 i32)\n    (local $v1 i32)\n    i32.const 3\n    local.set $v0\n    local.get $v0\n    i32.const 1\n    i32.add\n    local.set $v1\n  )",
        );

        assert!(!output.contains("(local $v0 i32)"), "{output}");
        assert!(!output.contains("local.get $v0"), "{output}");
        assert!(!output.contains("local.set $v0"), "{output}");
        assert!(
            output.contains("i32.const 3\n    i32.const 1\n    i32.add"),
            "{output}"
        );
    }

    #[test]
    fn stackifies_nested_intervals_deterministically() {
        let source = "  (func $f\n    (local $a i32)\n    (local $b i32)\n    i32.const 1\n    local.set $a\n    i32.const 2\n    local.set $b\n    local.get $b\n    drop\n    local.get $a\n    drop\n  )";
        let first = optimize(source);
        let second = optimize(source);

        assert_eq!(first, second);
        assert!(!first.contains("(local $a i32)"), "{first}");
        assert!(!first.contains("(local $b i32)"), "{first}");
        assert!(!first.contains("local.get $a"), "{first}");
        assert!(!first.contains("local.set $a"), "{first}");
        assert!(!first.contains("local.get $b"), "{first}");
        assert!(!first.contains("local.set $b"), "{first}");
    }

    #[test]
    fn leaves_a_single_use_local_when_the_intervening_stack_is_unbalanced() {
        let output = optimize(
            "  (func $f\n    (local $value i32)\n    i32.const 8\n    local.set $value\n    i32.const 1\n    local.get $value\n    i32.add\n    drop\n  )",
        );

        assert!(output.contains("local.set $value"), "{output}");
        assert!(output.contains("local.get $value"), "{output}");
    }

    #[test]
    fn leaves_crossing_single_use_intervals_materialized() {
        let output = optimize(
            "  (func $f\n    (local $a i32)\n    (local $b i32)\n    i32.const 1\n    local.set $a\n    i32.const 2\n    local.set $b\n    local.get $a\n    drop\n    local.get $b\n    drop\n  )",
        );

        assert!(output.contains("local.set $a"), "{output}");
        assert!(output.contains("local.get $a"), "{output}");
        assert!(output.contains("local.set $b"), "{output}");
        assert!(output.contains("local.get $b"), "{output}");
    }

    #[test]
    fn tees_an_adjacent_read_when_the_local_is_used_again() {
        let output = optimize(
            "  (func $f\n    (local $v0 i32)\n    (local $v1 i32)\n    (local $v2 i32)\n    i32.const 3\n    local.set $v0\n    local.get $v0\n    i32.const 1\n    i32.add\n    local.set $v1\n    local.get $v0\n    i32.const 2\n    i32.add\n    local.set $v2\n  )",
        );

        assert!(output.contains("local.tee $v0"), "{output}");
        assert_eq!(output.matches("local.get $v0").count(), 1, "{output}");
    }

    #[test]
    fn coalesces_disjoint_single_write_intervals_with_matching_types() {
        let output = optimize(
            "  (func $f\n    (local $a i32)\n    (local $b i32)\n    (local $sink0 i32)\n    (local $sink1 i32)\n    i32.const 3\n    local.set $a\n    i32.const 0\n    drop\n    local.get $a\n    local.get $a\n    i32.add\n    local.set $sink0\n    i32.const 5\n    local.set $b\n    i32.const 0\n    drop\n    local.get $b\n    local.get $b\n    i32.add\n    local.set $sink1\n  )",
        );

        assert!(!output.contains("(local $b i32)"), "{output}");
        assert_eq!(output.matches("local.set $a").count(), 2, "{output}");
        assert!(!output.contains("local.get $b"), "{output}");
    }

    #[test]
    fn does_not_coalesce_overlapping_intervals_or_different_types() {
        let output = optimize(
            "  (func $f\n    (local $a i32)\n    (local $b i32)\n    (local $c i64)\n    (local $sink i32)\n    i32.const 3\n    local.set $a\n    i32.const 5\n    local.set $b\n    local.get $a\n    local.get $a\n    i32.add\n    local.set $sink\n    local.get $b\n    local.get $b\n    i32.add\n    local.set $sink\n    i64.const 7\n    local.set $c\n    local.get $c\n    local.get $c\n    i64.add\n    drop\n  )",
        );

        assert!(output.contains("(local $a i32)"), "{output}");
        assert!(output.contains("(local $b i32)"), "{output}");
        assert!(output.contains("(local $c i64)"), "{output}");
    }

    #[test]
    fn keeps_values_across_call_memory_trap_and_unknown_fences() {
        for fence in [
            "    call $callee\n",
            "    local.get $ptr\n    i32.load offset=0 align=4\n",
            "    i32.div_s\n",
            "    i32.clz\n",
        ] {
            let mut body = String::from(
                "  (func $f\n    (param $ptr i32)\n    (local $v i32)\n    i32.const 8\n    local.set $v\n",
            );
            body.push_str(fence);
            body.push_str("    local.get $v\n    drop\n  )");
            let output = optimize(&body);
            assert!(output.contains("local.set $v"), "fence={fence:?}\n{output}");
            assert!(output.contains("local.get $v"), "fence={fence:?}\n{output}");
        }
    }

    #[test]
    fn pins_parameters_edge_snapshots_and_slice_components() {
        let output = optimize(
            "  (func $f\n    (param $p i32)\n    (local $edge_1_2_0_0 i32)\n    (local $slice_data i32)\n    (local $slice_len i32)\n    local.get $p\n    local.set $edge_1_2_0_0\n    local.get $edge_1_2_0_0\n    local.set $slice_data\n    local.get $slice_data\n    local.set $slice_len\n    local.get $p\n    drop\n  )",
        );

        assert!(output.contains("local.get $p"), "{output}");
        assert!(output.contains("local.set $edge_1_2_0_0"), "{output}");
        assert!(output.contains("(local $slice_data i32)"), "{output}");
        assert!(output.contains("(local $slice_len i32)"), "{output}");
    }

    #[test]
    fn pins_slice_components_with_numeric_collision_suffixes() {
        let output = optimize(
            "  (func $f\n    (local $ordinary i32)\n    (local $slice_data_1 i32)\n    (local $slice_len_2 i32)\n    (local $sink0 i32)\n    (local $sink1 i32)\n    (local $sink2 i32)\n    i32.const 1\n    local.set $ordinary\n    i32.const 0\n    drop\n    local.get $ordinary\n    local.get $ordinary\n    i32.add\n    local.set $sink0\n    i32.const 2\n    local.set $slice_data_1\n    i32.const 0\n    drop\n    local.get $slice_data_1\n    local.get $slice_data_1\n    i32.add\n    local.set $sink1\n    i32.const 3\n    local.set $slice_len_2\n    i32.const 0\n    drop\n    local.get $slice_len_2\n    local.get $slice_len_2\n    i32.add\n    local.set $sink2\n  )",
        );

        assert!(output.contains("(local $slice_data_1 i32)"), "{output}");
        assert!(output.contains("(local $slice_len_2 i32)"), "{output}");
        assert!(output.contains("local.set $slice_data_1"), "{output}");
        assert!(output.contains("local.set $slice_len_2"), "{output}");
    }

    #[test]
    fn leaves_o0_through_o2_wat_unchanged_and_is_deterministic() {
        let source = "(module\n  (func $f\n    (local $v i32)\n    i32.const 1\n    local.set $v\n    local.get $v\n    drop\n  )\n)\n";
        for opt_level in 0..=2 {
            assert_eq!(optimize_wat_module(source, opt_level), source);
        }
        assert_eq!(
            optimize_wat_module(source, 3),
            optimize_wat_module(source, 3)
        );
    }
}
