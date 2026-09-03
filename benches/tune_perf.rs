use std::{env, path::PathBuf, process::Command};

const USAGE: &str = "cargo bench --features native-toolchain --bench tune_perf -- --task <collect|collect-predicated-update> --out <report.json>";

fn main() {
    if let Err(error) = run() {
        eprintln!("tune_perf failed: {error}\n\n{USAGE}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    if !cfg!(feature = "native-toolchain") {
        return Err("tune_perf requires --features native-toolchain".to_string());
    }
    let mut arguments = env::args().skip(1).collect::<Vec<_>>();
    arguments.retain(|argument| argument != "--bench");
    if arguments.iter().any(|argument| argument == "--help") {
        println!("{USAGE}");
        return Ok(());
    }
    let [task_flag, task, out_flag, _output] = arguments.as_slice() else {
        return Err("the release harness requires exact --task <task> --out <path>".to_string());
    };
    if task_flag != "--task"
        || out_flag != "--out"
        || !matches!(task.as_str(), "collect" | "collect-predicated-update")
    {
        return Err("the release harness received a noncanonical task request".to_string());
    }
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let script = if task == "collect-predicated-update" {
        "scripts/measure-v014-predicated-update.py"
    } else {
        "scripts/measure-v014-performance.py"
    };
    let status = Command::new("python3")
        .arg("-B")
        .arg(repository.join(script))
        .args(arguments)
        .current_dir(&repository)
        .status()
        .map_err(|error| format!("start tuning performance collection: {error}"))?;
    if !status.success() {
        return Err(format!(
            "tuning performance collection exited with {status}"
        ));
    }
    Ok(())
}
