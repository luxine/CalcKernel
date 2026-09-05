use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

pub const V010_BASELINE_COMMIT: &str = "df816502876fba41676f9ebc190e4fadd18cd5a5";
pub const V010_BASELINE_COMPILER: &str =
    "calckernel 0.10.0 (df816502876fba41676f9ebc190e4fadd18cd5a5)";
pub const V010_BASELINE_MANIFEST_SHA256: &str =
    "27c0b995ba51cd799c2bcb89e1df0a4d40538fbf3200e1197f06ecab2ebad4f3";
pub const V011_BASELINE_COMMIT: &str = "80c0acf6bb5d65e4d9d40352b9501ea32b79f43d";
pub const V011_BASELINE_COMPILER: &str =
    "calckernel 0.11.0 (80c0acf6bb5d65e4d9d40352b9501ea32b79f43d)";
pub const V011_BASELINE_MANIFEST_SHA256: &str =
    "495cde2e3a2afb847ddcad9707fec4e6880f26dc6c3085442290af7e2737421e";
pub const RUNTIME_CASES: [&str; 4] = [
    "branch_mix",
    "integer_accumulate",
    "proof_loop",
    "remainder_chain",
];

#[allow(dead_code)]
pub fn sampling_round(round: usize) -> [usize; 12] {
    rotating_round(round)
}

#[derive(Debug)]
pub struct RuntimeSamples<const CHANNELS: usize> {
    pub warmup_order: Vec<[usize; CHANNELS]>,
    pub sample_order: Vec<[usize; CHANNELS]>,
    pub channels: [Vec<u128>; CHANNELS],
}

pub fn sample_channels<E>(
    warmup: usize,
    iterations: usize,
    call: impl FnMut(usize, bool) -> Result<u128, E>,
) -> Result<RuntimeSamples<12>, E> {
    sample_rotating_channels(warmup, iterations, call)
}

pub fn sample_three_channels<E>(
    warmup: usize,
    iterations: usize,
    call: impl FnMut(usize, bool) -> Result<u128, E>,
) -> Result<RuntimeSamples<3>, E> {
    sample_rotating_channels(warmup, iterations, call)
}

pub fn sample_three_channels_upper_median<E, const REPETITIONS: usize>(
    warmup: usize,
    iterations: usize,
    mut call: impl FnMut(usize, bool) -> Result<u128, E>,
) -> Result<RuntimeSamples<3>, E> {
    assert!(
        REPETITIONS > 0,
        "an upper median requires at least one sample"
    );
    let mut result = RuntimeSamples {
        warmup_order: Vec::with_capacity(warmup),
        sample_order: Vec::with_capacity(iterations),
        channels: std::array::from_fn(|_| Vec::with_capacity(iterations)),
    };
    for round in 0..warmup {
        let order = rotating_round(round);
        for channel in order {
            call(channel, true)?;
        }
        result.warmup_order.push(order);
    }
    for round in 0..iterations {
        let mut raw: [Vec<u128>; 3] = std::array::from_fn(|_| Vec::with_capacity(REPETITIONS));
        for repetition in 0..REPETITIONS {
            for channel in rotating_round::<3>(round.wrapping_add(repetition)) {
                raw[channel].push(call(channel, false)?);
            }
        }
        for (channel, samples) in raw.iter_mut().enumerate() {
            samples.sort_unstable();
            result.channels[channel].push(samples[REPETITIONS / 2]);
        }
        result.sample_order.push(rotating_round(round));
    }
    Ok(result)
}

pub fn sample_upper_median<E, const SAMPLES: usize>(
    mut call: impl FnMut() -> Result<u128, E>,
) -> Result<u128, E> {
    assert!(SAMPLES > 0, "an upper median requires at least one sample");
    let mut samples = [0_u128; SAMPLES];
    for sample in &mut samples {
        *sample = call()?;
    }
    samples.sort_unstable();
    Ok(samples[SAMPLES / 2])
}

fn rotating_round<const CHANNELS: usize>(round: usize) -> [usize; CHANNELS] {
    std::array::from_fn(|offset| (round % CHANNELS + offset) % CHANNELS)
}

fn sample_rotating_channels<E, const CHANNELS: usize>(
    warmup: usize,
    iterations: usize,
    mut call: impl FnMut(usize, bool) -> Result<u128, E>,
) -> Result<RuntimeSamples<CHANNELS>, E> {
    let mut result = RuntimeSamples {
        warmup_order: Vec::new(),
        sample_order: Vec::new(),
        channels: std::array::from_fn(|_| Vec::with_capacity(iterations)),
    };
    for round in 0..warmup {
        let order = rotating_round(round);
        for channel in order {
            call(channel, true)?;
        }
        result.warmup_order.push(order);
    }
    for round in 0..iterations {
        let order = rotating_round(round);
        for channel in order {
            result.channels[channel].push(call(channel, false)?);
        }
        result.sample_order.push(order);
    }
    Ok(result)
}

#[derive(Debug)]
pub struct ReplayArtifact {
    pub case: String,
    pub mode: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}

#[allow(dead_code)]
#[derive(Debug)]
pub struct RuntimeReplay {
    pub generation: ReplayGeneration,
    pub metadata: BTreeMap<String, String>,
    pub artifacts: Vec<ReplayArtifact>,
    pub manifest_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayGeneration {
    V010,
    V011,
}

impl ReplayGeneration {
    const fn header(self) -> &'static str {
        match self {
            Self::V010 => "ckc-v010-runtime-replay\t1",
            Self::V011 => "ckc-v011-runtime-replay\t1",
        }
    }

    pub const fn compiler_file(self) -> &'static str {
        match self {
            Self::V010 => "ckc-v010",
            Self::V011 => "ckc-v011",
        }
    }

    const fn commit(self) -> &'static str {
        match self {
            Self::V010 => V010_BASELINE_COMMIT,
            Self::V011 => V011_BASELINE_COMMIT,
        }
    }

    const fn compiler_identity(self) -> &'static str {
        match self {
            Self::V010 => V010_BASELINE_COMPILER,
            Self::V011 => V011_BASELINE_COMPILER,
        }
    }

    const fn manifest_sha256(self) -> &'static str {
        match self {
            Self::V010 => V010_BASELINE_MANIFEST_SHA256,
            Self::V011 => V011_BASELINE_MANIFEST_SHA256,
        }
    }
}

pub struct ExpectedReplay<'a> {
    pub generation: ReplayGeneration,
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
        "scripts/audit-performance-oracles.py",
        "benches/runtime_replay.rs",
        "benches/ckc_perf.rs",
        "benches/vector_perf.rs",
        "benches/pgo_perf.rs",
        "benches/cases/pgo-cases.tsv",
        "scripts/measure-v013-performance.py",
        "benches/oracles/manifest.toml",
        "benches/oracles/pgo/manifest.toml",
    ]
    .into_iter()
    .map(|name| Ok((name, sha256_file(&repo.join(name))?)))
    .collect::<Result<Vec<_>, String>>()?;
    Ok(named_digest(entries))
}

pub fn v010_adapter_set_digest(repo: &Path) -> Result<String, String> {
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
    if sha256_file(&repo.join("benches/baselines/v0_10_compiler.toml"))?
        != V010_BASELINE_MANIFEST_SHA256
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

pub fn v011_adapter_set_digest(repo: &Path) -> Result<String, String> {
    if sha256_file(&repo.join("benches/baselines/v0_11_compiler.toml"))?
        != V011_BASELINE_MANIFEST_SHA256
    {
        return Err("the frozen V0.11 baseline manifest has changed".into());
    }
    Ok(named_digest(Vec::new()))
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
    if lines.next() != Some(expected.generation.header()) {
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
        ("commit", expected.generation.commit()),
        ("compilerIdentity", expected.generation.compiler_identity()),
        ("llvmVersion", "22.1.8"),
        ("target", expected.target),
        ("cpuPolicy", expected.cpu),
        ("recipeSha256", expected.recipe_sha256),
        ("adapterSetSha256", expected.adapter_set_sha256),
        (
            "baselineManifestSha256",
            expected.generation.manifest_sha256(),
        ),
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
        &root.join(expected.generation.compiler_file()),
        positive_size(&metadata["compilerBytes"])?,
        &metadata["compilerSha256"],
    )?;
    for artifact in artifacts.values() {
        verify_file(&artifact.path, artifact.bytes, &artifact.sha256)?;
    }
    Ok(RuntimeReplay {
        generation: expected.generation,
        metadata,
        artifacts: artifacts.into_values().collect(),
        manifest_sha256: format!("{:x}", Sha256::digest(text.as_bytes())),
    })
}
