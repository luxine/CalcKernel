# CK 0.14 `.cktune` Inspection Schema 1

Status: normative, language-neutral public inspection contract

This attachment defines both outputs of `ckc tune inspect <decision.cktune>`.
Inspection first validates framing, bounds, canonical order, the trailing digest,
and every cross-record equality computable from the self-contained decision.
Source/KIR-dependent replay equalities are displayed but are rederived only by
source-backed tune-use and acceptance checks. Structurally invalid input produces
no inspection document.

## 1. Canonical JSON

`--json` emits UTF-8, no BOM, no insignificant whitespace, and one final LF.
The root object has exactly these keys in this order:

    fileMagic, formatSchema, decisionDigest, records

Values are respectively the string `CKTUNE01`, the JSON number `1`, a 64-character
lowercase digest string, and an eight-element array for top-level tags 1 through 8.
Every field in `records`, in a nested record, or in a list-of-records value is a
`Node` object with exactly these keys in this order:

    tag, type, value

`tag` is a positive JSON integer and nodes are in increasing tag order. `type` is
one of the exact strings below. `value` has the stated canonical representation:

| Type string | JSON value |
| --- | --- |
| `u8`, `u16`, `u32`, `u64`, `u128` | decimal string with no sign or leading zero except `0` |
| `bool` | JSON boolean |
| `d32` | 64-character lowercase hexadecimal string |
| `text` | JSON string containing the exact validated Unicode scalar sequence |
| `bytes` | lowercase even-length hexadecimal string |
| `enum:<EnumName>` | the Section 12 label left of `=`, with ASCII letters lowercased and punctuation preserved |
| `record:<RecordName>` | array of `Node` objects |
| `list:<ElementType>:<Bound>` | JSON array of element values encoded by `ElementType` |
| `optional:<InnerType>` | JSON null or one value encoded by `InnerType` |

For a list whose element type is a record, every element value is the record's
node array, not another wrapper object. Bounds and optional inner types use the
exact decision-schema type. JSON escaping follows RFC 8259 with only quotation
mark, reverse solidus, and control characters escaped; control characters use
lowercase `\u00xx`, while every other character is emitted as UTF-8. `/` and
non-ASCII characters are not escaped. No NaN, floating point, exponent, negative
zero, duplicate key, alternate integer spelling, or unknown node is permitted.

Record names are closed by this table; it also assigns names to records described
inline in the wire schema:

| Parent field | Record name |
| --- | --- |
| top-level tags 1..8 | `Identity`, `Contract`, `Workload`, `Environment`, `Frontier`, `Candidates`, `Selection`, `Replay` |
| Identity | `TargetIdentity`, `ProfileIdentity` |
| Workload lists | `EnvironmentEntry`, `InputIdentity`, `CaseIdentity` |
| Environment calibrations | `Calibration` |
| Frontier | `Site`, `RootAnchor`, `Unit`, `UnitVariant`, `SiteAlternative`, `AlternativePayload`, `Expansion` |
| AlternativePayload tag 2 | `InliningPayload`, `SpecializationPayload`, `UnrollingPayload`, `LoopSimdPayload`, `SlpPayload`, `ShortSliceVersioningPayload`, or `LayoutPayload`, selected only by tag 1 |
| specialization bindings | `SpecializationBinding` |
| Candidates | `Candidate`, `PlanChoice`, `MeasurementStream`, `MeasurementRow`, `CacheOrigin`, `TimeoutRecord` |
| Selection | `RoundSummary`, `RoundPlan`, `CaseMedian`, `Certificate` |
| Replay | `OutputIdentity`, `CacheOrigin` |

The decision schema's tag/type tables, bounds, enum labels, discriminants, and
required/optional state are the sole source of each node's `tag` and `type`.
Inspection cannot omit a valid field, synthesize a derived field, localize a label,
or include a path, timestamp, address, cache-access time, or diagnostic.

## 2. Stable text

The default format is a complete line-oriented rendering of the same tree. The
first line is exactly:

    CKTUNE-INSPECT<TAB>1<TAB><decisionDigest><LF>

It is followed by one line for every node in depth-first pre-order. Each line is:

    <path><TAB><type><TAB><summary><LF>

`path` starts with `/`; a record-field segment is its decimal tag and a list-index
segment is `@` followed by the zero-based decimal index. Examples are `/1`,
`/1/21/3/@0`, and `/6/2/@0/9/@0/6/@19/4`. Paths are unique and emitted in wire
node/list order.

For scalar and enum nodes, `summary` is the exact canonical JSON token used as that
node's JSON value. For a record it is `fields=<decimal-count>`; for a list it is
`items=<decimal-count>`; for an absent optional it is `absent`; and for a present
optional it is `present`, followed by traversal of the contained value at the same
path plus `/@0`. A present optional record or list emits its contained container
line at that child path before its descendants. Tabs and LF occur only as
separators; scalar strings use JSON escaping and therefore cannot inject a line.

The text output is not localized. Human-friendly translated explanation is a
separate diagnostic surface and is not inspection schema 1.

## 3. Golden and negative fixtures

`tests/fixtures/tune/decision-schema1-inspection.json` is the exact JSON rendering
of `decision-schema1-tuned.cktune`. The test suite also freezes its exact default
text rendering as `decision-schema1-inspection.txt` and pins both SHA-256 values.
The decoder, JSON renderer, text renderer, and an independent fixture checker must
all traverse the same validated tree. Mutation tests cover key order, numeric
spelling, enum spelling, optional state, record names, list indices, escaping,
unknown nodes, omitted nodes, and trailing bytes.
