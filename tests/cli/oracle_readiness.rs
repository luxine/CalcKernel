use std::{fs, path::Path, process::Command};

use super::support::{command::node_available, oracle::typescript_root, temp::temp_dir};

#[test]
fn typescript_oracle_verifier_should_confirm_default_cli_oracle_is_available() {
    if !node_available() {
        return;
    }

    let summary = verify_typescript_oracle(&typescript_root()).expect("verify TypeScript oracle");

    assert_eq!(summary.cli_status, "ok");
    assert!(
        !summary.cli_help_first_line.is_empty(),
        "TypeScript oracle verifier should report CLI health: {summary:?}"
    );
    assert!(
        summary.fixture_count > 0,
        "TypeScript oracle should expose fixtures: {summary:?}"
    );
}

#[test]
fn typescript_oracle_verifier_should_reject_missing_dist_cli() {
    if !node_available() {
        return;
    }

    let temp = temp_dir("rust-calckernel-missing-ts-cli");
    fs::create_dir_all(&temp).expect("create temp TypeScript oracle root");
    fs::write(
        temp.join("package.json"),
        r#"{
  "name": "calckernel",
  "main": "./dist/src/index.js",
  "bin": {
    "ckc": "./dist/src/cli.js"
  },
  "dependencies": {
    "wabt": "^1.0.39"
  }
}"#,
    )
    .expect("write package json");

    let error = verify_typescript_oracle(&temp).expect_err("missing dist CLI should fail");

    let _ = fs::remove_dir_all(&temp);

    assert!(
        error.contains("dist/src/cli.js"),
        "missing dist CLI failure should identify the oracle CLI: {error}"
    );
}

#[derive(Debug)]
struct OracleSummary {
    cli_status: &'static str,
    cli_help_first_line: String,
    fixture_count: usize,
}

fn verify_typescript_oracle(ts_root: &Path) -> Result<OracleSummary, String> {
    expect_exists(ts_root, "TypeScript oracle root")?;

    let package_json_path = ts_root.join("package.json");
    let package_json = fs::read_to_string(&package_json_path)
        .map_err(|error| format!("read {}: {error}", package_json_path.display()))?;
    for expected in [
        r#""name": "calckernel""#,
        r#""main": "./dist/src/index.js""#,
        r#""ckc": "./dist/src/cli.js""#,
        r#""wabt": "^1.0.39""#,
    ] {
        if !package_json.contains(expected) {
            return Err(format!(
                "TypeScript oracle package.json must contain {expected}"
            ));
        }
    }

    let cli_path = ts_root.join("dist/src/cli.js");
    expect_exists(&cli_path, "TypeScript oracle dist/src/cli.js")?;

    let fixture_roots = ["examples", "bench/perf/fixtures", "tests/fixtures"];
    for fixture_root in fixture_roots {
        expect_exists(
            &ts_root.join(fixture_root),
            &format!("TypeScript oracle fixture root {fixture_root}"),
        )?;
    }

    let output = Command::new("node")
        .arg(&cli_path)
        .arg("--help")
        .current_dir(ts_root)
        .output()
        .map_err(|error| format!("TypeScript oracle CLI failed to start: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "TypeScript oracle CLI --help failed with status {:?}\nstdout:\n{}\nstderr:\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let help = String::from_utf8_lossy(&output.stdout);
    for expected in ["ckc check <file>", "ckc build-llvm <file>"] {
        if !help.contains(expected) {
            return Err(format!(
                "TypeScript oracle CLI --help must include {expected}"
            ));
        }
    }

    let fixture_count = fixture_roots
        .iter()
        .map(|fixture_root| count_ck_files(&ts_root.join(fixture_root)))
        .sum::<Result<usize, String>>()?;

    Ok(OracleSummary {
        cli_status: "ok",
        cli_help_first_line: help.lines().next().unwrap_or_default().to_string(),
        fixture_count,
    })
}

fn expect_exists(path: &Path, label: &str) -> Result<(), String> {
    if path.exists() {
        Ok(())
    } else {
        Err(format!("{label} is missing: {}", path.display()))
    }
}

fn count_ck_files(dir: &Path) -> Result<usize, String> {
    let mut count = 0;
    for entry in fs::read_dir(dir).map_err(|error| format!("read {}: {error}", dir.display()))? {
        let path = entry
            .map_err(|error| format!("read entry in {}: {error}", dir.display()))?
            .path();
        if path.is_dir() {
            count += count_ck_files(&path)?;
        } else if path.extension().and_then(|value| value.to_str()) == Some("ck") {
            count += 1;
        }
    }
    Ok(count)
}
