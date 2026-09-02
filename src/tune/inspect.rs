use std::fmt::Write;

use super::schema::{TUNE_DECISION_MAGIC, TUNE_DECISION_SCHEMA};
use super::{TuneDecision, TuneDecisionError};

#[derive(Clone)]
enum Kind {
    Integer(&'static str, usize),
    Bool,
    Digest,
    Text,
    Enum(&'static str, &'static [&'static str]),
    Record(&'static str),
    List(Box<Kind>, u32),
    Optional(Box<Kind>),
}

impl Kind {
    fn type_name(&self) -> String {
        match self {
            Self::Integer(name, _) => (*name).to_owned(),
            Self::Bool => "bool".to_owned(),
            Self::Digest => "d32".to_owned(),
            Self::Text => "text".to_owned(),
            Self::Enum(name, _) => format!("enum:{name}"),
            Self::Record(name) => format!("record:{name}"),
            Self::List(item, bound) => format!("list:{}:{bound}", item.type_name()),
            Self::Optional(inner) => format!("optional:{}", inner.type_name()),
        }
    }
}

struct Node {
    tag: u16,
    kind: Kind,
    value: Value,
}

enum Value {
    Token(String),
    Bool(bool),
    Text(String),
    Record(Vec<Node>),
    List(Vec<Value>),
    Optional(Option<Box<Value>>),
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(
        &mut self,
        length: usize,
        context: &'static str,
    ) -> Result<&'a [u8], TuneDecisionError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(TuneDecisionError::ResourceLimit(context))?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(TuneDecisionError::Truncated(context))?;
        self.offset = end;
        Ok(value)
    }

    fn finish(self, context: &'static str) -> Result<(), TuneDecisionError> {
        if self.offset != self.bytes.len() {
            return Err(TuneDecisionError::InvalidValue(context));
        }
        Ok(())
    }

    fn u8(&mut self, context: &'static str) -> Result<u8, TuneDecisionError> {
        Ok(self.take(1, context)?[0])
    }

    fn u16(&mut self, context: &'static str) -> Result<u16, TuneDecisionError> {
        let bytes = self.take(2, context)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self, context: &'static str) -> Result<u32, TuneDecisionError> {
        let bytes = self.take(4, context)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
}

/// Renders a validated decision as canonical inspection-schema-1 JSON.
///
/// # Errors
///
/// Returns an error if the retained decision bytes no longer parse exactly.
pub fn inspect_tune_json(decision: &TuneDecision) -> Result<String, TuneDecisionError> {
    let (digest, records) = inspect_tree(decision)?;
    let magic =
        std::str::from_utf8(TUNE_DECISION_MAGIC).map_err(|_| TuneDecisionError::InvalidUtf8)?;
    let mut output = format!(
        "{{\"fileMagic\":\"{magic}\",\"formatSchema\":{TUNE_DECISION_SCHEMA},\"decisionDigest\":\""
    );
    output.push_str(&digest);
    output.push_str("\",\"records\":[");
    render_json_nodes(&records, &mut output);
    output.push_str("]}\n");
    Ok(output)
}

/// Renders a validated decision as canonical inspection-schema-1 text.
///
/// # Errors
///
/// Returns an error if the retained decision bytes no longer parse exactly.
pub fn inspect_tune_text(decision: &TuneDecision) -> Result<String, TuneDecisionError> {
    let (digest, records) = inspect_tree(decision)?;
    let mut output = format!("CKTUNE-INSPECT\t{TUNE_DECISION_SCHEMA}\t{digest}\n");
    for node in &records {
        render_text_node(node, &format!("/{}", node.tag), &mut output);
    }
    Ok(output)
}

fn inspect_tree(decision: &TuneDecision) -> Result<(String, Vec<Node>), TuneDecisionError> {
    decision.validate_self_contained()?;
    let bytes = decision.as_bytes();
    let body_end = bytes
        .len()
        .checked_sub(32)
        .ok_or(TuneDecisionError::Truncated("inspection digest"))?;
    let digest = hex(&bytes[body_end..]);
    let top = bytes
        .get(12..body_end)
        .ok_or(TuneDecisionError::Truncated("inspection records"))?;
    let names = [
        "Identity",
        "Contract",
        "Workload",
        "Environment",
        "Frontier",
        "Candidates",
        "Selection",
        "Replay",
    ];
    let mut cursor = Cursor::new(top);
    let mut records = Vec::with_capacity(names.len());
    for (index, name) in names.into_iter().enumerate() {
        let expected = u16::try_from(index + 1)
            .map_err(|_| TuneDecisionError::ResourceLimit("inspection tag"))?;
        let tag = cursor.u16("inspection field")?;
        if tag != expected {
            return Err(TuneDecisionError::NonCanonicalOrder("inspection records"));
        }
        let length = usize::try_from(cursor.u32("inspection field")?)
            .map_err(|_| TuneDecisionError::ResourceLimit("inspection field"))?;
        let payload = cursor.take(length, "inspection field")?;
        records.push(Node {
            tag,
            kind: Kind::Record(name),
            value: Value::Record(parse_record(name, payload)?),
        });
    }
    cursor.finish("inspection records")?;
    Ok((digest, records))
}

fn parse_record(name: &'static str, bytes: &[u8]) -> Result<Vec<Node>, TuneDecisionError> {
    let mut cursor = Cursor::new(bytes);
    let mut nodes = Vec::new();
    for (expected_tag, mut kind) in record_fields(name)? {
        let tag = cursor.u16("inspection record")?;
        if tag != expected_tag {
            return Err(TuneDecisionError::NonCanonicalOrder(name));
        }
        let length = usize::try_from(cursor.u32("inspection record")?)
            .map_err(|_| TuneDecisionError::ResourceLimit(name))?;
        let payload = cursor.take(length, "inspection record")?;
        if name == "AlternativePayload" && expected_tag == 2 {
            let class = nodes
                .first()
                .and_then(|node: &Node| match &node.value {
                    Value::Token(value) => alternative_record(value),
                    _ => None,
                })
                .ok_or(TuneDecisionError::InvalidValue("AlternativePayload.class"))?;
            kind = Kind::Record(class);
        }
        let value = parse_value(&kind, payload, name)?;
        nodes.push(Node { tag, kind, value });
    }
    cursor.finish(name)?;
    Ok(nodes)
}

fn parse_value(
    kind: &Kind,
    bytes: &[u8],
    context: &'static str,
) -> Result<Value, TuneDecisionError> {
    let mut cursor = Cursor::new(bytes);
    let value = parse_cursor_value(kind, &mut cursor, context)?;
    cursor.finish(context)?;
    Ok(value)
}

fn parse_cursor_value(
    kind: &Kind,
    cursor: &mut Cursor<'_>,
    context: &'static str,
) -> Result<Value, TuneDecisionError> {
    match kind {
        Kind::Integer(_, width) => {
            let bytes = cursor.take(*width, context)?;
            let value = bytes
                .iter()
                .fold(0u128, |value, byte| (value << 8) | u128::from(*byte));
            Ok(Value::Token(value.to_string()))
        }
        Kind::Bool => match cursor.u8(context)? {
            0 => Ok(Value::Bool(false)),
            1 => Ok(Value::Bool(true)),
            _ => Err(TuneDecisionError::InvalidValue(context)),
        },
        Kind::Digest => Ok(Value::Token(hex(cursor.take(32, context)?))),
        Kind::Text => {
            let length = usize::try_from(cursor.u32(context)?)
                .map_err(|_| TuneDecisionError::ResourceLimit(context))?;
            let bytes = cursor.take(length, context)?;
            let value = std::str::from_utf8(bytes)
                .map_err(|_| TuneDecisionError::InvalidUtf8)?
                .to_owned();
            Ok(Value::Text(value))
        }
        Kind::Enum(_, labels) => {
            let discriminant = cursor.u8(context)?;
            let index = usize::from(discriminant.saturating_sub(1));
            let label = labels
                .get(index)
                .filter(|_| discriminant != 0)
                .ok_or(TuneDecisionError::InvalidValue(context))?;
            Ok(Value::Token((*label).to_owned()))
        }
        Kind::Record(name) => {
            let length = usize::try_from(cursor.u32(context)?)
                .map_err(|_| TuneDecisionError::ResourceLimit(context))?;
            let bytes = cursor.take(length, context)?;
            Ok(Value::Record(parse_record(name, bytes)?))
        }
        Kind::List(item, bound) => {
            let count = cursor.u32(context)?;
            if count > *bound {
                return Err(TuneDecisionError::ResourceLimit(context));
            }
            let capacity =
                usize::try_from(count).map_err(|_| TuneDecisionError::ResourceLimit(context))?;
            let mut values = Vec::with_capacity(capacity);
            for _ in 0..count {
                values.push(parse_cursor_value(item, cursor, context)?);
            }
            Ok(Value::List(values))
        }
        Kind::Optional(inner) => match cursor.u8(context)? {
            0 => Ok(Value::Optional(None)),
            1 => Ok(Value::Optional(Some(Box::new(parse_cursor_value(
                inner, cursor, context,
            )?)))),
            _ => Err(TuneDecisionError::InvalidValue(context)),
        },
    }
}

fn render_json_nodes(nodes: &[Node], output: &mut String) {
    for (index, node) in nodes.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        let _ = write!(
            output,
            "{{\"tag\":{},\"type\":\"{}\",\"value\":",
            node.tag,
            node.kind.type_name()
        );
        render_json_value(&node.value, output);
        output.push('}');
    }
}

fn render_json_value(value: &Value, output: &mut String) {
    match value {
        Value::Token(token) => {
            output.push('"');
            output.push_str(token);
            output.push('"');
        }
        Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        Value::Text(value) => render_json_string(value, output),
        Value::Record(nodes) => {
            output.push('[');
            render_json_nodes(nodes, output);
            output.push(']');
        }
        Value::List(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    output.push(',');
                }
                render_json_value(value, output);
            }
            output.push(']');
        }
        Value::Optional(None) => output.push_str("null"),
        Value::Optional(Some(value)) => render_json_value(value, output),
    }
}

fn render_json_string(value: &str, output: &mut String) {
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\u{08}' => output.push_str("\\b"),
            '\u{0c}' => output.push_str("\\f"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            value if value <= '\u{1f}' => {
                let _ = write!(output, "\\u{:04x}", u32::from(value));
            }
            value => output.push(value),
        }
    }
    output.push('"');
}

fn render_text_node(node: &Node, path: &str, output: &mut String) {
    render_text_value(&node.kind, &node.value, path, output);
}

fn render_text_value(kind: &Kind, value: &Value, path: &str, output: &mut String) {
    let summary = match value {
        Value::Token(token) => format!("\"{token}\""),
        Value::Bool(value) => value.to_string(),
        Value::Text(value) => {
            let mut token = String::new();
            render_json_string(value, &mut token);
            token
        }
        Value::Record(nodes) => format!("fields={}", nodes.len()),
        Value::List(values) => format!("items={}", values.len()),
        Value::Optional(None) => "absent".to_owned(),
        Value::Optional(Some(_)) => "present".to_owned(),
    };
    let _ = writeln!(output, "{path}\t{}\t{summary}", kind.type_name());
    match value {
        Value::Record(nodes) => {
            for node in nodes {
                render_text_node(node, &format!("{path}/{}", node.tag), output);
            }
        }
        Value::List(values) => {
            let Kind::List(item, _) = kind else {
                return;
            };
            for (index, value) in values.iter().enumerate() {
                render_text_value(item, value, &format!("{path}/@{index}"), output);
            }
        }
        Value::Optional(Some(value)) => {
            let Kind::Optional(inner) = kind else {
                return;
            };
            render_text_value(inner, value, &format!("{path}/@0"), output);
        }
        Value::Token(_) | Value::Bool(_) | Value::Text(_) | Value::Optional(None) => {}
    }
}

fn record_fields(name: &'static str) -> Result<Vec<(u16, Kind)>, TuneDecisionError> {
    let u8_kind = || Kind::Integer("u8", 1);
    let u16_kind = || Kind::Integer("u16", 2);
    let u32_kind = || Kind::Integer("u32", 4);
    let u64_kind = || Kind::Integer("u64", 8);
    let u128_kind = || Kind::Integer("u128", 16);
    let d32 = || Kind::Digest;
    let text = || Kind::Text;
    let record = |name| Kind::Record(name);
    let list = |kind, bound| Kind::List(Box::new(kind), bound);
    let optional = |kind| Kind::Optional(Box::new(kind));
    let enumeration = |name, labels| Kind::Enum(name, labels);
    let fields = match name {
        "Identity" => vec![
            (1, text()),
            (2, d32()),
            (3, text()),
            (4, text()),
            (5, d32()),
            (6, u32_kind()),
            (7, u32_kind()),
            (8, u32_kind()),
            (9, u32_kind()),
            (10, u32_kind()),
            (11, u32_kind()),
            (12, u32_kind()),
            (13, u32_kind()),
            (14, u32_kind()),
            (15, u32_kind()),
            (16, d32()),
            (17, d32()),
            (18, d32()),
            (19, d32()),
            (20, enumeration("OutputKind", &["executable", "dynamic"])),
            (21, record("TargetIdentity")),
            (22, optional(record("ProfileIdentity"))),
        ],
        "TargetIdentity" => vec![
            (1, text()),
            (2, text()),
            (3, list(text(), 256)),
            (4, text()),
        ],
        "ProfileIdentity" => vec![
            (1, u32_kind()),
            (2, d32()),
            (3, d32()),
            (4, d32()),
            (5, d32()),
            (6, u64_kind()),
        ],
        "Contract" => {
            let mut fields: Vec<_> = (1..=5).map(|tag| (tag, u32_kind())).collect();
            fields.push((6, enumeration("Budget", &["quick", "standard", "thorough"])));
            fields.extend((7..=11).map(|tag| (tag, u32_kind())));
            fields.push((12, u64_kind()));
            fields.extend((13..=14).map(|tag| (tag, u32_kind())));
            fields.extend((15..=16).map(|tag| (tag, u64_kind())));
            fields.extend((17..=31).map(|tag| (tag, u32_kind())));
            fields.push((32, d32()));
            fields
        }
        "Workload" => vec![
            (1, d32()),
            (2, d32()),
            (3, u64_kind()),
            (4, list(text(), 64)),
            (5, list(record("EnvironmentEntry"), 16)),
            (6, u32_kind()),
            (7, list(record("InputIdentity"), 64)),
            (8, list(record("CaseIdentity"), 16)),
        ],
        "EnvironmentEntry" => vec![(1, text()), (2, u64_kind()), (3, d32())],
        "InputIdentity" => vec![(1, text()), (2, d32()), (3, u64_kind())],
        "CaseIdentity" => vec![
            (1, text()),
            (2, enumeration("CaseRole", &["search", "validation"])),
            (3, u64_kind()),
            (4, u32_kind()),
            (5, d32()),
        ],
        "Environment" => {
            let mut fields: Vec<_> = (1..=9).map(|tag| (tag, text())).collect();
            fields.extend([
                (10, list(text(), 256)),
                (11, optional(u32_kind())),
                (12, u32_kind()),
                (13, optional(u32_kind())),
                (14, text()),
                (15, u64_kind()),
                (16, text()),
                (17, list(record("Calibration"), 16)),
                (18, d32()),
                (19, d32()),
            ]);
            fields
        }
        "Calibration" => vec![
            (1, text()),
            (2, u64_kind()),
            (3, u32_kind()),
            (4, u64_kind()),
            (5, u64_kind()),
            (6, Kind::Bool),
        ],
        "Frontier" => vec![
            (1, d32()),
            (2, list(record("Site"), 4_096)),
            (3, list(record("Unit"), 64)),
            (4, list(record("Expansion"), 16_384)),
        ],
        "Site" => vec![
            (1, d32()),
            (2, alternative_class()),
            (3, d32()),
            (4, d32()),
            (5, u32_kind()),
            (6, record("RootAnchor")),
        ],
        "RootAnchor" => vec![
            (1, text()),
            (
                2,
                enumeration(
                    "RootKind",
                    &["module", "function", "loop", "block", "instruction", "call"],
                ),
            ),
            (3, u32_kind()),
        ],
        "Unit" => vec![
            (1, d32()),
            (2, list(d32(), 4_096)),
            (3, d32()),
            (4, list(record("UnitVariant"), 4)),
        ],
        "UnitVariant" => vec![
            (1, d32()),
            (2, alternative_class()),
            (3, list(record("SiteAlternative"), 4_096)),
            (4, u64_kind()),
            (5, u64_kind()),
            (6, u64_kind()),
            (7, d32()),
        ],
        "SiteAlternative" => vec![
            (1, d32()),
            (2, d32()),
            (3, d32()),
            (4, d32()),
            (5, record("AlternativePayload")),
        ],
        "AlternativePayload" => vec![(1, alternative_class()), (2, record("InliningPayload"))],
        "InliningPayload" => vec![
            (1, text()),
            (
                2,
                enumeration("InliningAction", &["force-inline", "keep-out-of-line"]),
            ),
        ],
        "SpecializationPayload" => vec![
            (1, list(record("SpecializationBinding"), 16)),
            (2, Kind::Bool),
        ],
        "SpecializationBinding" => vec![
            (1, u32_kind()),
            (
                2,
                enumeration(
                    "SpecializationValueKind",
                    &[
                        "u32",
                        "u64",
                        "i32",
                        "i64",
                        "f32-bits",
                        "f64-bits",
                        "length-u32",
                    ],
                ),
            ),
            (3, u128_kind()),
        ],
        "UnrollingPayload" => vec![(1, u32_kind())],
        "LoopSimdPayload" | "ShortSliceVersioningPayload" => {
            vec![(1, u32_kind()), (2, u32_kind()), (3, u32_kind())]
        }
        "SlpPayload" => vec![(1, u32_kind()), (2, list(record("RootAnchor"), 64))],
        "LayoutPayload" => vec![
            (
                1,
                enumeration("LayoutScope", &["block", "function", "section"]),
            ),
            (2, list(d32(), 4_096)),
        ],
        "Expansion" => vec![
            (1, u32_kind()),
            (2, d32()),
            (3, d32()),
            (4, d32()),
            (
                5,
                enumeration(
                    "ExpansionDisposition",
                    &["legal", "illegal", "duplicate", "growth-rejected"],
                ),
            ),
            (6, optional(d32())),
            (7, u16_kind()),
            (8, optional(u64_kind())),
            (9, optional(u64_kind())),
            (10, optional(u64_kind())),
        ],
        "Candidates" => vec![(1, record("Candidate")), (2, list(record("Candidate"), 32))],
        "Candidate" => vec![
            (1, d32()),
            (2, list(record("PlanChoice"), 64)),
            (3, d32()),
            (4, d32()),
            (5, u64_kind()),
            (
                6,
                enumeration(
                    "CandidateOutcome",
                    &[
                        "baseline",
                        "compiled-unmeasured",
                        "size-rejected",
                        "timed-out",
                        "search-nonwinner",
                        "validation-threshold",
                        "validation-nonwinner",
                        "selected",
                    ],
                ),
            ),
            (7, u16_kind()),
            (8, optional(d32())),
            (9, list(record("MeasurementStream"), 48)),
            (10, record("CacheOrigin")),
            (11, optional(record("TimeoutRecord"))),
            (12, d32()),
        ],
        "PlanChoice" => vec![
            (1, d32()),
            (2, d32()),
            (3, alternative_class()),
            (4, d32()),
            (5, d32()),
        ],
        "MeasurementStream" => vec![
            (1, ordering_phase()),
            (2, u8_kind()),
            (3, text()),
            (4, d32()),
            (5, u64_kind()),
            (6, list(record("MeasurementRow"), 20)),
            (7, d32()),
        ],
        "MeasurementRow" => vec![
            (1, u32_kind()),
            (2, d32()),
            (3, list(u64_kind(), 3)),
            (4, u64_kind()),
        ],
        "CacheOrigin" => vec![
            (
                1,
                enumeration("CacheOriginKind", &["freshly-built", "verified-local-hit"]),
            ),
            (2, d32()),
            (3, d32()),
        ],
        "TimeoutRecord" => vec![
            (1, ordering_phase()),
            (2, u8_kind()),
            (3, u32_kind()),
            (4, text()),
            (5, u8_kind()),
            (6, u64_kind()),
        ],
        "Selection" => vec![
            (1, record("RoundSummary")),
            (2, record("RoundSummary")),
            (3, d32()),
            (
                4,
                enumeration(
                    "SelectionReason",
                    &[
                        "tuned",
                        "no-candidate",
                        "validation-threshold",
                        "validation-disagreement",
                    ],
                ),
            ),
            (5, optional(record("Certificate"))),
        ],
        "RoundSummary" => vec![
            (1, u8_kind()),
            (2, list(record("RoundPlan"), 4)),
            (3, list(d32(), 4)),
        ],
        "RoundPlan" => vec![
            (1, d32()),
            (2, list(record("CaseMedian"), 16)),
            (3, u64_kind()),
            (4, Kind::Bool),
            (5, Kind::Bool),
            (6, u32_kind()),
        ],
        "CaseMedian" => vec![
            (1, text()),
            (2, u64_kind()),
            (3, u64_kind()),
            (4, u64_kind()),
        ],
        "Certificate" => (1..=8).map(|tag| (tag, d32())).collect(),
        "Replay" => vec![
            (1, d32()),
            (2, d32()),
            (3, d32()),
            (4, d32()),
            (5, d32()),
            (6, list(record("OutputIdentity"), 3)),
            (7, record("CacheOrigin")),
            (8, record("CacheOrigin")),
            (9, d32()),
            (10, d32()),
        ],
        "OutputIdentity" => vec![
            (
                1,
                enumeration("OutputRole", &["primary", "header", "import-library"]),
            ),
            (2, text()),
            (3, d32()),
            (4, u64_kind()),
        ],
        _ => {
            return Err(TuneDecisionError::InvalidValue("inspection record name"));
        }
    };
    Ok(fields)
}

fn alternative_class() -> Kind {
    Kind::Enum(
        "AlternativeClass",
        &[
            "inlining",
            "specialization",
            "unrolling",
            "loop-simd",
            "slp",
            "short-slice/versioning",
            "layout",
        ],
    )
}

fn alternative_record(label: &str) -> Option<&'static str> {
    match label {
        "inlining" => Some("InliningPayload"),
        "specialization" => Some("SpecializationPayload"),
        "unrolling" => Some("UnrollingPayload"),
        "loop-simd" => Some("LoopSimdPayload"),
        "slp" => Some("SlpPayload"),
        "short-slice/versioning" => Some("ShortSliceVersioningPayload"),
        "layout" => Some("LayoutPayload"),
        _ => None,
    }
}

fn ordering_phase() -> Kind {
    Kind::Enum(
        "OrderingPhase",
        &[
            "candidate-smoke",
            "search-warmup",
            "search-measured",
            "validation-one-warmup",
            "validation-one-measured",
            "validation-two-warmup",
            "validation-two-measured",
        ],
    )
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}
