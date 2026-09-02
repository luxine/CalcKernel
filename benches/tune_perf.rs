use std::{env, path::PathBuf, process::Command};

const USAGE: &str = "cargo bench --features native-toolchain --bench tune_perf -- --task collect --out <target/ckc-perf/v0.14-results.json>";

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
    if arguments
        .windows(2)
        .all(|pair| pair != ["--task", "collect"])
        || !arguments.iter().any(|argument| argument == "--out")
    {
        return Err("the release harness requires --task collect --out <path>".to_string());
    }
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let status = Command::new("python3")
        .arg("-B")
        .arg(repository.join("scripts/measure-v014-performance.py"))
        .args(arguments)
        .current_dir(&repository)
        .status()
        .map_err(|error| format!("start schema-9 measurement: {error}"))?;
    if !status.success() {
        return Err(format!("schema-9 measurement exited with {status}"));
    }
    Ok(())
}
