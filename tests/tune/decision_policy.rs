use calckernel::{
    TuneDecisionError, decode_tune_decision, encode_tune_decision, inspect_tune_json,
};
use sha2::{Digest, Sha256};

use super::support;
#[path = "policy_fixture.rs"]
mod fixture;

#[test]
fn decision_policy_current_encoder_binds_current_contract_and_valid_replay() {
    let bytes = fixture::encoded();
    let top = fixture::fields(&bytes[12..bytes.len() - 32]);
    assert_eq!(fixture::fields(&top[1])[1], 2u32.to_be_bytes());
    decode_tune_decision(&bytes)
        .expect("current decision")
        .replay_requirements()
        .expect("current replay");
}

#[test]
fn decision_policy_checker_accepts_each_unchanged_selection_table_result() {
    use calckernel::SelectionReason;
    for reason in [
        SelectionReason::NoCandidate,
        SelectionReason::ValidationThreshold,
        SelectionReason::ValidationDisagreement,
        SelectionReason::Tuned,
    ] {
        let bytes = fixture::encoded_for_reason(reason);
        decode_tune_decision(&bytes)
            .expect("independently validated table result")
            .replay_requirements()
            .expect("replayable contract 2");
    }
}

#[test]
fn decision_policy_checker_rejects_re_signed_summary_mutations() {
    let original = fixture::top();
    for field in [2, 3, 4, 5] {
        let mut top = original.clone();
        let mut selection = fixture::fields(&top[6]);
        let mut round = fixture::record_fields(&selection[0]);
        let mut plans = fixture::records(&round[1]);
        let changed = plans[0][field].last_mut().expect("derived field");
        *changed ^= 1;
        round[1] = fixture::record_list(&plans);
        selection[0] = fixture::record(&round);
        top[6] = fixture::pack(&selection);
        assert!(
            decode_tune_decision(&fixture::reframe(top)).is_err(),
            "accepted forged RoundPlan field {}",
            field + 1
        );
    }
}

#[test]
fn decision_policy_checker_rejects_re_signed_raw_median_mutation() {
    let mut top = fixture::top();
    let mut selection = fixture::fields(&top[6]);
    let mut round = fixture::record_fields(&selection[0]);
    let mut plans = fixture::records(&round[1]);
    let mut medians = fixture::records(&plans[0][1]);
    medians[0][2][7] ^= 1;
    plans[0][1] = fixture::record_list(&medians);
    round[1] = fixture::record_list(&plans);
    selection[0] = fixture::record(&round);
    top[6] = fixture::pack(&selection);
    assert!(
        decode_tune_decision(&fixture::reframe(top)).is_err(),
        "accepted median unrelated to raw samples"
    );
}

#[test]
fn decision_policy_checker_rejects_re_signed_qualified_rank_swap() {
    let mut top = fixture::top();
    let mut selection = fixture::fields(&top[6]);
    let mut round = fixture::record_fields(&selection[0]);
    assert!(round[2].len() >= 68, "fixture has two qualifiers");
    let first = round[2][4..36].to_vec();
    let second = round[2][36..68].to_vec();
    round[2][4..36].copy_from_slice(&second);
    round[2][36..68].copy_from_slice(&first);
    selection[0] = fixture::record(&round);
    top[6] = fixture::pack(&selection);
    assert!(
        decode_tune_decision(&fixture::reframe(top)).is_err(),
        "accepted forged qualifying rank"
    );
}

#[test]
fn decision_policy_checker_rejects_re_signed_omitted_entrant() {
    let mut top = fixture::top();
    let mut selection = fixture::fields(&top[6]);
    for index in [0, 1] {
        let mut round = fixture::record_fields(&selection[index]);
        let mut plans = fixture::records(&round[1]);
        plans.pop().expect("entrant");
        round[1] = fixture::record_list(&plans);
        selection[index] = fixture::record(&round);
    }
    top[6] = fixture::pack(&selection);
    assert!(
        decode_tune_decision(&fixture::reframe(top)).is_err(),
        "accepted incomplete entrant set"
    );
}

fn mutate_baseline_streams(top: &mut [Vec<u8>], mutate: impl FnOnce(&mut Vec<Vec<Vec<u8>>>)) {
    let mut candidates = fixture::fields(&top[5]);
    let mut baseline = fixture::record_fields(&candidates[0]);
    let mut streams = fixture::records(&baseline[8]);
    mutate(&mut streams);
    baseline[8] = fixture::record_list(&streams);
    candidates[0] = fixture::record(&baseline);
    top[5] = fixture::pack(&candidates);
}

#[test]
fn decision_policy_checker_rejects_re_signed_stream_identity_mutations() {
    let original = fixture::top();
    for (field, replacement) in [
        (1, vec![1]),
        (3, vec![0; 32]),
        (4, 2u64.to_be_bytes().to_vec()),
        (6, vec![0; 32]),
    ] {
        let mut top = original.clone();
        mutate_baseline_streams(&mut top, |streams| streams[0][field] = replacement);
        assert_eq!(
            decode_tune_decision(&fixture::reframe(top)),
            Err(TuneDecisionError::InvalidValue(
                "MeasurementStream.selection identity"
            )),
            "stream field {}",
            field + 1
        );
    }
}

#[test]
fn decision_policy_checker_rejects_duplicate_and_missing_raw_streams() {
    let original = fixture::top();
    for duplicate in [true, false] {
        let mut top = original.clone();
        mutate_baseline_streams(&mut top, |streams| {
            if duplicate {
                streams.push(streams[0].clone());
            } else {
                streams.remove(0);
            }
        });
        assert_eq!(
            decode_tune_decision(&fixture::reframe(top)),
            Err(TuneDecisionError::InvalidValue(if duplicate {
                "MeasurementStream.selection duplicate"
            } else {
                "Candidate.incomplete selection streams"
            }))
        );
    }
}

#[test]
fn decision_policy_checker_requires_baseline_validation_even_without_entrants() {
    let bytes = fixture::encoded_for_reason(calckernel::SelectionReason::NoCandidate);
    let mut top = fixture::fields(&bytes[12..bytes.len() - 32]);
    mutate_baseline_streams(&mut top, |streams| {
        streams.retain(|stream| stream[0] == [3]);
    });
    assert_eq!(
        decode_tune_decision(&fixture::reframe(top)).map(|_| ()),
        Err(TuneDecisionError::InvalidValue(
            "Candidate.incomplete selection streams"
        ))
    );
}

#[test]
fn decision_policy_checker_rejects_re_signed_duplicate_row_ordinals() {
    let mut top = fixture::top();
    mutate_baseline_streams(&mut top, |streams| {
        let mut rows = fixture::records(&streams[0][5]);
        rows[1][0] = rows[0][0].clone();
        streams[0][5] = fixture::record_list(&rows);
    });
    assert_eq!(
        decode_tune_decision(&fixture::reframe(top)),
        Err(TuneDecisionError::InvalidValue(
            "MeasurementRow.selection evidence"
        ))
    );
}

fn replace_calls(row: &mut [Vec<u8>], ns: u64) {
    row[2] = support::list(&[ns.to_be_bytes(); 3].map(|call| call.to_vec()));
    row[3] = ns.to_be_bytes().to_vec();
}

#[test]
fn decision_policy_checker_preserves_exact_sixteen_of_twenty_stability() {
    let original = fixture::top();
    for outliers in [4, 5] {
        let mut top = original.clone();
        mutate_baseline_streams(&mut top, |streams| {
            let mut rows = fixture::records(&streams[0][5]);
            for row in rows.iter_mut().take(outliers) {
                replace_calls(row, 200_000_000);
            }
            streams[0][5] = fixture::record_list(&rows);
        });
        let result = decode_tune_decision(&fixture::reframe(top));
        if outliers == 4 {
            result.expect("exactly sixteen stable baseline search rows");
        } else {
            assert_eq!(
                result,
                Err(TuneDecisionError::InvalidValue(
                    "MeasurementStream.selection stability"
                ))
            );
        }
    }
}

#[test]
fn decision_policy_checker_rejects_raw_ratio_overflow_without_panicking() {
    let mut top = fixture::top();
    let mut candidates = fixture::fields(&top[5]);
    let mut trials = fixture::records(&candidates[1]);
    let measured = trials
        .iter_mut()
        .find(|trial| trial[5][0] >= 5)
        .expect("measured trial");
    let mut streams = fixture::records(&measured[8]);
    let mut rows = fixture::records(&streams[0][5]);
    for row in &mut rows {
        replace_calls(row, u64::MAX);
    }
    streams[0][5] = fixture::record_list(&rows);
    measured[8] = fixture::record_list(&streams);
    candidates[1] = fixture::record_list(&trials);
    top[5] = fixture::pack(&candidates);
    assert_eq!(
        decode_tune_decision(&fixture::reframe(top)),
        Err(TuneDecisionError::InvalidValue("Selection.ratio overflow"))
    );
}

#[test]
fn decision_policy_checker_binds_session_to_current_contract() {
    let mut top = fixture::top();
    let mut environment = fixture::fields(&top[3]);
    environment[17][0] ^= 1;
    top[3] = fixture::pack(&environment);
    assert!(decode_tune_decision(&fixture::reframe(top)).is_err());
}

fn contract_payload(version: u32) -> Vec<u8> {
    let mut contract = support::contract_payload();
    contract[16..20].copy_from_slice(&version.to_be_bytes());
    let split = contract.len() - 38;
    let mut hash = Sha256::new();
    hash.update(b"CK-TUNE-POLICY\0");
    hash.update(support::record(&contract[..split]));
    contract[split + 6..].copy_from_slice(&hash.finalize());
    contract
}

#[test]
fn decision_policy_accepts_current_contract_before_requiring_workload() {
    let mut payloads = support::decision_payloads();
    payloads[1] = contract_payload(2);
    payloads[2].clear();
    assert_eq!(
        decode_tune_decision(&support::outer_decision(&payloads)),
        Err(TuneDecisionError::MissingField {
            record: "Workload",
            tag: 1
        })
    );
}

#[test]
fn decision_policy_rejects_unknown_contract_versions() {
    for version in [0, 3, u32::MAX] {
        let mut payloads = support::decision_payloads();
        payloads[1] = contract_payload(version);
        assert_eq!(
            decode_tune_decision(&support::outer_decision(&payloads)),
            Err(TuneDecisionError::InvalidValue("Contract.contractSchema"))
        );
    }
}

#[test]
fn decision_policy_rejects_current_contract_with_legacy_policy_digest() {
    let mut payloads = support::decision_payloads();
    payloads[1][16..20].copy_from_slice(&2u32.to_be_bytes());
    assert_eq!(
        decode_tune_decision(&support::outer_decision(&payloads)),
        Err(TuneDecisionError::DigestMismatch)
    );
}

#[test]
fn decision_policy_legacy_contract_is_inspection_only_and_preserves_bytes() {
    let bytes = include_bytes!("../fixtures/tune/decision-schema1-tuned.cktune");
    let decision = decode_tune_decision(bytes).expect("legacy inspection");
    assert_eq!(encode_tune_decision(&decision), bytes);
    assert!(
        inspect_tune_json(&decision)
            .expect("inspection")
            .contains("record:Contract")
    );
    assert!(
        decision.replay_requirements().is_err(),
        "legacy selection cannot be replayed as contract 2"
    );
}
