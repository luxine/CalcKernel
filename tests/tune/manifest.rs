use std::path::Path;

use calckernel::{TuneManifest, TuneManifestError};

const VALID: &str = r#"
schema = 1

[runner]
path = "./runner"
args = ["--ck-tune"]
inputs = ["data/search.bin"]
inherit_env = []
timeout_ms = 30000

[[case]]
id = "search"
role = "search"
seed = 1
weight = 2
expected_digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"

[[case]]
id = "validation"
role = "validation"
seed = 2
weight = 3
expected_digest = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
"#;

#[test]
fn manifest_schema_one_accepts_only_the_closed_schema() {
    let manifest = TuneManifest::parse(VALID.as_bytes(), Path::new("/tmp/workload.cktune.toml"))
        .expect("valid manifest");
    assert_eq!(manifest.timeout_ms(), 30_000);
    assert_eq!(manifest.cases().len(), 2);

    let unknown = VALID.replace("schema = 1", "schema = 1\nunknown = true");
    assert_eq!(
        TuneManifest::parse(unknown.as_bytes(), Path::new("/tmp/workload.cktune.toml")),
        Err(TuneManifestError::UnknownField("unknown".to_owned()))
    );

    let missing_role = VALID.replace("role = \"validation\"\n", "");
    assert_eq!(
        TuneManifest::parse(
            missing_role.as_bytes(),
            Path::new("/tmp/workload.cktune.toml")
        ),
        Err(TuneManifestError::MissingField("case.role"))
    );
}

#[test]
fn manifest_schema_one_rejects_partition_and_path_ambiguity() {
    let duplicate_seed = VALID.replace("seed = 2", "seed = 1");
    assert_eq!(
        TuneManifest::parse(
            duplicate_seed.as_bytes(),
            Path::new("/tmp/workload.cktune.toml")
        ),
        Err(TuneManifestError::DuplicateCaseSeed(1))
    );

    let traversing = VALID.replace("data/search.bin", "../search.bin");
    assert_eq!(
        TuneManifest::parse(
            traversing.as_bytes(),
            Path::new("/tmp/workload.cktune.toml")
        ),
        Err(TuneManifestError::InvalidInputPath(
            "../search.bin".to_owned()
        ))
    );
}
