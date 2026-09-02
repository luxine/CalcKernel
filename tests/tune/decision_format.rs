use calckernel::{TuneBudget, TuneDecisionError, decode_tune_decision, encode_tune_decision};
use sha2::{Digest, Sha256};

use super::support;
use std::fs;
use std::path::PathBuf;

#[test]
fn tune_budget_contract_should_freeze_standard_schema_one() {
    let contract = TuneBudget::Standard.contract();

    assert_eq!(
        (
            contract.beam_width,
            contract.expansion_limit,
            contract.compile_attempt_limit,
            contract.measured_finalist_limit,
            contract.validation_entrant_limit,
            contract.wall_clock_ms,
        ),
        (8, 4_096, 16, 8, 3, 1_800_000)
    );
}

#[test]
fn tune_decision_should_reject_empty_input_as_truncated() {
    let error = decode_tune_decision(&[]).expect_err("empty input must fail");

    assert_eq!(error, TuneDecisionError::Truncated("decision header"));
}

#[test]
fn tune_decision_should_reject_wrong_outer_digest() {
    let mut bytes = empty_outer_decision();
    let last = bytes.last_mut().expect("digest byte");
    *last ^= 1;

    assert_eq!(
        decode_tune_decision(&bytes),
        Err(TuneDecisionError::DigestMismatch)
    );
}

#[test]
fn tune_decision_should_require_top_level_tags_one_through_eight() {
    let mut bytes = empty_outer_decision();
    bytes[12..14].copy_from_slice(&2u16.to_be_bytes());
    resign(&mut bytes);

    assert_eq!(
        decode_tune_decision(&bytes),
        Err(TuneDecisionError::NonCanonicalOrder("top-level records"))
    );
}

#[test]
fn tune_decision_should_validate_identity_before_accepting_outer_frame() {
    let bytes = empty_outer_decision();

    assert_eq!(
        decode_tune_decision(&bytes),
        Err(TuneDecisionError::MissingField {
            record: "Identity",
            tag: 1,
        })
    );
}

#[test]
fn tune_decision_should_parse_complete_identity_before_contract() {
    let mut payloads = vec![Vec::new(); 8];
    payloads[0] = identity_payload();
    let bytes = outer_decision(&payloads);

    assert_eq!(
        decode_tune_decision(&bytes),
        Err(TuneDecisionError::MissingField {
            record: "Contract",
            tag: 1,
        })
    );
}

#[test]
fn decision_schema_one_rejects_noncanonical_text() {
    let mut payloads = vec![Vec::new(); 8];
    payloads[0] = identity_payload_with_version("e\u{301}");
    let bytes = outer_decision(&payloads);

    assert_eq!(
        decode_tune_decision(&bytes),
        Err(TuneDecisionError::InvalidValue("Identity text"))
    );

    payloads[0] = identity_payload_with_version("/absolute/path");
    let bytes = outer_decision(&payloads);
    assert_eq!(
        decode_tune_decision(&bytes),
        Err(TuneDecisionError::InvalidValue("Identity text"))
    );
}

#[test]
fn tune_decision_should_parse_complete_contract_before_workload() {
    let mut payloads = vec![Vec::new(); 8];
    payloads[0] = identity_payload();
    payloads[1] = contract_payload();
    let bytes = outer_decision(&payloads);

    assert_eq!(
        decode_tune_decision(&bytes),
        Err(TuneDecisionError::MissingField {
            record: "Workload",
            tag: 1,
        })
    );
}

#[test]
fn decision_schema_one_accepts_complete_canonical_frame() {
    let bytes = support::baseline_decision();

    decode_tune_decision(&bytes).expect("canonical schema-1 decision must decode");
}

#[test]
fn decision_schema_one_rejects_incomplete_workload_record() {
    let mut payloads = support::decision_payloads();
    payloads[2].clear();
    support::field(&mut payloads[2], 1, &[0x31; 32]);
    let bytes = support::outer_decision(&payloads);

    assert_eq!(
        decode_tune_decision(&bytes),
        Err(TuneDecisionError::MissingField {
            record: "Workload",
            tag: 2,
        })
    );
}

#[test]
fn decision_schema_one_rejects_noncanonical_framing_and_limits() {
    let canonical = support::baseline_decision();

    let mut wrong_endian = canonical.clone();
    wrong_endian[8..12].copy_from_slice(&1u32.to_le_bytes());
    assert_eq!(
        decode_tune_decision(&wrong_endian),
        Err(TuneDecisionError::UnsupportedSchema)
    );

    let mut trailing = canonical.clone();
    trailing.insert(trailing.len() - 32, 0xff);
    support::resign(&mut trailing);
    assert_eq!(
        decode_tune_decision(&trailing),
        Err(TuneDecisionError::NonCanonicalOrder("top-level records"))
    );

    let mut payloads = support::decision_payloads();
    *payloads[6].last_mut().expect("selection optional") = 2;
    assert_eq!(
        decode_tune_decision(&support::outer_decision(&payloads)),
        Err(TuneDecisionError::InvalidValue("Selection.certificate"))
    );

    let mut payloads = support::decision_payloads();
    payloads[4][44..48].copy_from_slice(&4_097u32.to_be_bytes());
    assert_eq!(
        decode_tune_decision(&support::outer_decision(&payloads)),
        Err(TuneDecisionError::ResourceLimit("Frontier.sites"))
    );

    let oversized = vec![0; 32 * 1024 * 1024 + 1];
    assert_eq!(
        decode_tune_decision(&oversized),
        Err(TuneDecisionError::ResourceLimit("decision bytes"))
    );
}

#[test]
fn decision_round_trip_matches_canonical_bytes() {
    let bytes = support::baseline_decision();
    let decision = decode_tune_decision(&bytes).expect("canonical decision");

    decision
        .validate_self_contained()
        .expect("self-contained validation");
    assert_eq!(encode_tune_decision(&decision), bytes);
}

#[test]
fn decision_round_trip_matches_five_normative_fixture_digests() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tune");
    let expected = [
        (
            "decision-schema1-framing.hex",
            "b8712242c022ccef8a13b9b20185f33743b9af0800eb50d877b4a59787d5f63d",
        ),
        (
            "decision-schema1-baseline.cktune",
            "c4ac671ebacc766dd9d914a677103bf4919a5d84d36f9e62e9503d63d92ac213",
        ),
        (
            "decision-schema1-tuned.cktune",
            "915eaa8a2f761e5dd668e7feb245b234253881f04241338dc29dbb6af718322e",
        ),
        (
            "decision-schema1-inspection.json",
            "c9df1833857789a92d9115721193fa9461542d073619ddf08835d753f4b014d8",
        ),
        (
            "decision-schema1-inspection.txt",
            "748f54ddd0214c9940cca5e92a662553ee94943bbe53d16a410d60d9be27f8fd",
        ),
    ];
    for (name, digest) in expected {
        let bytes = fs::read(root.join(name)).expect("normative fixture must exist");
        assert_eq!(hex_digest(&bytes), digest, "fixture digest: {name}");
    }

    for name in [
        "decision-schema1-baseline.cktune",
        "decision-schema1-tuned.cktune",
    ] {
        let bytes = fs::read(root.join(name)).expect("decision fixture");
        let decision = decode_tune_decision(&bytes).expect("fixture decode");
        assert_eq!(encode_tune_decision(&decision), bytes);
    }
}

#[test]
fn decision_checker_rederives_every_self_contained_equality() {
    let mut payloads = support::decision_payloads();
    payloads[5][16] ^= 1;
    let forged_plan = support::outer_decision(&payloads);
    assert_eq!(
        decode_tune_decision(&forged_plan),
        Err(TuneDecisionError::InvalidValue("Candidate.planDigest"))
    );

    let mut payloads = support::decision_payloads();
    let selection = &mut payloads[6];
    let selected_offset = selection.len() - 45;
    selection[selected_offset] ^= 1;
    let forged_selection = support::outer_decision(&payloads);
    assert_eq!(
        decode_tune_decision(&forged_selection),
        Err(TuneDecisionError::InvalidValue(
            "Selection.selectedPlanDigest"
        ))
    );
}

fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn empty_outer_decision() -> Vec<u8> {
    outer_decision(&vec![Vec::new(); 8])
}

fn outer_decision(payloads: &[Vec<u8>]) -> Vec<u8> {
    assert_eq!(payloads.len(), 8);
    let mut bytes = b"CKTUNE01".to_vec();
    bytes.extend_from_slice(&1u32.to_be_bytes());
    for (index, payload) in payloads.iter().enumerate() {
        field(&mut bytes, u16::try_from(index + 1).expect("tag"), payload);
    }
    let mut hasher = Sha256::new();
    hasher.update(b"CK-TUNING-DECISION\0");
    hasher.update(&bytes);
    bytes.extend_from_slice(&hasher.finalize());
    bytes
}

fn identity_payload() -> Vec<u8> {
    identity_payload_with_version("0.14.0")
}

fn identity_payload_with_version(version: &str) -> Vec<u8> {
    let mut payload = Vec::new();
    field(&mut payload, 1, &text(version));
    field(&mut payload, 2, &[0x02; 32]);
    field(&mut payload, 3, &text("rustc 1.90.0"));
    field(&mut payload, 4, &text("LLVM 22.1.8"));
    field(&mut payload, 5, &[0x05; 32]);
    for (tag, value) in (6u16..=15).zip([1u32, 1, 2, 3, 3, 3, 1, 5, 1, 1]) {
        field(&mut payload, tag, &value.to_be_bytes());
    }
    for tag in 16u16..=19 {
        field(&mut payload, tag, &[u8::try_from(tag).expect("byte"); 32]);
    }
    field(&mut payload, 20, &[1]);
    let mut target = Vec::new();
    field(&mut target, 1, &text("x86_64-unknown-linux-gnu"));
    field(&mut target, 2, &text("native"));
    let mut features = Vec::new();
    features.extend_from_slice(&1u32.to_be_bytes());
    features.extend_from_slice(&text("+sse2"));
    field(&mut target, 3, &features);
    field(&mut target, 4, &text("native-profile"));
    field(&mut payload, 21, &record(&target));
    field(&mut payload, 22, &[0]);
    payload
}

fn contract_payload() -> Vec<u8> {
    let mut payload = Vec::new();
    for tag in 1u16..=5 {
        field(&mut payload, tag, &1u32.to_be_bytes());
    }
    field(&mut payload, 6, &[2]);
    for (tag, value) in (7u16..=11).zip([8u32, 4_096, 16, 8, 3]) {
        field(&mut payload, tag, &value.to_be_bytes());
    }
    field(&mut payload, 12, &1_800_000u64.to_be_bytes());
    for (tag, value) in (13u16..=14).zip([11u32, 10]) {
        field(&mut payload, tag, &value.to_be_bytes());
    }
    for (tag, value) in (15u16..=16).zip([50_000_000u64, 250_000_000]) {
        field(&mut payload, tag, &value.to_be_bytes());
    }
    for (tag, value) in (17u16..=31).zip([
        32u32, 3, 20, 3, 2_250, 4, 5, 6, 5, 16, 97, 100, 102, 100, 16,
    ]) {
        field(&mut payload, tag, &value.to_be_bytes());
    }
    let mut hasher = Sha256::new();
    hasher.update(b"CK-TUNE-POLICY\0");
    hasher.update(record(&payload));
    field(&mut payload, 32, &hasher.finalize());
    payload
}

fn field(output: &mut Vec<u8>, tag: u16, payload: &[u8]) {
    output.extend_from_slice(&tag.to_be_bytes());
    output.extend_from_slice(
        &u32::try_from(payload.len())
            .expect("fixture field length")
            .to_be_bytes(),
    );
    output.extend_from_slice(payload);
}

fn text(value: &str) -> Vec<u8> {
    let mut bytes = u32::try_from(value.len())
        .expect("fixture text length")
        .to_be_bytes()
        .to_vec();
    bytes.extend_from_slice(value.as_bytes());
    bytes
}

fn record(fields: &[u8]) -> Vec<u8> {
    let mut bytes = u32::try_from(fields.len())
        .expect("fixture record length")
        .to_be_bytes()
        .to_vec();
    bytes.extend_from_slice(fields);
    bytes
}

fn resign(bytes: &mut [u8]) {
    let split = bytes.len() - 32;
    let (body, digest) = bytes.split_at_mut(split);
    let mut hasher = Sha256::new();
    hasher.update(b"CK-TUNING-DECISION\0");
    hasher.update(body);
    digest.copy_from_slice(&hasher.finalize());
}
