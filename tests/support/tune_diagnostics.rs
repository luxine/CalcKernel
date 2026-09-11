//! Test-only post-failure observations; never a tuning result or acceptance gate.

use std::{
    fs::{File, OpenOptions},
    io::{self, Read},
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
    process::{Command, ExitStatus, Stdio},
    sync::mpsc,
    time::Instant,
};

#[path = "../../src/tune/runner/process_unix.rs"]
mod process_unix;

#[derive(Debug)]
pub(super) struct DiagnosticOutput {
    pub(super) status: ExitStatus,
    pub(super) timed_out: bool,
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
}

pub(super) fn run_bounded(
    command: &mut Command,
    root: &Path,
    label: &str,
    deadline: Instant,
) -> io::Result<DiagnosticOutput> {
    if Instant::now() >= deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "diagnostic deadline expired",
        ));
    }
    let stdout_path = root.join(format!("{label}.stdout.log"));
    let stderr_path = root.join(format!("{label}.stderr.log"));
    command
        .stdin(Stdio::null())
        .stdout(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&stdout_path)?,
        )
        .stderr(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&stderr_path)?,
        );
    process_unix::configure(command);
    let mut child = command.spawn()?;
    let containment = match process_unix::establish(&child) {
        Ok(containment) => containment,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    let observer = match process_unix::ExitObserver::new(&child) {
        Ok(observer) => observer,
        Err(error) => {
            containment.terminate();
            let _ = child.wait();
            return Err(error);
        }
    };
    let (sender, receiver) = mpsc::sync_channel(1);
    let observer_thread = match std::thread::Builder::new().spawn(move || {
        let _ = sender.send(observer.wait().map(|()| Instant::now()));
    }) {
        Ok(thread) => thread,
        Err(error) => {
            containment.terminate();
            let _ = child.wait();
            return Err(error);
        }
    };
    let observed = receiver.recv_timeout(deadline.saturating_duration_since(Instant::now()));
    let completed_in_time = matches!(&observed, Ok(Ok(at)) if *at < deadline);
    if !completed_in_time {
        // The observer has not reaped the leader. This is still our owned group,
        // including descendants which retain output handles or ignore SIGTERM.
        containment.terminate();
    }
    let joined = observer_thread.join();
    if completed_in_time {
        // SAFETY: configure created this group and establish checked the PID
        // range. The non-reaping observer leaves ownership with Child until
        // wait below, so the group ID cannot have been reused. A compiler or
        // diagnostic must not leave background descendants after its exit.
        unsafe {
            libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
        }
    }
    let status = child.wait()?;
    joined.map_err(|_| io::Error::other("diagnostic exit observer panicked"))?;
    match observed {
        Ok(Err(error)) => return Err(error),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            return Err(io::Error::other("diagnostic exit observer disconnected"));
        }
        _ => {}
    }
    Ok(DiagnosticOutput {
        status,
        timed_out: !completed_in_time,
        stdout: read_excerpt(&stdout_path)?,
        stderr: read_excerpt(&stderr_path)?,
    })
}

fn read_excerpt(path: &Path) -> io::Result<Vec<u8>> {
    // The complete file remains on disk even if a compiler emits too much to
    // include in a failed-test log. No pipe reader can outlive timeout cleanup.
    const MAXIMUM: usize = 1_048_576;
    let mut bytes = Vec::new();
    File::open(path)?
        .take((MAXIMUM + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAXIMUM {
        return Err(io::Error::other(format!(
            "diagnostic output exceeds display bound; full file: {}",
            path.display()
        )));
    }
    Ok(bytes)
}

pub(super) fn after_failure(succeeded: bool, diagnostic: impl FnOnce() -> Result<(), String>) {
    if succeeded {
        return;
    }
    match catch_unwind(AssertUnwindSafe(diagnostic)) {
        Ok(Ok(())) => {}
        Ok(Err(error)) => eprintln!("post-failure startup diagnostic incomplete: {error}"),
        Err(_) => {
            eprintln!("post-failure startup diagnostic panicked; original failure is unchanged")
        }
    }
}

#[cfg(target_os = "macos")]
pub(super) fn collect_startup(
    root: &Path,
    source: &Path,
    compiler: &Path,
    original_runner: &Path,
) -> io::Result<()> {
    use sha2::{Digest, Sha256};
    use std::{fs, time::Duration};

    // Reserve the existing containment's 2.25-second cleanup grace within a
    // 60-second diagnostic budget. This never changes an original test timer.
    let deadline = Instant::now() + Duration::from_secs(57);
    let evidence = root.join("post-failure-startup");
    fs::create_dir(&evidence)?;
    fs::copy(source, evidence.join("comparator-source.ck"))?;
    fs::copy(original_runner, evidence.join("original-runner"))?;
    let c_source = evidence.join("diagnostic-supervisor.c");
    fs::write(
        &c_source,
        include_str!("../fixtures/tune/cli-executable-diagnostic.c"),
    )?;
    let supervisor = evidence.join("diagnostic-supervisor");
    eprintln!(
        "post-failure startup evidence: {} (diagnostic only; not the failed candidate)",
        evidence.display()
    );
    run_reported(
        Command::new("cc")
            .args(["-std=c11", "-Wall", "-Wextra", "-Werror"])
            .arg(&c_source)
            .arg("-o")
            .arg(&supervisor),
        &evidence,
        "compile-supervisor",
        deadline,
    )?;
    let compiler_digest = hash_file(compiler)?;
    let artifact = evidence.join("ordinary-program");
    let compiler_home = evidence.join("compiler-home");
    fs::create_dir(&compiler_home)?;
    run_reported(
        Command::new(compiler)
            .arg("build")
            .arg(source)
            .args(["--kind", "executable", "--cpu", "native", "-O3", "--out"])
            .arg(&artifact)
            .env("HOME", &compiler_home),
        &evidence,
        "compile-program",
        deadline,
    )?;
    let supervisor_bytes = fs::read(&supervisor)?;
    let artifact_bytes = fs::read(&artifact)?;
    let manifest = format!(
        "CKTUNE-STARTUP-COMPARATOR/2\ndiagnosticOnly=true\nfailedCandidate=false\nartifactKind=ordinary-native-O3-executable\nos={}\narch={}\nwarmupRows=3\nsampleRows=20\ncallsPerRow=3\niterationsPerCall=16\nworkDeadlineSeconds=57\ncleanupReserveSeconds=3\ncompiler={compiler:?}\ncompilerSha256={compiler_digest}\nsourceSha256={}\noriginalRunnerSha256={}\nsupervisorSha256={:x}\nartifactSha256={:x}\nexpectedOutputSha256={:x}\ncolumns=iteration,childPid,supervisorPgid,forkReturnNs,firstByteNsOrZero,eofNs,reapedNs,userCpuNs,systemCpuNs,stdoutBytes,valid,rawWaitStatus\nresourceRows=one-per-child\nresourceColumns=iteration,childPid,exitObservationResult,exitObservationErrno,exitObservedNs,rusageResult,rusageErrno,resourceQueryDoneNs,machTimebaseNumer,machTimebaseDenom,userMachTicks,systemMachTicks,instructions,cycles,pageins,runnableMachTicks,childUserMachTicks,childSystemMachTicks,processStartMachTicks,processExitMachTicks,executableUuidHex\nresourceLimitations=optional exited-child V4; convert raw Mach ticks with numer/denom; zero instruction/cycle counters mean unavailable; retain wait4 separately; exit observation and query overhead are included before reapedNs\nlimitation=parent-observed phases include scheduling and pipe observation; this separate ordinary artifact is not an exact failed-plan reproduction\n",
        std::env::consts::OS,
        std::env::consts::ARCH,
        hash_file(source)?,
        hash_file(original_runner)?,
        Sha256::digest(&supervisor_bytes),
        Sha256::digest(&artifact_bytes),
        Sha256::digest(b"66\n"),
    );
    fs::write(evidence.join("manifest.txt"), &manifest)?;
    eprintln!("{manifest}");
    for row in 0..23 {
        let kind = if row < 3 { "warmup" } else { "sample" };
        let ordinal = if row < 3 { row } else { row - 3 };
        for call in 0..3 {
            let label = format!("{kind}-{ordinal:02}-call-{call}");
            let invocation = evidence.join(&label);
            fs::create_dir(&invocation)?;
            let fresh_supervisor = invocation.join("supervisor");
            let fresh_artifact = invocation.join("artifact");
            write_executable(&fresh_supervisor, &supervisor_bytes)?;
            write_executable(&fresh_artifact, &artifact_bytes)?;
            run_reported(
                Command::new(&fresh_supervisor)
                    .arg(&fresh_artifact)
                    .arg("16")
                    .env_clear()
                    .current_dir(&invocation),
                &invocation,
                &label,
                deadline,
            )?;
        }
    }
    eprintln!(
        "post-failure startup cohort complete: 69 calls, 1104 real child executions; original result unchanged"
    );
    Ok(())
}

#[cfg(target_os = "macos")]
fn run_reported(
    command: &mut Command,
    root: &Path,
    label: &str,
    deadline: Instant,
) -> io::Result<()> {
    let start = Instant::now();
    let output = run_bounded(command, root, label, deadline)?;
    eprintln!(
        "post-failure startup call={label} elapsedNs={} status={} timedOut={}\nstdout:\n{}stderr:\n{}",
        start.elapsed().as_nanos(),
        output.status,
        output.timed_out,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    if output.timed_out || !output.status.success() {
        return Err(io::Error::other(format!(
            "{label} did not complete successfully; raw files retained in {}",
            root.display()
        )));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub(super) fn write_executable(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::{fs, io::Write, os::unix::fs::PermissionsExt};
    // Write new bytes/inodes, as the original runner does, rather than clone a
    // potentially prevalidated executable image between diagnostic calls.
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(target_os = "macos")]
fn hash_file(path: &Path) -> io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut stream = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
