/// One ordered baseline calibration observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalibrationObservation {
    pub iterations: u64,
    pub elapsed_ns: u64,
}

/// Frozen per-case iteration count and calibration receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalibrationRecord {
    pub case_id: String,
    pub iterations: u64,
    pub attempts: u8,
    pub elapsed_ns: u64,
    pub confirmation_elapsed_ns: u64,
    pub overshoot: bool,
}

/// Runs exact baseline calibration and confirmation for every case in ID order.
pub fn calibrate_cases(
    runner: &super::TuneRunner,
    workload: &super::CapturedWorkload,
    baseline: &super::NonPublishableTuneTrial,
    wall_remaining_ms: u64,
) -> Result<Vec<CalibrationRecord>, super::RunnerFailure> {
    let mut records = Vec::with_capacity(workload.cases().len());
    for case in workload.cases() {
        let mut iterations = 1u64;
        let mut observations = Vec::new();
        let mut accepted = false;
        for _ in 0..32 {
            let result = runner.invoke(
                workload,
                baseline,
                &super::TuneInvocation::new(case, iterations, wall_remaining_ms),
            )?;
            observations.push(CalibrationObservation {
                iterations,
                elapsed_ns: result.elapsed_ns,
            });
            if result.elapsed_ns >= 50_000_000 {
                accepted = true;
                break;
            }
            iterations = iterations
                .checked_mul(2)
                .ok_or(super::RunnerFailure::TimerOverflow)?;
        }
        if !accepted {
            return Err(super::RunnerFailure::Calibration);
        }
        let confirmation = runner.invoke(
            workload,
            baseline,
            &super::TuneInvocation::new(case, iterations, wall_remaining_ms),
        )?;
        observations.push(CalibrationObservation {
            iterations,
            elapsed_ns: confirmation.elapsed_ns,
        });
        records.push(
            calibrate_case_observations(&case.id, &observations)
                .map_err(|_| super::RunnerFailure::Calibration)?,
        );
    }
    Ok(records)
}

/// Checks the exact doubling trace, threshold selection, and confirmation.
pub fn calibrate_case_observations(
    case_id: &str,
    observations: &[CalibrationObservation],
) -> Result<CalibrationRecord, String> {
    if case_id.is_empty() || observations.len() < 2 || observations.len() > 33 {
        return Err("invalid calibration observation count".to_string());
    }
    let mut expected = 1u64;
    let mut accepted = None;
    for (index, observation) in observations.iter().enumerate() {
        if observation.iterations != expected {
            return Err("calibration iterations are not the exact doubling sequence".to_string());
        }
        if observation.elapsed_ns >= 50_000_000 {
            accepted = Some((index, observation));
            break;
        }
        if index == 31 {
            return Err("calibration did not reach 50 ms in 32 attempts".to_string());
        }
        expected = expected
            .checked_mul(2)
            .ok_or("calibration iteration overflow")?;
    }
    let (accepted_index, accepted_observation) =
        accepted.ok_or("calibration did not contain an accepted attempt")?;
    if accepted_index >= 32 || observations.len() != accepted_index + 2 {
        return Err("calibration confirmation is missing or trailing data exists".to_string());
    }
    let confirmation = &observations[accepted_index + 1];
    if confirmation.iterations != accepted_observation.iterations {
        return Err("calibration confirmation iteration mismatch".to_string());
    }
    Ok(CalibrationRecord {
        case_id: case_id.to_string(),
        iterations: accepted_observation.iterations,
        attempts: u8::try_from(accepted_index + 1).map_err(|_| "calibration attempts overflow")?,
        elapsed_ns: accepted_observation.elapsed_ns,
        confirmation_elapsed_ns: confirmation.elapsed_ns,
        overshoot: accepted_observation.elapsed_ns > 250_000_000,
    })
}
