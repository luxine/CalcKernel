use std::{
    process::{Child, ExitStatus},
    sync::mpsc::{self, RecvTimeoutError},
    time::Duration,
};

use super::{Containment, RunnerFailure, timer::MonotonicTimer};

#[cfg(unix)]
use super::process_unix::ExitObserver;
#[cfg(windows)]
use super::process_windows::ExitObserver;

/// Observe completion without polling or transferring ownership of the child.
/// The observer never reaps: the PID cannot be reused during timeout cleanup.
pub(super) fn wait(
    child: &mut Child,
    containment: &Containment,
    timer: MonotonicTimer,
    timeout: Duration,
) -> Result<(Option<ExitStatus>, u64), RunnerFailure> {
    let observer = match ExitObserver::new(child) {
        Ok(observer) => observer,
        Err(error) => {
            terminate(child, containment);
            let _ = child.wait();
            return Err(error.into());
        }
    };
    let (sender, receiver) = mpsc::sync_channel(1);
    let observer_thread = match std::thread::Builder::new().spawn(move || {
        let result = observer
            .wait()
            .map_err(RunnerFailure::Io)
            .and_then(|()| timer.elapsed_ns().map_err(|_| RunnerFailure::TimerOverflow));
        // The parent may already be terminating the child after its full deadline.
        let _ = sender.send(result);
    }) {
        Ok(thread) => thread,
        Err(error) => {
            terminate(child, containment);
            let _ = child.wait();
            return Err(error.into());
        }
    };
    let observed = receiver.recv_timeout(timer.remaining(timeout));
    let completed_in_time = matches!(
        &observed,
        Ok(Ok(elapsed_ns)) if Duration::from_nanos(*elapsed_ns) < timeout
    );
    if !completed_in_time {
        terminate(child, containment);
    }
    // Do not reap until the non-reaping observer has stopped accessing the PID.
    let joined = observer_thread.join();
    let status = child.wait()?;
    joined.map_err(|_| RunnerFailure::Staging)?;
    match observed {
        Ok(Ok(elapsed_ns)) if completed_in_time => Ok((Some(status), elapsed_ns)),
        Ok(Ok(_)) | Err(RecvTimeoutError::Timeout) => Ok((
            None,
            timer
                .elapsed_ns()
                .map_err(|_| RunnerFailure::TimerOverflow)?,
        )),
        Ok(Err(error)) => Err(error),
        Err(RecvTimeoutError::Disconnected) => Err(RunnerFailure::Staging),
    }
}

fn terminate(child: &mut Child, containment: &Containment) {
    containment.terminate();
    let _ = child.kill();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tune::runner::{configure_process, establish_containment};
    use std::process::{Command, Stdio};

    const PROBE: &str = "CKC_TUNE_COMPLETION_PROBE";

    #[test]
    fn completion_probe_child() {
        match std::env::var(PROBE).as_deref() {
            Ok("slow") => std::thread::sleep(Duration::from_secs(10)),
            Ok("failure") => std::process::exit(23),
            Ok("success") => std::thread::sleep(Duration::from_millis(5)),
            _ => {}
        }
    }

    fn spawn_probe(kind: &str) -> (Child, Containment, MonotonicTimer) {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "tune::runner::completion::tests::completion_probe_child",
            ])
            .env(PROBE, kind)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        configure_process(&mut command);
        let timer = MonotonicTimer::start();
        let child = command.spawn().unwrap();
        let containment = establish_containment(&child).unwrap();
        (child, containment, timer)
    }

    #[test]
    fn completion_observer_leaves_exit_status_for_parent_reaping() {
        let (mut child, _containment, _) = spawn_probe("failure");
        ExitObserver::new(&child).unwrap().wait().unwrap();
        assert_eq!(child.try_wait().unwrap().unwrap().code(), Some(23));
    }

    #[test]
    fn completion_wait_preserves_exit_status_and_external_elapsed_time() {
        for kind in ["success", "failure"] {
            let (mut child, containment, timer) = spawn_probe(kind);
            let (status, elapsed_ns) =
                wait(&mut child, &containment, timer, Duration::from_secs(5)).unwrap();
            let expected = if kind == "success" { 0 } else { 23 };
            assert_eq!(status.unwrap().code(), Some(expected));
            assert!(elapsed_ns > 0);
            assert!(elapsed_ns <= timer.elapsed_ns().unwrap());
            assert_eq!(child.try_wait().unwrap().unwrap().code(), Some(expected));
        }
    }

    #[test]
    fn completion_timeout_consumes_full_deadline_and_reaps_child() {
        let (mut child, containment, timer) = spawn_probe("slow");
        let timeout = Duration::from_millis(100);
        let (status, elapsed_ns) = wait(&mut child, &containment, timer, timeout).unwrap();
        assert!(status.is_none());
        assert!(Duration::from_nanos(elapsed_ns) >= timeout);
        assert!(child.try_wait().unwrap().is_some());
    }

    #[test]
    fn concurrent_completion_observers_do_not_reap_each_others_children() {
        let threads = (0..4)
            .map(|_| {
                std::thread::spawn(|| {
                    let (mut child, containment, timer) = spawn_probe("success");
                    let (status, _) =
                        wait(&mut child, &containment, timer, Duration::from_secs(5)).unwrap();
                    assert!(status.unwrap().success());
                    assert!(child.try_wait().unwrap().unwrap().success());
                })
            })
            .collect::<Vec<_>>();
        for thread in threads {
            thread.join().unwrap();
        }
    }
}
