use calckernel::{CalibrationObservation, calibrate_case_observations};

#[test]
fn calibration_doubles_from_one_confirms_and_records_overshoot() {
    let attempts = vec![
        CalibrationObservation {
            iterations: 1,
            elapsed_ns: 10_000_000,
        },
        CalibrationObservation {
            iterations: 2,
            elapsed_ns: 20_000_000,
        },
        CalibrationObservation {
            iterations: 4,
            elapsed_ns: 300_000_000,
        },
        CalibrationObservation {
            iterations: 4,
            elapsed_ns: 280_000_000,
        },
    ];
    let record = calibrate_case_observations("case", &attempts).expect("calibration");
    assert_eq!(record.iterations, 4);
    assert_eq!(record.attempts, 3);
    assert!(record.overshoot);
}

#[test]
fn calibration_rejects_missing_confirmation_or_non_doubling_trace() {
    assert!(
        calibrate_case_observations(
            "case",
            &[CalibrationObservation {
                iterations: 1,
                elapsed_ns: 50_000_000
            }]
        )
        .is_err()
    );
    assert!(
        calibrate_case_observations(
            "case",
            &[
                CalibrationObservation {
                    iterations: 2,
                    elapsed_ns: 50_000_000
                },
                CalibrationObservation {
                    iterations: 2,
                    elapsed_ns: 50_000_000
                }
            ]
        )
        .is_err()
    );
}
