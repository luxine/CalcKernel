mod protocol;
mod timer;

#[cfg(unix)]
mod process_unix;
#[cfg(windows)]
mod process_windows;

use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use super::{CapturedWorkload, NonPublishableTuneTrial, TuneCase, stage_invocation_inputs};

static INVOCATION_ORDINAL: AtomicU64 = AtomicU64::new(0);

/// Exact invocation coordinate and wall-budget admission material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuneInvocation {
    pub case_id: String,
    pub seed: u64,
    pub iterations: u64,
    pub expected_digest: [u8; 32],
    pub wall_remaining_ms: u64,
    pub candidate: bool,
}

impl TuneInvocation {
    #[must_use]
    pub fn new(case: &TuneCase, iterations: u64, wall_remaining_ms: u64) -> Self {
        Self {
            case_id: case.id.clone(),
            seed: case.seed,
            iterations,
            expected_digest: case.expected_digest,
            wall_remaining_ms,
            candidate: false,
        }
    }

    #[must_use]
    pub const fn candidate(mut self, candidate: bool) -> Self {
        self.candidate = candidate;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvocationResult {
    pub elapsed_ns: u64,
    pub completed: u64,
    pub digest: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalCandidateTimeout {
    pub case_id: String,
    pub iterations: u64,
    pub timeout_ms: u32,
    pub elapsed_ns: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum RunnerFailure {
    #[error("insufficient wall budget for one complete invocation")]
    WallBudgetAdmission,
    #[error("runner process I/O failed")]
    Io(#[from] std::io::Error),
    #[error("runner snapshot or staging failed")]
    Staging,
    #[error("runner containment could not be established")]
    ProcessSetup,
    #[error("runner exited unsuccessfully with status {0:?}")]
    NonZero(Option<i32>),
    #[error("runner stdout exceeded 4 KiB")]
    StdoutLimit,
    #[error("runner stderr exceeded 1 MiB")]
    StderrLimit,
    #[error("runner emitted an invalid CKTUNE/1 record")]
    Protocol,
    #[error("runner correctness digest mismatch")]
    Correctness,
    #[error("baseline runner timed out")]
    BaselineTimeout,
    #[error("candidate runner reached its complete configured timeout")]
    CandidateTimeout(CanonicalCandidateTimeout),
    #[error("monotonic elapsed time overflow")]
    TimerOverflow,
    #[error("baseline calibration did not satisfy the frozen contract")]
    Calibration,
}

#[derive(Debug, Default)]
pub struct TuneRunner;

impl TuneRunner {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    pub fn invoke(
        &self,
        workload: &CapturedWorkload,
        trial: &NonPublishableTuneTrial,
        invocation: &TuneInvocation,
    ) -> Result<InvocationResult, RunnerFailure> {
        let timeout_ms = workload.timeout_ms();
        if invocation.wall_remaining_ms < u64::from(timeout_ms) + 2_250 {
            return Err(RunnerFailure::WallBudgetAdmission);
        }
        let run = PrivateRun::create()?;
        let runner_path = run.path.join(if cfg!(windows) {
            "runner.exe"
        } else {
            "runner"
        });
        write_new(&runner_path, workload.runner_bytes(), true)?;
        let artifact_path = run.path.join(if cfg!(windows) {
            "artifact.exe"
        } else {
            "artifact"
        });
        trial
            .stage_primary_for_measurement(&artifact_path)
            .map_err(|_| RunnerFailure::Staging)?;
        let inputs = stage_invocation_inputs(workload, &run.path.join("run"))
            .map_err(|_| RunnerFailure::Staging)?;

        let mut command = Command::new(&runner_path);
        command
            .args(workload.args())
            .env_clear()
            .current_dir(&run.path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (name, value) in workload.environment_values() {
            command.env(name, bytes_to_os(value)?);
        }
        command
            .env("CK_TUNE_PROTOCOL", "1")
            .env("CK_TUNE_ARTIFACT", &artifact_path)
            .env("CK_TUNE_ARTIFACT_KIND", trial.artifact_kind_name())
            .env("CK_TUNE_CASE", &invocation.case_id)
            .env("CK_TUNE_SEED", invocation.seed.to_string())
            .env("CK_TUNE_ITERATIONS", invocation.iterations.to_string())
            .env("CK_TUNE_TEMP", &run.path)
            .env("CK_TUNE_INPUT_MAP", inputs.map_path());
        configure_process(&mut command);
        let mut child = command.spawn()?;
        let containment = establish_containment(&child).map_err(|_| RunnerFailure::ProcessSetup)?;
        let stdout = child.stdout.take().ok_or(RunnerFailure::Staging)?;
        let stderr = child.stderr.take().ok_or(RunnerFailure::Staging)?;
        let stdout_reader = read_bounded(stdout, 4_096);
        let stderr_reader = read_bounded(stderr, 1_048_576);
        let timer = timer::MonotonicTimer::start();
        let timeout = Duration::from_millis(u64::from(timeout_ms));
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break Some(status);
            }
            if timer.reached(timeout) {
                containment.terminate();
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        let stdout = stdout_reader.join().map_err(|_| RunnerFailure::Staging)??;
        let stderr = stderr_reader.join().map_err(|_| RunnerFailure::Staging)??;
        if stdout.1 {
            return Err(RunnerFailure::StdoutLimit);
        }
        if stderr.1 {
            return Err(RunnerFailure::StderrLimit);
        }
        let Some(status) = status else {
            if invocation.candidate {
                let elapsed_ns = timer
                    .elapsed_ns()
                    .map_err(|_| RunnerFailure::TimerOverflow)?;
                return Err(RunnerFailure::CandidateTimeout(CanonicalCandidateTimeout {
                    case_id: invocation.case_id.clone(),
                    iterations: invocation.iterations,
                    timeout_ms,
                    elapsed_ns,
                }));
            }
            return Err(RunnerFailure::BaselineTimeout);
        };
        if !status.success() {
            return Err(RunnerFailure::NonZero(status.code()));
        }
        protocol::parse(
            &stdout.0,
            invocation,
            timer
                .elapsed_ns()
                .map_err(|_| RunnerFailure::TimerOverflow)?,
        )
    }
}

fn read_bounded(
    mut stream: impl Read + Send + 'static,
    limit: u64,
) -> std::thread::JoinHandle<Result<(Vec<u8>, bool), std::io::Error>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stream.by_ref().take(limit + 1).read_to_end(&mut bytes)?;
        let overflow = u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit;
        if overflow {
            bytes.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
        }
        Ok((bytes, overflow))
    })
}

fn write_new(path: &Path, bytes: &[u8], _executable: bool) -> Result<(), RunnerFailure> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    #[cfg(unix)]
    if _executable {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn bytes_to_os(value: &[u8]) -> Result<OsString, RunnerFailure> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(OsString::from_vec(value.to_vec()))
    }
    #[cfg(windows)]
    {
        Ok(OsString::from(
            std::str::from_utf8(value).map_err(|_| RunnerFailure::Staging)?,
        ))
    }
}

fn configure_process(command: &mut Command) {
    #[cfg(unix)]
    process_unix::configure(command);
    #[cfg(windows)]
    process_windows::configure(command);
}

fn establish_containment(child: &std::process::Child) -> Result<Containment, std::io::Error> {
    #[cfg(unix)]
    {
        process_unix::establish(child)
    }
    #[cfg(windows)]
    {
        process_windows::establish(child)
    }
}

#[cfg(unix)]
type Containment = process_unix::Containment;
#[cfg(windows)]
type Containment = process_windows::Containment;

struct PrivateRun {
    path: PathBuf,
}

impl PrivateRun {
    fn create() -> Result<Self, std::io::Error> {
        let ordinal = INVOCATION_ORDINAL.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("ckc-tune-{}-{ordinal:016x}", std::process::id()));
        fs::create_dir(&path)?;
        let path = fs::canonicalize(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        }
        Ok(Self { path })
    }
}

impl Drop for PrivateRun {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
