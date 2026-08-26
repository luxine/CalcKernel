use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use super::support::fixtures;
use super::support::oracle::{configured_typescript_root, repo_root};

#[test]
fn typescript_oracle_fixtures_should_be_covered_by_rust_backend_tests() {
    let Some(ts_root) = configured_typescript_root() else {
        return;
    };
    let report = audit_typescript_oracle_fixture_coverage(&ts_root)
        .expect("run TypeScript oracle fixture coverage audit");

    assert!(
        report.failures.is_empty(),
        "TypeScript oracle fixture coverage audit failed:\n{}",
        report.failures.join("\n")
    );

    assert!(
        report
            .generated_output_fixtures
            .iter()
            .any(|fixture| fixture == "tests/fixtures/f64_edges.ck"),
        "f64 edge fixture should be part of cross-backend generated output coverage"
    );
}

struct FixtureCoverageReport {
    generated_output_fixtures: Vec<String>,
    failures: Vec<String>,
}

fn audit_typescript_oracle_fixture_coverage(
    ts_root: &Path,
) -> Result<FixtureCoverageReport, String> {
    let fixture_roots = ["examples", "bench/perf/fixtures", "tests/fixtures"];
    let backend_coverage = [
        ("MIR", "tests/ir/mir.rs"),
        ("C", "tests/backend/c.rs"),
        ("WASM", "tests/backend/wasm.rs"),
        ("LLVM", "tests/backend/llvm.rs"),
    ];
    let mut failures = Vec::new();

    if !ts_root.exists() {
        failures.push(format!(
            "TypeScript oracle root is missing: {}",
            ts_root.display()
        ));
    }

    let mut fixtures = Vec::new();
    for fixture_root in fixture_roots {
        let root = ts_root.join(fixture_root);
        if root.exists() {
            fixtures.extend(list_ck_files(ts_root, &root)?);
        } else {
            failures.push(format!(
                "TypeScript fixture directory is missing: {}",
                root.display()
            ));
        }
    }
    fixtures.sort();

    let mapped = fixtures::ORACLE_EXAMPLES
        .iter()
        .chain(fixtures::BENCHMARK_FIXTURES)
        .copied()
        .collect::<Vec<_>>();
    let expected = mapped
        .iter()
        .map(|fixture| fixture.oracle.to_owned())
        .chain(std::iter::once("tests/fixtures/f64_edges.ck".to_owned()))
        .collect::<BTreeSet<_>>();
    let discovered = fixtures.iter().cloned().collect::<BTreeSet<_>>();
    for missing in expected.difference(&discovered) {
        failures.push(format!("mapped oracle fixture is missing: {missing}"));
    }
    for unmapped in discovered.difference(&expected) {
        failures.push(format!("oracle fixture has no registry entry: {unmapped}"));
    }

    for fixture in mapped {
        let local_path = repo_root().join(fixture.local);
        let oracle_path = ts_root.join(fixture.oracle);
        match (fs::read(&local_path), fs::read(&oracle_path)) {
            (Ok(local), Ok(oracle)) if local == oracle => {}
            (Ok(_), Ok(_)) => failures.push(format!(
                "fixture content diverges: {} != {}",
                fixture.local, fixture.oracle
            )),
            (Err(error), _) => failures.push(format!(
                "local fixture is missing: {}: {error}",
                fixture.local
            )),
            (_, Err(error)) => failures.push(format!(
                "oracle fixture is missing: {}: {error}",
                fixture.oracle
            )),
        }
    }

    let expected_local = fixtures::ORACLE_EXAMPLES
        .iter()
        .map(|fixture| fixture.local.to_owned())
        .chain(
            fixtures::LOCAL_ONLY_EXAMPLES
                .iter()
                .map(|path| (*path).to_owned()),
        )
        .collect::<BTreeSet<_>>();
    let discovered_local = list_ck_files(repo_root(), &repo_root().join("examples"))?
        .into_iter()
        .collect::<BTreeSet<_>>();
    for missing in expected_local.difference(&discovered_local) {
        failures.push(format!("registered local example is missing: {missing}"));
    }
    for unregistered in discovered_local.difference(&expected_local) {
        failures.push(format!(
            "local example has no registry entry: {unregistered}"
        ));
    }

    for (label, path) in backend_coverage {
        let absolute = repo_root().join(path);
        match fs::read_to_string(&absolute) {
            Ok(text) => {
                for required in [
                    "use super::support::fixtures;",
                    "fixtures::",
                    "tests/fixtures/f64_edges.ck",
                ] {
                    if !text.contains(required) {
                        failures.push(format!(
                            "{label} backend oracle coverage must use {required:?}"
                        ));
                    }
                }
            }
            Err(error) => failures.push(format!("Rust test file is missing: {path}: {error}")),
        }
    }

    Ok(FixtureCoverageReport {
        generated_output_fixtures: fixtures,
        failures,
    })
}

fn list_ck_files(base: &Path, dir: &Path) -> Result<Vec<String>, String> {
    Ok(list_files(dir)?
        .into_iter()
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("ck"))
        .map(|path| normalize_relative(base, &path))
        .collect())
}

fn list_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).map_err(|error| format!("read {}: {error}", dir.display()))? {
        let path = entry
            .map_err(|error| format!("read entry in {}: {error}", dir.display()))?
            .path();
        if path.is_dir() {
            files.extend(list_files(&path)?);
        } else if path.is_file() {
            files.push(path);
        }
    }
    Ok(files)
}

fn normalize_relative(base: &Path, path: &Path) -> String {
    path.strip_prefix(base)
        .expect("fixture under TypeScript root")
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/")
}
