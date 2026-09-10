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

fn unstable_decision(phase: u8, candidate: bool) -> (Vec<u8>, String) {
    let mut top = fixture::top();
    let mut candidates = fixture::fields(&top[5]);
    let mut trials = fixture::records(&candidates[1]);
    let mut baseline = fixture::record_fields(&candidates[0]);
    let selected = if candidate {
        trials
            .iter_mut()
            .find(|trial| trial[5][0] >= 6)
            .expect("validation entrant")
    } else {
        &mut baseline
    };
    let mut streams = fixture::records(&selected[8]);
    let case = if phase == 3 { "search" } else { "validation-b" };
    let stream = streams
        .iter_mut()
        .find(|stream| stream[0] == [phase] && stream[2] == support::text(case))
        .expect("requested measured stream");
    let mut rows = fixture::records(&stream[5]);
    let minima = std::array::from_fn::<_, 20, _>(|index| {
        if index < 5 {
            200_000_000u64
        } else {
            100_000_000
        }
    });
    let calls = minima.map(|minimum| [minimum + 7, minimum, minimum + 3]);
    for (index, row) in rows.iter_mut().enumerate() {
        row[2] = support::list(&calls[index].map(|call| call.to_be_bytes().to_vec()));
        row[3] = minima[index].to_be_bytes().to_vec();
    }
    stream[5] = fixture::record_list(&rows);
    let plan = stream[3]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let round = stream[1][0];
    let expected = format!(
        "invalid value for MeasurementStream.selection stability: phase={phase} round={round} case={case:?} caseBytes={} caseTruncated=false plan={plan} iterations=1 inRange=15/20 required=16 upperMedianNs=100000000 minimaNs={minima:?} callsNs={calls:?}",
        case.len(),
    );
    selected[8] = fixture::record_list(&streams);
    candidates[0] = fixture::record(&baseline);
    candidates[1] = fixture::record_list(&trials);
    top[5] = fixture::pack(&candidates);
    (fixture::reframe(top), expected)
}

#[test]
fn decision_policy_stability_diagnostic_reports_search_evidence() {
    let (bytes, expected) = unstable_decision(3, false);
    assert_eq!(
        decode_tune_decision(&bytes)
            .expect_err("five unstable search rows")
            .to_string(),
        expected,
    );
}

#[test]
fn decision_policy_stability_diagnostic_reports_first_validation_evidence() {
    let (bytes, expected) = unstable_decision(5, false);
    assert_eq!(
        decode_tune_decision(&bytes)
            .expect_err("five unstable first-round rows")
            .to_string(),
        expected,
    );
}

#[test]
fn decision_policy_stability_diagnostic_reports_second_validation_evidence() {
    let (bytes, expected) = unstable_decision(7, false);
    assert_eq!(
        decode_tune_decision(&bytes)
            .expect_err("five unstable second-round rows")
            .to_string(),
        expected,
    );
}

#[test]
fn decision_policy_stability_diagnostic_reports_candidate_evidence() {
    let (bytes, expected) = unstable_decision(3, true);
    assert_eq!(
        decode_tune_decision(&bytes)
            .expect_err("five unstable candidate rows")
            .to_string(),
        expected,
    );
}

#[test]
fn decision_policy_stability_diagnostic_reaches_cli_stderr_without_mutating_input() {
    let (bytes, expected) = unstable_decision(7, false);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!(
            "tune-stability-diagnostic-{}",
            super::temp::unique_id()
        ));
    std::fs::create_dir(&root).expect("new private fixture directory");
    let input = root.join("invalid.cktune");
    std::fs::write(&input, &bytes).expect("invalid synthetic decision");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ckc"))
        .args(["tune", "inspect"])
        .arg(&input)
        .arg("--json")
        .output()
        .expect("inspect without compiling or running artifacts");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "failure must not emit JSON");
    assert_eq!(
        String::from_utf8(output.stderr).expect("diagnostic UTF-8"),
        format!("{expected}\n"),
    );
    assert_eq!(std::fs::read(&input).expect("retained input"), bytes);
    assert_eq!(
        std::fs::read_dir(&root).expect("fixture directory").count(),
        1
    );
    std::fs::remove_dir_all(&root).expect("remove only the owned fixture");
}

fn rename_unstable_search_case(bytes: &[u8], id: &str, maximum_timings: bool) -> Vec<u8> {
    let mut top = fixture::fields(&bytes[12..bytes.len() - 32]);
    let original = support::text("search");
    let replacement = support::text(id);
    let mut workload = fixture::fields(&top[2]);
    let mut cases = fixture::records(&workload[7]);
    cases
        .iter_mut()
        .find(|case| case[0] == original)
        .expect("search identity")[0] = replacement.clone();
    workload[7] = fixture::record_list(&cases);
    top[2] = fixture::pack(&workload);
    let mut environment = fixture::fields(&top[3]);
    let mut calibrations = fixture::records(&environment[16]);
    calibrations
        .iter_mut()
        .find(|calibration| calibration[0] == original)
        .expect("search calibration")[0] = replacement.clone();
    environment[16] = fixture::record_list(&calibrations);
    top[3] = fixture::pack(&environment);
    mutate_baseline_streams(&mut top, |streams| {
        let stream = streams
            .iter_mut()
            .find(|stream| stream[0] == [3])
            .expect("unstable baseline search");
        stream[2] = replacement;
        if maximum_timings {
            let mut rows = fixture::records(&stream[5]);
            for (index, row) in rows.iter_mut().enumerate() {
                let minimum = if index < 5 {
                    u64::MAX - 7
                } else {
                    u64::MAX / 2
                };
                row[2] = support::list(
                    &[minimum + 7, minimum, minimum + 3].map(|call| call.to_be_bytes().to_vec()),
                );
                row[3] = minimum.to_be_bytes().to_vec();
            }
            stream[5] = fixture::record_list(&rows);
        }
    });
    fixture::reframe(top)
}

#[test]
fn decision_policy_stability_diagnostic_escapes_case_name_control_characters() {
    let (bytes, _) = unstable_decision(3, false);
    let bytes = rename_unstable_search_case(&bytes, "search\n\u{1b}[2J\"\\", false);
    let diagnostic = decode_tune_decision(&bytes)
        .expect_err("unstable stream with hostile case text")
        .to_string();
    assert!(
        diagnostic.contains("case=\"search\\n\\u{1b}[2J\\\"\\\\\""),
        "{diagnostic}"
    );
    assert_eq!(diagnostic.lines().count(), 1);
    assert!(!diagnostic.contains('\u{1b}'));
}

#[test]
fn decision_policy_stability_diagnostic_bounds_names_without_truncating_raw_timings() {
    let (bytes, _) = unstable_decision(3, false);
    let name = format!("s{}", "\u{202e}".repeat(1365));
    assert_eq!(name.len(), 4096, "maximum decoded text length");
    let bytes = rename_unstable_search_case(&bytes, &name, true);
    let diagnostic = decode_tune_decision(&bytes)
        .expect_err("unstable maximum-width timings")
        .to_string();
    assert!(
        diagnostic.contains("caseBytes=4096 caseTruncated=true"),
        "{diagnostic}"
    );
    assert!(diagnostic.contains(&format!("upperMedianNs={}", u64::MAX / 2)));
    let calls = std::array::from_fn::<_, 20, _>(|index| {
        let minimum = if index < 5 {
            u64::MAX - 7
        } else {
            u64::MAX / 2
        };
        [minimum + 7, minimum, minimum + 3]
    });
    assert!(
        diagnostic.ends_with(&format!("callsNs={calls:?}")),
        "{diagnostic}"
    );
    assert!(diagnostic.is_ascii(), "invisible Unicode must be escaped");
    assert_eq!(diagnostic.lines().count(), 1);
    assert!(
        diagnostic.len() < 4096,
        "diagnostic bytes: {}",
        diagnostic.len()
    );
}

#[test]
fn decision_policy_stability_diagnostic_preserves_sixty_four_character_names() {
    let (bytes, _) = unstable_decision(3, false);
    for length in [64, 65] {
        let name = "s".repeat(length);
        let mutated = rename_unstable_search_case(&bytes, &name, false);
        let diagnostic = decode_tune_decision(&mutated)
            .expect_err("unstable stream with boundary-length name")
            .to_string();
        assert!(
            diagnostic.contains(&format!(
                "case={:?} caseBytes={length} caseTruncated={}",
                "s".repeat(64),
                length == 65,
            )),
            "{diagnostic}"
        );
    }
}

#[test]
fn decision_policy_stability_diagnostic_reports_upper_not_lower_median() {
    let (bytes, _) = unstable_decision(3, false);
    let mut top = fixture::fields(&bytes[12..bytes.len() - 32]);
    mutate_baseline_streams(&mut top, |streams| {
        let mut rows = fixture::records(&streams[0][5]);
        for (index, row) in rows.iter_mut().enumerate() {
            replace_calls(row, if index < 10 { 60_000_000 } else { 100_000_000 });
        }
        streams[0][5] = fixture::record_list(&rows);
    });
    let diagnostic = decode_tune_decision(&fixture::reframe(top))
        .expect_err("ten samples outside the upper-median interval")
        .to_string();
    assert!(
        diagnostic.contains("inRange=10/20 required=16 upperMedianNs=100000000"),
        "{diagnostic}"
    );
}

#[test]
fn decision_policy_stability_diagnostic_preserves_valid_current_decision_bytes() {
    for reason in [
        calckernel::SelectionReason::NoCandidate,
        calckernel::SelectionReason::ValidationThreshold,
        calckernel::SelectionReason::ValidationDisagreement,
        calckernel::SelectionReason::Tuned,
    ] {
        let bytes = fixture::encoded_for_reason(reason);
        let decision = decode_tune_decision(&bytes).expect("valid current decision");
        assert_eq!(encode_tune_decision(&decision), bytes);
    }
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
            let error = result.expect_err("fifteen stable rows must still fail");
            assert!(matches!(
                error,
                TuneDecisionError::UnstableSelectionStream(_)
            ));
            assert!(
                error
                    .to_string()
                    .contains("inRange=15/20 required=16 upperMedianNs=100000000")
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
