use std::{fs, process::Command};

use calckernel::{
    RunnerFailure, TuneArtifactKind, TuneInvocation, TuneManifest, TuneRunner,
    TuneTrialBuildRequest, capture_workload, compile_tune_trial, enumerate_tuning_space,
};

use super::trial::state;

const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[test]
fn runner_protocol_process_uses_empty_environment_fresh_cwd_and_exact_echo() {
    let temp = tempfile_dir("runner");
    let source = temp.join("probe.c");
    let runner = temp.join("probe");
    fs::write(
        &source,
        format!(
            "#include <stdio.h>\n#include <stdlib.h>\n#include <string.h>\n#include <unistd.h>\nint main(void) {{ char cwd[4096]; if (getenv(\"PATH\")) return 10; if (!getcwd(cwd, sizeof(cwd))) return 11; if (strcmp(cwd, getenv(\"CK_TUNE_TEMP\"))) return 12; const char *c=getenv(\"CK_TUNE_CASE\"), *s=getenv(\"CK_TUNE_SEED\"), *i=getenv(\"CK_TUNE_ITERATIONS\"); if (!strcmp(c, \"validation\")) usleep(1500000); printf(\"CKTUNE/1 %s %s %s %s {DIGEST}\\n\", c,s,i,i); return 0; }}\n"
        ),
    )
    .unwrap();
    assert!(
        Command::new("cc")
            .args([source.as_os_str(), "-o".as_ref(), runner.as_os_str()])
            .status()
            .unwrap()
            .success()
    );
    let manifest_path = temp.join("workload.cktune.toml");
    let manifest = TuneManifest::parse(
        format!(
            "schema=1\n[runner]\npath=\"{}\"\ninput_root=\".\"\ntimeout_ms=1000\n[[case]]\nid=\"search\"\nrole=\"search\"\nseed=7\nweight=1\nexpected_digest=\"{DIGEST}\"\n[[case]]\nid=\"validation\"\nrole=\"validation\"\nseed=8\nweight=1\nexpected_digest=\"{DIGEST}\"\n",
            runner.display()
        ).as_bytes(),
        &manifest_path,
    ).unwrap();
    let workload = capture_workload(&manifest).unwrap();
    let state = state();
    let space = enumerate_tuning_space(&state).unwrap();
    let trial = compile_tune_trial(
        &state,
        &space,
        &calckernel::TuningPlan::baseline(),
        TuneTrialBuildRequest::new(
            TuneArtifactKind::Executable,
            vec![1],
            None,
            None,
            vec![("program.o".into(), vec![2])],
            vec!["test".into()],
        ),
    )
    .unwrap();
    assert!(matches!(
        TuneRunner::new().invoke(
            &workload,
            &trial,
            &TuneInvocation::new(&manifest.cases()[0], 3, 3_249)
        ),
        Err(RunnerFailure::WallBudgetAdmission)
    ));
    let result = TuneRunner::new()
        .invoke(
            &workload,
            &trial,
            &TuneInvocation::new(&manifest.cases()[0], 3, 10_000),
        )
        .unwrap();
    assert_eq!(result.completed, 3);
    assert_eq!(result.digest, [0xaa; 32]);
    assert!(matches!(
        TuneRunner::new().invoke(
            &workload,
            &trial,
            &TuneInvocation::new(&manifest.cases()[1], 3, 10_000).candidate(true)
        ),
        Err(RunnerFailure::CandidateTimeout(_))
    ));
    fs::remove_dir_all(temp).unwrap();
}

fn tempfile_dir(label: &str) -> std::path::PathBuf {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/tune-tests");
    fs::create_dir_all(&root).unwrap();
    let path = root.join(format!(
        "ckc-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&path).unwrap();
    path
}
