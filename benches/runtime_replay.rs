use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

pub const BASELINE_COMMIT: &str = "df816502876fba41676f9ebc190e4fadd18cd5a5";
pub const BASELINE_COMPILER: &str = "calckernel 0.10.0 (df816502876fba41676f9ebc190e4fadd18cd5a5)";
pub const BASELINE_MANIFEST_SHA256: &str =
    "27c0b995ba51cd799c2bcb89e1df0a4d40538fbf3200e1197f06ecab2ebad4f3";
pub const RUNTIME_CASES: [&str; 4] = [
    "branch_mix",
    "integer_accumulate",
    "proof_loop",
    "remainder_chain",
];

#[derive(Debug)]
pub struct ReplayArtifact {
    pub case: String,
    pub mode: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug)]
pub struct RuntimeReplay {
    pub metadata: BTreeMap<String, String>,
    pub artifacts: Vec<ReplayArtifact>,
    pub manifest_sha256: String,
}

pub struct ExpectedReplay<'a> {
    pub target: &'a str,
    pub cpu: &'a str,
    pub recipe_sha256: &'a str,
    pub adapter_set_sha256: &'a str,
    pub llvm_component_sha256: &'a str,
}

fn named_digest(mut entries: Vec<(&str, String)>) -> String {
    entries.sort();
    let mut digest = Sha256::new();
    for (name, value) in entries {
        digest.update(name.as_bytes());
        digest.update(b"\0");
        digest.update(value.as_bytes());
        digest.update(b"\n");
    }
    format!("{:x}", digest.finalize())
}

pub fn recipe_digest(repo: &Path) -> Result<String, String> {
    let entries = [
        "scripts/prepare-performance-replay.py",
        "benches/runtime_replay.rs",
        "benches/ckc_perf.rs",
    ]
    .into_iter()
    .map(|name| Ok((name, sha256_file(&repo.join(name))?)))
    .collect::<Result<Vec<_>, String>>()?;
    Ok(named_digest(entries))
}

pub fn adapter_set_digest(repo: &Path) -> Result<String, String> {
    const ADAPTERS: [(&str, &str); 4] = [
        (
            "benches/baselines/v0_10_linux_cpp_runtime_harness.patch",
            "099305e8a9d5ff8d54e574b0fbd202a511f28a8543508f8c0ea06001704cdaff",
        ),
        (
            "benches/baselines/v0_10_clang_cpu_harness.patch",
            "f22d58f4e2712e792a5b933376fe3a81fa1bd44a4cdb39b2790359ab5a40c7f1",
        ),
        (
            "benches/baselines/v0_10_mir_optimizer_harness.patch",
            "828138f376472b177d8bbd1aa4f7888ed323ec03d098e21a74abcfce32a98d0b",
        ),
        (
            "benches/baselines/v0_10_proof_loop_harness.patch",
            "316b64bf3e24ade271d870444bb66a85018c4dcb66229afce202da2d2b53af6e",
        ),
    ];
    if sha256_file(&repo.join("benches/baselines/v0_10_compiler.toml"))? != BASELINE_MANIFEST_SHA256
    {
        return Err("the frozen V0.10 baseline manifest has changed".into());
    }
    let mut entries = Vec::new();
    for (name, expected) in ADAPTERS {
        let actual = sha256_file(&repo.join(name))?;
        if actual != expected {
            return Err(format!("pinned replay adapter has changed: {name}"));
        }
        entries.push((name, actual));
    }
    Ok(named_digest(entries))
}

pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file =
        fs::File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65_536];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("hash {}: {error}", path.display()))?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn check_hash(value: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("replay digest must be exactly 64 lowercase hexadecimal characters".into());
    }
    Ok(())
}

fn positive_size(value: &str) -> Result<u64, String> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("replay byte count must be a positive integer".into());
    }
    value
        .parse::<u64>()
        .ok()
        .filter(|size| *size > 0)
        .ok_or_else(|| "replay byte count must fit a positive u64".into())
}

fn regular_file(path: &Path) -> Result<fs::Metadata, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("replay file {}: {error}", path.display()))?;
    if !metadata.file_type().is_file() || metadata.len() == 0 {
        return Err(format!(
            "replay input must be a nonempty regular file, not a link: {}",
            path.display()
        ));
    }
    Ok(metadata)
}

fn verify_file(path: &Path, size: u64, digest: &str) -> Result<(), String> {
    check_hash(digest)?;
    if regular_file(path)?.len() != size || sha256_file(path)? != digest {
        return Err(format!(
            "replay file size or SHA-256 mismatch: {}",
            path.display()
        ));
    }
    Ok(())
}

pub fn load_replay(bundle: &Path, expected: &ExpectedReplay<'_>) -> Result<RuntimeReplay, String> {
    const FIELDS: [&str; 12] = [
        "commit",
        "compilerIdentity",
        "compilerSha256",
        "compilerBytes",
        "llvmVersion",
        "target",
        "cpuPolicy",
        "recipeSha256",
        "adapterSetSha256",
        "sourceDiffSha256",
        "baselineManifestSha256",
        "llvmComponentSha256",
    ];
    let root = bundle.canonicalize().map_err(|error| {
        format!(
            "missing replay bundle {}: {error}; run scripts/prepare-performance-replay.py first",
            bundle.display()
        )
    })?;
    let manifest_path = root.join("replay.tsv");
    regular_file(&manifest_path)?;
    let text = fs::read_to_string(&manifest_path)
        .map_err(|error| format!("read replay manifest: {error}"))?;
    let mut lines = text.lines();
    if lines.next() != Some("ckc-v010-runtime-replay\t1") {
        return Err("unsupported replay manifest schema".into());
    }
    let suffix = match expected.target {
        "linux-x86_64" | "linux-aarch64" => ".so",
        "macos-x86_64" | "macos-aarch64" => ".dylib",
        "windows-x86_64" | "windows-aarch64" => ".dll",
        _ => return Err("unsupported replay target".into()),
    };
    let mut metadata = BTreeMap::<String, String>::new();
    let mut artifacts = BTreeMap::new();
    for line in lines {
        let fields = line.split('\t').collect::<Vec<_>>();
        match fields.as_slice() {
            ["artifact", mode, case, filename, size, digest] => {
                if !matches!(*mode, "checked" | "unchecked") || !RUNTIME_CASES.contains(case) {
                    return Err("replay artifact has an unknown case or safety mode".into());
                }
                if *filename != format!("{case}-{mode}{suffix}") {
                    return Err("replay artifact must use its exact non-escaping basename".into());
                }
                check_hash(digest)?;
                let artifact = ReplayArtifact {
                    case: (*case).into(),
                    mode: (*mode).into(),
                    path: root.join(filename),
                    bytes: positive_size(size)?,
                    sha256: (*digest).into(),
                };
                if artifacts
                    .insert(((*mode).to_string(), (*case).to_string()), artifact)
                    .is_some()
                {
                    return Err("duplicate replay case/mode".into());
                }
            }
            [name, value] if FIELDS.contains(name) && !value.is_empty() => {
                if metadata.insert((*name).into(), (*value).into()).is_some() {
                    return Err(format!("duplicate replay identity field {name}"));
                }
            }
            _ => return Err("unknown or malformed replay manifest record".into()),
        }
    }
    if metadata.len() != FIELDS.len() || artifacts.len() != RUNTIME_CASES.len() * 2 {
        return Err("replay manifest must contain every identity and exact case/mode".into());
    }
    for (field, value) in [
        ("commit", BASELINE_COMMIT),
        ("compilerIdentity", BASELINE_COMPILER),
        ("llvmVersion", "22.1.8"),
        ("target", expected.target),
        ("cpuPolicy", expected.cpu),
        ("recipeSha256", expected.recipe_sha256),
        ("adapterSetSha256", expected.adapter_set_sha256),
        ("baselineManifestSha256", BASELINE_MANIFEST_SHA256),
        ("llvmComponentSha256", expected.llvm_component_sha256),
    ] {
        if metadata[field] != value {
            return Err(format!("replay {field} does not match pinned identity"));
        }
    }
    for field in [
        "compilerSha256",
        "recipeSha256",
        "adapterSetSha256",
        "sourceDiffSha256",
        "baselineManifestSha256",
        "llvmComponentSha256",
    ] {
        check_hash(&metadata[field])?;
    }
    verify_file(
        &root.join("ckc-v010"),
        positive_size(&metadata["compilerBytes"])?,
        &metadata["compilerSha256"],
    )?;
    for artifact in artifacts.values() {
        verify_file(&artifact.path, artifact.bytes, &artifact.sha256)?;
    }
    Ok(RuntimeReplay {
        metadata,
        artifacts: artifacts.into_values().collect(),
        manifest_sha256: format!("{:x}", Sha256::digest(text.as_bytes())),
    })
}
