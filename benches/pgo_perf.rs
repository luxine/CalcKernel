use std::{env, path::PathBuf, process::Command};

const USAGE: &str = "cargo bench --features native-toolchain --bench pgo_perf -- --task collect --out <target/ckc-perf/v0.13-results.json> [--quick]";

fn main() {
    if let Err(error) = run() {
        eprintln!("pgo_perf failed: {error}\n\n{USAGE}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    if !cfg!(feature = "native-toolchain") {
        return Err("pgo_perf requires --features native-toolchain".into());
    }
    let mut arguments = env::args().skip(1).collect::<Vec<_>>();
    arguments.retain(|argument| argument != "--bench");
    if arguments.iter().any(|argument| argument == "--help") {
        println!("{USAGE}");
        return Ok(());
    }
    if arguments
        .windows(2)
        .all(|pair| pair != ["--task", "collect"])
        || !arguments.iter().any(|argument| argument == "--out")
    {
        return Err("the release harness requires `--task collect --out <path>`".into());
    }
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let script = repository.join("scripts/measure-v013-performance.py");
    let mut command = Command::new("python3");
    command.arg("-B").arg(script);
    command.args(arguments.drain(..));
    let status = command
        .current_dir(&repository)
        .status()
        .map_err(|error| format!("start schema-8 measurement: {error}"))?;
    if !status.success() {
        return Err(format!("schema-8 measurement exited with {status}"));
    }
    Ok(())
}
