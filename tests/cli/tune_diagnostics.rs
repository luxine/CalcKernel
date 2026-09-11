use super::{compile_tune_c_fixture, diagnostics, temp_dir, unique_id};
use std::{
    fs,
    process::Command,
    time::{Duration, Instant},
};

// These are functional observations, not a five-second performance gate.
// Match the existing real executable CLI fixture's containment ceiling so a
// loaded host can finish cold image startup without losing required records.
const PHASE_PROBE_TIMEOUT: Duration = Duration::from_secs(30);

fn phase_probe(root: &std::path::Path) -> std::path::PathBuf {
    compile_tune_c_fixture(
        root,
        "phase-probe",
        include_str!("../fixtures/tune/cli-executable-diagnostic.c"),
        Command::new("cc"),
    )
}

fn phase_records(bytes: &[u8]) -> Vec<Vec<u64>> {
    std::str::from_utf8(bytes)
        .expect("phase UTF-8")
        .lines()
        .map(|line| {
            let mut columns = line.split('\t');
            assert_eq!(columns.next(), Some("CKTUNE-CHILD/1"));
            let values = columns
                .map(|value| value.parse::<u64>().expect("phase integer"))
                .collect::<Vec<_>>();
            assert_eq!(values.len(), 12);
            values
        })
        .collect()
}

#[test]
fn phase_probe_observes_every_real_child_execution_in_order() {
    let root = temp_dir(&format!("ckc-diagnostic-phases-{}", unique_id()));
    let probe = phase_probe(&root);
    let artifact = compile_tune_c_fixture(
        &root,
        "artifact",
        include_str!("../fixtures/tune/cli-traced-artifact.c"),
        Command::new("cc"),
    );
    for iterations in [1, 2, 5, 16] {
        let trace = root.join(format!("trace-{iterations}"));
        let output = diagnostics::run_bounded(
            Command::new(&probe)
                .arg(&artifact)
                .arg(iterations.to_string())
                .env("CK_FIXTURE_TRACE", &trace),
            &root,
            &format!("phases-{iterations}"),
            Instant::now() + PHASE_PROBE_TIMEOUT,
        )
        .unwrap();
        assert!(
            output.status.success(),
            "missing truthful phase observations: {output:?}"
        );
        assert!(!output.timed_out);
        assert!(output.stderr.is_empty());
        let records = phase_records(&output.stdout);
        assert_eq!(records.len(), iterations);
        assert_eq!(fs::read(trace).unwrap(), vec![b'x'; iterations]);
        for (ordinal, row) in records.iter().enumerate() {
            assert_eq!(row[0], ordinal as u64);
            assert!(row[1] > 0 && row[2] > 0);
            // Parent-side observations, not claimed pure loader/CPU phases.
            assert!(0 < row[3] && row[3] <= row[4] && row[4] <= row[5] && row[5] <= row[6]);
            assert_eq!(row[9], 3);
            assert_eq!(row[10], 1);
            assert_eq!(row[11], 0);
        }
    }
}

#[test]
fn phase_probe_rejects_incorrect_artifact_output_or_exit() {
    let root = temp_dir(&format!("ckc-diagnostic-invalid-{}", unique_id()));
    let probe = phase_probe(&root);
    let artifact = compile_tune_c_fixture(
        &root,
        "artifact",
        include_str!("../fixtures/tune/cli-traced-artifact.c"),
        Command::new("cc"),
    );
    for mode in [
        "missing",
        "exit",
        "signal",
        "wrong-output",
        "empty-output",
        "extra-output",
    ] {
        let input = if mode == "missing" {
            root.join("missing")
        } else {
            artifact.clone()
        };
        let output = diagnostics::run_bounded(
            Command::new(&probe)
                .arg(input)
                .arg("1")
                .env("CK_FIXTURE_MODE", mode),
            &root,
            mode,
            Instant::now() + PHASE_PROBE_TIMEOUT,
        )
        .unwrap();
        assert!(!output.status.success());
        let records = phase_records(&output.stdout);
        assert_eq!(
            records.len(),
            1,
            "failed child observations must be retained for {mode}"
        );
        assert_eq!(records[0][10], 0);
        assert!(!String::from_utf8_lossy(&output.stdout).contains("CKTUNE/1 "));
    }
}

#[test]
fn phase_probe_rejects_invalid_or_unbounded_counts_without_execution() {
    let root = temp_dir(&format!("ckc-diagnostic-counts-{}", unique_id()));
    let probe = phase_probe(&root);
    let artifact = compile_tune_c_fixture(
        &root,
        "artifact",
        include_str!("../fixtures/tune/cli-traced-artifact.c"),
        Command::new("cc"),
    );
    for count in [
        "",
        "0",
        "01",
        "-1",
        "+1",
        " 1",
        "1x",
        "17",
        "18446744073709551616",
    ] {
        let label = format!("invalid-{}", unique_id());
        let trace = root.join(format!("{label}.trace"));
        let output = diagnostics::run_bounded(
            Command::new(&probe)
                .arg(&artifact)
                .arg(count)
                .env("CK_FIXTURE_TRACE", &trace),
            &root,
            &label,
            Instant::now() + PHASE_PROBE_TIMEOUT,
        )
        .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(!trace.exists());
    }
}

#[test]
fn bounded_command_cleans_descendants_before_reaping_a_completed_leader() {
    let root = temp_dir(&format!("ckc-diagnostic-completed-{}", unique_id()));
    let program = compile_tune_c_fixture(
        &root,
        "completed",
        r#"
#define _POSIX_C_SOURCE 200809L
#include <stdio.h>
#include <time.h>
#include <unistd.h>
int main(int argc, char **argv) {
    if (argc != 2) return 2;
    const pid_t child = fork();
    if (child < 0) return 3;
    if (child == 0) {
        struct timespec delay = {0, 200000000};
        while (nanosleep(&delay, &delay) != 0) {}
        FILE *trace = fopen(argv[1], "ab");
        if (!trace) return 4;
        fputc('x', trace);
        fclose(trace);
    }
    return 0;
}
"#,
        Command::new("cc"),
    );
    let trace = root.join("must-not-survive.trace");
    let output = diagnostics::run_bounded(
        Command::new(program).arg(&trace),
        &root,
        "completed",
        Instant::now() + Duration::from_secs(5),
    )
    .unwrap();
    assert!(!output.timed_out);
    assert!(output.status.success());
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        !trace.exists(),
        "a diagnostic descendant survived its completed leader"
    );
}

#[test]
fn bounded_command_retains_exit_status_and_both_streams() {
    let root = temp_dir(&format!("ckc-diagnostic-streams-{}", unique_id()));
    let program = compile_tune_c_fixture(
        &root,
        "streams",
        "#include <stdio.h>\nint main(void) { fputs(\"diagnostic stdout\\n\", stdout); fputs(\"diagnostic stderr\\n\", stderr); return 23; }\n",
        Command::new("cc"),
    );
    let output = diagnostics::run_bounded(
        &mut Command::new(program),
        &root,
        "streams",
        Instant::now() + Duration::from_secs(5),
    )
    .expect("bounded diagnostic should observe the real process");
    assert!(!output.timed_out);
    assert_eq!(output.status.code(), Some(23));
    assert_eq!(output.stdout, b"diagnostic stdout\n");
    assert_eq!(output.stderr, b"diagnostic stderr\n");
}

#[test]
fn expired_diagnostic_deadline_does_not_execute_the_command() {
    let root = temp_dir(&format!("ckc-diagnostic-expired-{}", unique_id()));
    let program = compile_tune_c_fixture(
        &root,
        "artifact",
        include_str!("../fixtures/tune/cli-traced-artifact.c"),
        Command::new("cc"),
    );
    let trace = root.join("must-not-exist");
    let error = diagnostics::run_bounded(
        Command::new(program).env("CK_FIXTURE_TRACE", &trace),
        &root,
        "expired",
        Instant::now(),
    )
    .expect_err("expired diagnostics must not execute");
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    assert!(!trace.exists());
    assert!(!root.join("expired.stdout.log").exists());
}

#[test]
fn successful_original_result_never_runs_failure_diagnostics() {
    diagnostics::after_failure(true, || panic!("success ran failure-only diagnostics"));
}

#[test]
fn failed_original_result_runs_diagnostics_even_when_the_probe_errors() {
    let mut called = false;
    diagnostics::after_failure(false, || {
        called = true;
        Err("the original failure must remain authoritative".to_string())
    });
    assert!(
        called,
        "the failed result did not collect diagnostic evidence"
    );
}

#[test]
fn failed_original_result_survives_a_panicking_diagnostic() {
    let mut called = false;
    diagnostics::after_failure(false, || {
        called = true;
        panic!("diagnostic panic must not replace the original failure");
    });
    assert!(called);
}

#[test]
fn bounded_command_timeout_stops_its_owned_descendant_and_retains_partial_output() {
    let root = temp_dir(&format!("ckc-diagnostic-timeout-{}", unique_id()));
    let program = compile_tune_c_fixture(
        &root,
        "timeout",
        r#"#define _POSIX_C_SOURCE 200809L
#include <signal.h>
#include <stdio.h>
#include <time.h>
#include <unistd.h>
int main(int argc, char **argv) {
    if (argc != 2) return 2;
    signal(SIGTERM, SIG_IGN);
    const pid_t child = fork();
    if (child < 0) return 3;
    if (child == 0) {
        const struct timespec interval = {0, 1000000};
        for (;;) {
            FILE *trace = fopen(argv[1], "ab");
            if (!trace) return 4;
            fputc('x', trace);
            fclose(trace);
            nanosleep(&interval, NULL);
        }
    }
    fputs("owned descendant started\n", stdout);
    fflush(stdout);
    for (;;) pause();
}
"#,
        Command::new("cc"),
    );
    let trace = root.join("descendant.trace");
    let output = diagnostics::run_bounded(
        Command::new(program).arg(&trace),
        &root,
        "timeout",
        // Leave room for a fresh executable's host security/loader checks;
        // this test must exercise a running descendant, not pre-main startup.
        Instant::now() + Duration::from_secs(5),
    )
    .expect("timeout must retain a bounded diagnostic result");
    assert!(output.timed_out);
    assert!(!output.status.success());
    assert_eq!(output.stdout, b"owned descendant started\n", "{output:?}");
    let stopped = fs::read(&trace).expect("the real descendant executed");
    assert!(!stopped.is_empty());
    std::thread::sleep(Duration::from_millis(30));
    assert_eq!(
        fs::read(&trace).unwrap(),
        stopped,
        "owned descendant survived timeout cleanup"
    );
    assert_eq!(
        fs::read(root.join("timeout.stdout.log")).unwrap(),
        output.stdout
    );
}

#[cfg(target_os = "macos")]
#[test]
fn failure_cohort_retains_setup_evidence_without_modifying_original_inputs() {
    let root = temp_dir(&format!("ckc-diagnostic-setup-{}", unique_id()));
    let runner = super::compile_tune_fixture_runner(&root, Command::new("cc"));
    let source = root.join("main.ck");
    let bytes = b"fn main() -> i32 { print_u32(66); print_newline(); return 0; }";
    fs::write(&source, bytes).unwrap();
    let runner_before = fs::read(&runner).unwrap();
    let error =
        diagnostics::collect_startup(&root, &source, &root.join("missing-compiler"), &runner)
            .expect_err("missing compiler should stop this diagnostic, not alter original inputs");
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    let evidence = root.join("post-failure-startup");
    assert!(evidence.join("diagnostic-supervisor.c").is_file());
    assert!(evidence.join("compile-supervisor.stdout.log").is_file());
    assert_eq!(
        fs::read(evidence.join("original-runner")).unwrap(),
        runner_before
    );
    assert_eq!(fs::read(&source).unwrap(), bytes);
    assert_eq!(fs::read(&runner).unwrap(), runner_before);
    assert!(!root.join("program").exists());
    assert!(!root.join("program.cktune").exists());
}

#[cfg(target_os = "macos")]
#[test]
fn diagnostic_staging_keeps_fresh_owner_only_bytes_and_never_overwrites() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp_dir(&format!("ckc-diagnostic-staging-{}", unique_id()));
    fs::create_dir(&root).unwrap();
    let path = root.join("image");
    diagnostics::write_executable(&path, b"exact image").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"exact image");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        diagnostics::write_executable(&path, b"replacement")
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::AlreadyExists
    );
    assert_eq!(fs::read(&path).unwrap(), b"exact image");
}

#[cfg(target_os = "macos")]
#[test]
fn diagnostic_staging_matches_the_original_durable_write_before_execute_order() {
    let helper = include_str!("../support/tune_diagnostics.rs");
    let staging = helper
        .split("fn write_executable(")
        .nth(1)
        .unwrap()
        .split("#[cfg(")
        .next()
        .unwrap();
    let write = staging.find("write_all(bytes)").unwrap();
    let sync = staging
        .find("sync_all()")
        .expect("diagnostic images must be synced just like the original staging path");
    let executable = staging
        .find("set_permissions(")
        .expect("enable execution after durable bytes, as in production staging");
    assert!(write < sync && sync < executable);
}
