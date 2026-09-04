use super::*;

use calckernel::{
    KIR_VECTOR_COST_MODEL_SCHEMA, KIR_VECTOR_PROOF_SCHEMA, KirTargetIdentity, KirTargetProfile,
    build_kir_module_with_profile,
};

const ORACLE_MANIFEST: &str = "benches/oracles/manifest.toml";
const C_ORACLE: &str = "benches/oracles/c/vector_oracle.c";
const RUST_ORACLE: &str = "benches/oracles/rust/vector_oracle.rs";
const FIXTURES: &str = "benches/oracles/fixtures";
const VECTOR_CASES: [(&str, u8); 8] = [
    ("map_u32", 1),
    ("zip_u32", 2),
    ("strict_f64", 3),
    ("integer_cast", 4),
    ("modular_reduction", 5),
    ("slp_quad", 6),
    ("runtime_noalias", 7),
    ("specialized_length", 8),
];
const DOMAIN_CASES: [(&str, u8); 2] = [("contract_noalias", 9), ("contract_fixed_length", 10)];
const ORACLE_BATCH_ITERATIONS: usize = 20_000_000;
const QUICK_BATCH_ITERATIONS: usize = 200_000;
const ORACLE_LENGTH: usize = 4_000;
const COMPILE_SAMPLES: usize = 15;
// The AArch64 release workers expose a shared two-band frequency state for this
// four-element kernel. A long, identical, untimed ramp places every channel in
// the sustained state before the unchanged timed batch starts.
const SLP_CONDITIONING_BATCHES: usize = 32;
const ORACLE_SAMPLING_PROTOCOL: &str = "rotating-three-channel-v1";
const ORACLE_MANIFEST_SHA256: &str =
    "9bb4c1c7384a8d5c98e0b120d0daedcf88986be37f3e22487f43033798e8856d";

#[cfg(target_os = "linux")]
struct LinuxCpuAffinityGuard {
    previous: libc::cpu_set_t,
}

#[cfg(target_os = "linux")]
impl LinuxCpuAffinityGuard {
    fn pin_current() -> Result<Self, String> {
        let mut previous = std::mem::MaybeUninit::<libc::cpu_set_t>::zeroed();
        if unsafe {
            libc::sched_getaffinity(
                0,
                std::mem::size_of::<libc::cpu_set_t>(),
                previous.as_mut_ptr(),
            )
        } != 0
        {
            return Err(format!(
                "read Linux CPU affinity before performance measurement: {}",
                std::io::Error::last_os_error()
            ));
        }
        let previous = unsafe { previous.assume_init() };
        let current = unsafe { libc::sched_getcpu() };
        let selected = if current >= 0 && unsafe { libc::CPU_ISSET(current as usize, &previous) } {
            current as usize
        } else {
            (0..libc::CPU_SETSIZE as usize)
                .find(|cpu| unsafe { libc::CPU_ISSET(*cpu, &previous) })
                .ok_or_else(|| "Linux performance affinity has no allowed CPU".to_string())?
        };
        let mut pinned = unsafe { std::mem::zeroed::<libc::cpu_set_t>() };
        unsafe {
            libc::CPU_ZERO(&mut pinned);
            libc::CPU_SET(selected, &mut pinned);
        }
        if unsafe { libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &pinned) }
            != 0
        {
            return Err(format!(
                "pin Linux performance measurement to CPU {selected}: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(Self { previous })
    }
}

#[cfg(target_os = "linux")]
type RuntimeTimer = libc::timespec;

#[cfg(target_os = "linux")]
fn runtime_timer_start() -> Result<RuntimeTimer, String> {
    let mut value = std::mem::MaybeUninit::<libc::timespec>::uninit();
    if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, value.as_mut_ptr()) } != 0 {
        return Err(format!(
            "read Linux thread CPU time before runtime sample: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(unsafe { value.assume_init() })
}

#[cfg(target_os = "linux")]
fn runtime_timer_elapsed(timer: RuntimeTimer) -> Result<u128, String> {
    let end = runtime_timer_start()?;
    let start_ns = runtime_timespec_ns(timer)?;
    let end_ns = runtime_timespec_ns(end)?;
    Ok(end_ns
        .checked_sub(start_ns)
        .ok_or_else(|| "Linux thread CPU clock moved backwards".to_string())?
        .max(1))
}

#[cfg(target_os = "linux")]
fn runtime_timespec_ns(value: libc::timespec) -> Result<u128, String> {
    let seconds = u128::try_from(value.tv_sec)
        .map_err(|_| "Linux thread CPU time contains negative seconds".to_string())?;
    let nanos = u128::try_from(value.tv_nsec)
        .map_err(|_| "Linux thread CPU time contains negative nanoseconds".to_string())?;
    if nanos >= 1_000_000_000 {
        return Err("Linux thread CPU time contains invalid nanoseconds".into());
    }
    Ok(seconds * 1_000_000_000 + nanos)
}

#[cfg(not(target_os = "linux"))]
type RuntimeTimer = Instant;

#[cfg(not(target_os = "linux"))]
fn runtime_timer_start() -> Result<RuntimeTimer, String> {
    Ok(Instant::now())
}

#[cfg(not(target_os = "linux"))]
fn runtime_timer_elapsed(timer: RuntimeTimer) -> Result<u128, String> {
    Ok(timer.elapsed().as_nanos().max(1))
}

#[cfg(target_os = "linux")]
impl Drop for LinuxCpuAffinityGuard {
    fn drop(&mut self) {
        unsafe {
            libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &self.previous);
        }
    }
}

#[cfg(not(target_os = "linux"))]
struct LinuxCpuAffinityGuard;

#[cfg(not(target_os = "linux"))]
impl LinuxCpuAffinityGuard {
    fn pin_current() -> Result<Self, String> {
        Ok(Self)
    }
}

pub(super) struct VectorPerformanceReport {
    pub target_profile_digest: String,
    pub rust_version: String,
    pub vector_suites: Vec<OracleSuite>,
    pub domain_suites: Vec<OracleSuite>,
    pub artifacts: Vec<OracleArtifact>,
    pub sizes: Vec<ArtifactSizeComparison>,
    pub compile_times: Vec<CompileTimeComparison>,
}

pub(super) struct OracleSuite {
    mode: &'static str,
    cases: Vec<OracleCase>,
}

pub(super) struct OracleCase {
    name: String,
    prefixes: [&'static str; 3],
    samples: [Vec<u128>; 3],
    medians: [u128; 3],
    warmup_order: Vec<[usize; 3]>,
    sample_order: Vec<[usize; 3]>,
    result_digest: String,
    batch_iterations: usize,
}

pub(super) struct OracleArtifact {
    suite: &'static str,
    case_name: String,
    mode: &'static str,
    channel: &'static str,
    file: String,
    bytes: u64,
    sha256: String,
}

pub(super) struct ArtifactSizeComparison {
    case_name: String,
    mode: &'static str,
    source_sha256: String,
    candidate_bytes: u64,
    replay_v011_bytes: u64,
}

pub(super) struct CompileTimeComparison {
    case_name: String,
    mode: &'static str,
    source_sha256: String,
    candidate_samples_ns: Vec<u128>,
    replay_v011_samples_ns: Vec<u128>,
    warmup_order: Vec<[usize; 2]>,
    sample_order: Vec<[usize; 2]>,
}

pub(super) fn measure(
    repo_root: &Path,
    config: &Config,
    clang: &Path,
    evidence_root: &Path,
    replay_v011: &runtime_replay::RuntimeReplay,
) -> Result<VectorPerformanceReport, String> {
    if config.cpu_policy != CpuPolicy::Baseline {
        return Err("schema 7 measurement requires baseline CPU policy".into());
    }
    if !matches!((config.warmup, config.iterations), (3, 20) | (1, 5)) {
        return Err("schema 7 measurement accepts only the release schedule or explicit --quick diagnostics".into());
    }
    validate_oracle_manifest(repo_root)?;
    audit_oracles(repo_root, clang)?;
    let rust_version = pinned_rust_version()?;
    let candidate = candidate_compiler(repo_root)?;
    let replay_compiler = replay_v011
        .metadata
        .get("compilerSha256")
        .ok_or_else(|| "v0.11 replay compiler identity disappeared".to_string())?;
    if replay_compiler.is_empty() {
        return Err("v0.11 replay compiler identity is empty".into());
    }
    let replay_compiler = replay_v011
        .artifacts
        .first()
        .and_then(|artifact| artifact.path.parent())
        .ok_or_else(|| "v0.11 replay bundle path disappeared".to_string())?
        .join(replay_v011.generation.compiler_file());

    let target =
        NativeTarget::host_with_cpu(NativeCpu::Baseline).map_err(|error| error.to_string())?;
    let profile = target
        .kir_profile(KirConsumer::NativeLibrary)
        .map_err(|error| error.to_string())?;
    if profile.schema_version() != 1
        || KIR_VECTOR_COST_MODEL_SCHEMA != 1
        || KIR_VECTOR_PROOF_SCHEMA != 1
        || !profile.vector_operations_enabled()
    {
        return Err(
            "baseline Native target profile is not the frozen vector-capable schema".into(),
        );
    }

    let mut vector_suites = Vec::new();
    let mut domain_suites = Vec::new();
    let mut artifacts = Vec::new();
    let mut sizes = Vec::new();
    let mut compile_times = Vec::new();
    for checked in [false, true] {
        let mode = mode_name(checked);
        let mut vector_cases = Vec::new();
        let mut domain_cases = Vec::new();
        for (suite, corpus, prefixes) in [
            (
                "vector",
                VECTOR_CASES.as_slice(),
                ["candidate", "cSimd", "rustSimd"],
            ),
            (
                "domain",
                DOMAIN_CASES.as_slice(),
                ["candidate", "cGeneric", "rustGeneric"],
            ),
        ] {
            for &(name, case_number) in corpus {
                let fixture = repo_root.join(FIXTURES).join(format!("{name}.ck"));
                validate_candidate_kir(&fixture, checked, profile.clone(), suite == "vector")?;
                let source_sha256 = sha256_file(&fixture)?;
                let channels = [prefixes[0], prefixes[1], prefixes[2]];
                let candidate_path = evidence_root.join(format!(
                    "{suite}-{name}-{mode}-{}{}",
                    channels[0],
                    dynamic_suffix()
                ));
                compile_ck(&candidate, &fixture, &candidate_path, "dynamic", checked)?;
                let c_path = evidence_root.join(format!(
                    "{suite}-{name}-{mode}-{}{}",
                    channels[1],
                    dynamic_suffix()
                ));
                compile_c_oracle(repo_root, clang, case_number, checked, &c_path)?;
                let rust_path = evidence_root.join(format!(
                    "{suite}-{name}-{mode}-{}{}",
                    channels[2],
                    dynamic_suffix()
                ));
                compile_rust_oracle(repo_root, name, checked, &rust_path)?;
                for (channel, path) in
                    channels
                        .into_iter()
                        .zip([&candidate_path, &c_path, &rust_path])
                {
                    artifacts.push(OracleArtifact::new(suite, name, mode, channel, path)?);
                }
                let measured = measure_case(
                    name,
                    checked,
                    [&candidate_path, &c_path, &rust_path],
                    prefixes,
                    config,
                )?;
                if suite == "vector" {
                    vector_cases.push(measured);
                } else {
                    domain_cases.push(measured);
                }

                if suite == "vector" {
                    let candidate_object =
                        evidence_root.join(format!("size-{name}-{mode}-candidate.o"));
                    let replay_object =
                        evidence_root.join(format!("size-{name}-{mode}-replay-v011.o"));
                    compile_ck(&candidate, &fixture, &candidate_object, "object", checked)?;
                    compile_ck(
                        &replay_compiler,
                        &fixture,
                        &replay_object,
                        "object",
                        checked,
                    )?;
                    sizes.push(ArtifactSizeComparison {
                        case_name: name.into(),
                        mode,
                        source_sha256: source_sha256.clone(),
                        candidate_bytes: regular_size(&candidate_object)?,
                        replay_v011_bytes: regular_size(&replay_object)?,
                    });
                    compile_times.push(measure_compile_time(
                        evidence_root,
                        name,
                        mode,
                        &source_sha256,
                        &fixture,
                        &candidate,
                        &replay_compiler,
                        checked,
                    )?);
                }
            }
        }
        vector_suites.push(OracleSuite {
            mode,
            cases: vector_cases,
        });
        domain_suites.push(OracleSuite {
            mode,
            cases: domain_cases,
        });
    }
    for artifact in &artifacts {
        artifact.verify(evidence_root)?;
    }
    Ok(VectorPerformanceReport {
        target_profile_digest: profile.digest_hex(),
        rust_version,
        vector_suites,
        domain_suites,
        artifacts,
        sizes,
        compile_times,
    })
}

fn validate_oracle_manifest(repo_root: &Path) -> Result<(), String> {
    let manifest = repo_root.join(ORACLE_MANIFEST);
    if sha256_file(&manifest)? != ORACLE_MANIFEST_SHA256 {
        return Err("the pinned schema 7 oracle manifest has changed".into());
    }
    let text =
        fs::read_to_string(&manifest).map_err(|error| format!("read oracle manifest: {error}"))?;
    if !text.contains(ORACLE_SAMPLING_PROTOCOL) {
        return Err("oracle manifest does not pin rotating-three-channel-v1".into());
    }
    for (name, _) in VECTOR_CASES.into_iter().chain(DOMAIN_CASES) {
        let fixture = repo_root.join(FIXTURES).join(format!("{name}.ck"));
        let digest = sha256_file(&fixture)?;
        if !text.contains(&format!("name = \"{name}\""))
            || !text.contains(&format!("ck_sha256 = \"{digest}\""))
        {
            return Err(format!("oracle manifest does not bind {name}"));
        }
    }
    for relative in [C_ORACLE, RUST_ORACLE] {
        let digest = sha256_file(&repo_root.join(relative))?;
        if !text.contains(&format!("source = \"{relative}\""))
            || !text.contains(&format!("sha256 = \"{digest}\""))
        {
            return Err(format!("oracle manifest does not bind {relative}"));
        }
    }
    Ok(())
}

fn audit_oracles(repo_root: &Path, clang: &Path) -> Result<(), String> {
    let output = Command::new("python3")
        .arg("-B")
        .arg(repo_root.join("scripts/audit-performance-oracles.py"))
        .arg("--clang")
        .arg(clang)
        .output()
        .map_err(|error| format!("execute oracle audit: {error}"))?;
    if !output.status.success()
        || !String::from_utf8_lossy(&output.stdout).contains("oracle audit passed")
    {
        return Err(format!(
            "oracle differential/UB audit failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

fn pinned_rust_version() -> Result<String, String> {
    let output = Command::new("rustc")
        .arg("--version")
        .output()
        .map_err(|error| format!("inspect Rust oracle compiler: {error}"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() || !text.starts_with("rustc 1.90.0 ") {
        return Err(format!("Rust oracle compiler must be 1.90.0, got {text:?}"));
    }
    Ok("1.90.0".into())
}

fn candidate_compiler(repo_root: &Path) -> Result<PathBuf, String> {
    let path = env::var_os("CKC_CANDIDATE_COMPILER")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            repo_root.join(format!("target/release/ckc{}", env::consts::EXE_SUFFIX))
        });
    let output = Command::new(&path)
        .arg("--version")
        .output()
        .map_err(|error| format!("execute candidate compiler {}: {error}", path.display()))?;
    if !output.status.success()
        || !String::from_utf8_lossy(&output.stdout).starts_with("ckc 0.13.0")
    {
        return Err("CKC_CANDIDATE_COMPILER must identify ckc 0.13.0".into());
    }
    Ok(path)
}

fn validate_candidate_kir(
    fixture: &Path,
    checked: bool,
    profile: KirTargetProfile,
    require_vector: bool,
) -> Result<(), String> {
    let input = CaseInput {
        name: fixture.file_stem().unwrap().to_string_lossy().into_owned(),
        path: fixture.to_path_buf(),
        source_text: fs::read_to_string(fixture)
            .map_err(|error| format!("read {}: {error}", fixture.display()))?,
    };
    let checked_program = checked_program(&input)?;
    let mir = lower_to_mir(&checked_program).map_err(|error| error.to_string())?;
    let module = build_kir_module_with_profile(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::NativeLibrary,
            overflow_mode: if checked {
                KirOverflowMode::Checked
            } else {
                KirOverflowMode::Unchecked
            },
            bounds_mode: if checked {
                KirBoundsMode::Checked
            } else {
                KirBoundsMode::Unchecked
            },
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        profile,
    )
    .map_err(|error| error.to_string())?;
    let contracts =
        import_contract_facts(&module, &checked_program, 0).map_err(|error| error.to_string())?;
    let result = run_kir_pass_pipeline(module, KirOptimizationLevel::O3, Some(&contracts));
    let artifact = verified_artifact(&result)?;
    let native_llvm_reduction = fixture
        .file_stem()
        .is_some_and(|name| name == "modular_reduction")
        && matches!(
            artifact.profile.target_identity(),
            KirTargetIdentity::Native { triple } if triple.starts_with("x86_64-")
        )
        && result.analysis_fallbacks.iter().any(|fallback| {
            fallback.reason == "x86-horizontal-reduction-deferred-to-native-loop-vectorizer"
        });
    if require_vector
        && !checked
        && !native_llvm_reduction
        && !print_kir_module(artifact).contains("vector_")
    {
        return Err(format!(
            "{} did not materialize an accepted KIR vector operation",
            fixture.display()
        ));
    }
    Ok(())
}

fn compile_ck(
    compiler: &Path,
    fixture: &Path,
    output: &Path,
    kind: &str,
    checked: bool,
) -> Result<u128, String> {
    let timer = compile_timer_start()?;
    let result = Command::new(compiler)
        .arg("build")
        .arg(fixture)
        .args(["--kind", kind, "--out"])
        .arg(output)
        .args([
            "-O3",
            "--cpu",
            "baseline",
            "--overflow",
            mode_name(checked),
            "--bounds",
            mode_name(checked),
        ])
        .output()
        .map_err(|error| format!("execute {}: {error}", compiler.display()))?;
    if !result.status.success() {
        return Err(format!(
            "{} failed for {}: {}{}",
            compiler.display(),
            fixture.display(),
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    regular_size(output)?;
    compile_timer_elapsed(timer)
}

#[cfg(unix)]
type CompileTimer = u128;

#[cfg(not(unix))]
type CompileTimer = Instant;

#[cfg(unix)]
fn compile_timer_start() -> Result<CompileTimer, String> {
    terminated_child_cpu_time_ns()
}

#[cfg(not(unix))]
fn compile_timer_start() -> Result<CompileTimer, String> {
    Ok(Instant::now())
}

#[cfg(unix)]
fn compile_timer_elapsed(timer: CompileTimer) -> Result<u128, String> {
    Ok(terminated_child_cpu_time_ns()?
        .checked_sub(timer)
        .ok_or_else(|| "terminated-child CPU clock moved backwards".to_string())?
        .max(1))
}

#[cfg(not(unix))]
fn compile_timer_elapsed(timer: CompileTimer) -> Result<u128, String> {
    Ok(timer.elapsed().as_nanos().max(1))
}

#[cfg(unix)]
fn terminated_child_cpu_time_ns() -> Result<u128, String> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: getrusage initializes the pointed-to rusage on success. The pointer is valid,
    // properly aligned, and exclusively owned for the duration of the call.
    let status = unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, usage.as_mut_ptr()) };
    if status != 0 {
        return Err(format!(
            "read terminated-child CPU time: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: a zero getrusage status guarantees that usage was initialized.
    let usage = unsafe { usage.assume_init() };
    Ok(timeval_ns(usage.ru_utime)? + timeval_ns(usage.ru_stime)?)
}

#[cfg(unix)]
fn timeval_ns(value: libc::timeval) -> Result<u128, String> {
    let seconds = u128::try_from(value.tv_sec)
        .map_err(|_| "terminated-child CPU time contains negative seconds".to_string())?;
    let micros = u128::try_from(value.tv_usec)
        .map_err(|_| "terminated-child CPU time contains negative microseconds".to_string())?;
    if micros >= 1_000_000 {
        return Err("terminated-child CPU time contains invalid microseconds".to_string());
    }
    Ok(seconds * 1_000_000_000 + micros * 1_000)
}

fn compile_c_oracle(
    repo_root: &Path,
    clang: &Path,
    case_number: u8,
    checked: bool,
    output: &Path,
) -> Result<(), String> {
    let mut command = Command::new(clang);
    command.args([
        "-std=c11",
        "-O3",
        "-fno-fast-math",
        "-ffp-contract=off",
        "-fno-builtin",
        "-fno-unwind-tables",
        "-fno-asynchronous-unwind-tables",
        "-falign-functions=64",
        "-fuse-ld=lld",
        "-nostdlib",
    ]);
    command.arg(format!("-DORACLE_CASE={case_number}"));
    command.arg(format!("-DORACLE_CHECKED={}", u8::from(checked)));
    if cfg!(target_arch = "x86_64") {
        command.args(["-march=x86-64", "-mtune=generic"]);
    } else if cfg!(target_arch = "aarch64") {
        command.arg("-mcpu=generic");
    } else {
        return Err("schema 7 oracle has no baseline policy for this architecture".into());
    }
    add_dynamic_link_flags(&mut command);
    let result = command
        .arg(repo_root.join(C_ORACLE))
        .arg("-o")
        .arg(output)
        .output()
        .map_err(|error| format!("compile C SIMD oracle: {error}"))?;
    if !result.status.success() {
        return Err(format!(
            "C SIMD oracle failed: {}",
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    regular_size(output)?;
    Ok(())
}

fn compile_rust_oracle(
    repo_root: &Path,
    name: &str,
    checked: bool,
    output: &Path,
) -> Result<(), String> {
    let mut command = Command::new("rustc");
    command.args([
        "--edition",
        "2021",
        "--crate-type",
        "cdylib",
        "-Awarnings",
        "-C",
        "opt-level=3",
        "-C",
        "target-cpu=generic",
        "-C",
        "panic=abort",
        "--cfg",
    ]);
    command.arg(format!("oracle_case=\"{name}\""));
    if checked {
        command.args(["--cfg", "oracle_checked"]);
    }
    let result = command
        .arg(repo_root.join(RUST_ORACLE))
        .arg("-o")
        .arg(output)
        .output()
        .map_err(|error| format!("compile Rust SIMD oracle: {error}"))?;
    if !result.status.success() {
        return Err(format!(
            "Rust SIMD oracle failed: {}",
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    regular_size(output)?;
    Ok(())
}

fn add_dynamic_link_flags(command: &mut Command) {
    if cfg!(target_os = "macos") {
        command.args([
            "-dynamiclib",
            "-Wl,-platform_version,macos,11.0,11.0",
            "-Wl,-adhoc_codesign",
        ]);
    } else if cfg!(target_os = "windows") {
        command.args(["-shared", "-Wl,/noentry"]);
    } else {
        command.args(["-shared", "-fPIC", "-Wl,--no-undefined"]);
    }
}

fn regular_size(path: &Path) -> Result<u64, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("inspect {}: {error}", path.display()))?;
    if !metadata.file_type().is_file() || metadata.len() == 0 {
        return Err(format!(
            "{} must be a nonempty regular file",
            path.display()
        ));
    }
    Ok(metadata.len())
}

fn mode_name(checked: bool) -> &'static str {
    if checked { "checked" } else { "unchecked" }
}

fn measure_case(
    name: &str,
    checked: bool,
    paths: [&Path; 3],
    prefixes: [&'static str; 3],
    config: &Config,
) -> Result<OracleCase, String> {
    let _affinity = LinuxCpuAffinityGuard::pin_current()?;
    let mut runners = [
        KernelRunner::new(paths[0], "kernel", name, checked)?,
        KernelRunner::new(paths[1], "ck_oracle_kernel", name, checked)?,
        KernelRunner::new(paths[2], "ck_oracle_kernel", name, checked)?,
    ];
    let batch_iterations = if config.iterations <= 5 {
        QUICK_BATCH_ITERATIONS
    } else {
        ORACLE_BATCH_ITERATIONS
    };
    let expected = runners[0].run_batch(batch_iterations)?;
    for (channel, runner) in runners.iter_mut().enumerate().skip(1) {
        let actual = runner.run_batch(batch_iterations)?;
        if actual != expected {
            return Err(format!(
                "{name}/{} oracle channel {channel} is not equivalent",
                mode_name(checked)
            ));
        }
    }
    let sampled = runtime_replay::sample_three_channels(
        config.warmup,
        config.iterations,
        |channel, warmup| {
            if warmup {
                runners[channel].measure_once(&expected, batch_iterations)
            } else {
                runtime_replay::sample_upper_median::<_, SAMPLE_REPETITIONS>(|| {
                    runners[channel].measure_once(&expected, batch_iterations)
                })
            }
        },
    )?;
    let medians = std::array::from_fn(|channel| median(&sampled.channels[channel]));
    Ok(OracleCase {
        name: name.into(),
        prefixes,
        samples: sampled.channels,
        medians,
        warmup_order: sampled.warmup_order,
        sample_order: sampled.sample_order,
        result_digest: expected,
        batch_iterations,
    })
}

type MapUnchecked = unsafe extern "C" fn(*mut u32, u32, *mut u32, u32, u32);
type MapChecked = unsafe extern "C" fn(*mut u32, u32, *mut u32, u32, u32) -> i32;
type SpecializedUnchecked = unsafe extern "C" fn(*mut u32, u32, *mut u32, u32);
type SpecializedChecked = unsafe extern "C" fn(*mut u32, u32, *mut u32, u32) -> i32;
type ZipUnchecked = unsafe extern "C" fn(*mut u32, u32, *mut u32, u32, *mut u32, u32, u32);
type ZipChecked = unsafe extern "C" fn(*mut u32, u32, *mut u32, u32, *mut u32, u32, u32) -> i32;
type F64Unchecked = unsafe extern "C" fn(*mut f64, u32, *mut f64, u32, u32, f64);
type F64Checked = unsafe extern "C" fn(*mut f64, u32, *mut f64, u32, u32, f64) -> i32;
type CastUnchecked = unsafe extern "C" fn(*mut u32, u32, *mut f64, u32, u32);
type CastChecked = unsafe extern "C" fn(*mut u32, u32, *mut f64, u32, u32) -> i32;
type ReductionUnchecked = unsafe extern "C" fn(*mut u32, u32, u32) -> u32;
type ReductionChecked = unsafe extern "C" fn(*mut u32, u32, u32, *mut u32) -> i32;
type SlpUnchecked = unsafe extern "C" fn(*mut u32, u32, *mut u32, u32, *mut u32, u32);
type SlpChecked = unsafe extern "C" fn(*mut u32, u32, *mut u32, u32, *mut u32, u32) -> i32;

#[derive(Clone, Copy)]
enum KernelEntry {
    MapUnchecked(MapUnchecked),
    MapChecked(MapChecked),
    SpecializedUnchecked(SpecializedUnchecked),
    SpecializedChecked(SpecializedChecked),
    ZipUnchecked(ZipUnchecked),
    ZipChecked(ZipChecked),
    F64Unchecked(F64Unchecked),
    F64Checked(F64Checked),
    CastUnchecked(CastUnchecked),
    CastChecked(CastChecked),
    ReductionUnchecked(ReductionUnchecked),
    ReductionChecked(ReductionChecked),
    SlpUnchecked(SlpUnchecked),
    SlpChecked(SlpChecked),
}

impl KernelEntry {
    unsafe fn load(
        library: &DynamicLibrary,
        symbol: &str,
        name: &str,
        checked: bool,
    ) -> Result<Self, String> {
        match (name, checked) {
            ("zip_u32", false) => Ok(Self::ZipUnchecked(unsafe { library.symbol(symbol)? })),
            ("zip_u32", true) => Ok(Self::ZipChecked(unsafe { library.symbol(symbol)? })),
            ("strict_f64", false) => Ok(Self::F64Unchecked(unsafe { library.symbol(symbol)? })),
            ("strict_f64", true) => Ok(Self::F64Checked(unsafe { library.symbol(symbol)? })),
            ("integer_cast", false) => Ok(Self::CastUnchecked(unsafe { library.symbol(symbol)? })),
            ("integer_cast", true) => Ok(Self::CastChecked(unsafe { library.symbol(symbol)? })),
            ("modular_reduction", false) => {
                Ok(Self::ReductionUnchecked(unsafe { library.symbol(symbol)? }))
            }
            ("modular_reduction", true) => {
                Ok(Self::ReductionChecked(unsafe { library.symbol(symbol)? }))
            }
            ("slp_quad", false) => Ok(Self::SlpUnchecked(unsafe { library.symbol(symbol)? })),
            ("slp_quad", true) => Ok(Self::SlpChecked(unsafe { library.symbol(symbol)? })),
            ("specialized_length", false) => Ok(Self::SpecializedUnchecked(unsafe {
                library.symbol(symbol)?
            })),
            ("specialized_length", true) => {
                Ok(Self::SpecializedChecked(unsafe { library.symbol(symbol)? }))
            }
            (
                "map_u32" | "runtime_noalias" | "contract_noalias" | "contract_fixed_length",
                false,
            ) => Ok(Self::MapUnchecked(unsafe { library.symbol(symbol)? })),
            (
                "map_u32" | "runtime_noalias" | "contract_noalias" | "contract_fixed_length",
                true,
            ) => Ok(Self::MapChecked(unsafe { library.symbol(symbol)? })),
            _ => Err(format!("unsupported vector performance case {name}")),
        }
    }
}

struct KernelRunner {
    _library: DynamicLibrary,
    entry: KernelEntry,
    name: String,
    a_u32: Vec<u32>,
    b_u32: Vec<u32>,
    out_u32: Vec<u32>,
    a_f64: Vec<f64>,
    out_f64: Vec<f64>,
}

impl KernelRunner {
    fn new(path: &Path, symbol: &'static str, name: &str, checked: bool) -> Result<Self, String> {
        let library = DynamicLibrary::open(path)?;
        let entry = unsafe { KernelEntry::load(&library, symbol, name, checked)? };
        let a_u32 = (0..ORACLE_LENGTH)
            .map(|index| {
                ((index as u32).wrapping_add(7)).wrapping_mul(2_654_435_761) % 1_000_002 + 1
            })
            .collect();
        let b_u32 = (0..ORACLE_LENGTH)
            .map(|index| {
                ((index as u32).wrapping_add(19)).wrapping_mul(2_654_435_761) % 1_000_002 + 1
            })
            .collect();
        let a_f64 = (0..ORACLE_LENGTH)
            .map(|index| (index as f64 - 2_000.0) / 16.0 + 0.25)
            .collect();
        Ok(Self {
            _library: library,
            entry,
            name: name.into(),
            a_u32,
            b_u32,
            out_u32: vec![0; ORACLE_LENGTH],
            a_f64,
            out_f64: vec![0.0; ORACLE_LENGTH],
        })
    }

    fn measure_once(&mut self, expected: &str, batch_iterations: usize) -> Result<u128, String> {
        if self.name == "slp_quad" {
            for _ in 0..SLP_CONDITIONING_BATCHES {
                self.invoke_repeated(batch_iterations)?;
            }
        }
        let timer = runtime_timer_start()?;
        self.invoke_repeated(batch_iterations)?;
        let elapsed = runtime_timer_elapsed(timer)?;
        let actual = self.result_digest();
        if actual != expected {
            return Err(format!(
                "{} produced a different result during measurement",
                self.name
            ));
        }
        Ok(elapsed)
    }

    fn run_batch(&mut self, batch_iterations: usize) -> Result<String, String> {
        self.invoke_repeated(batch_iterations)?;
        Ok(self.result_digest())
    }

    fn invoke_repeated(&mut self, batch_iterations: usize) -> Result<(), String> {
        let work_items = work_items(&self.name);
        let calls = batch_iterations / work_items;
        debug_assert_eq!(calls * work_items, batch_iterations);
        let n = u32::try_from(work_items).map_err(|_| "oracle work size is not u32")?;
        match self.entry {
            KernelEntry::MapUnchecked(function) => {
                for _ in 0..calls {
                    unsafe {
                        function(self.a_u32.as_mut_ptr(), n, self.out_u32.as_mut_ptr(), n, n)
                    };
                }
            }
            KernelEntry::MapChecked(function) => {
                for _ in 0..calls {
                    if unsafe {
                        function(self.a_u32.as_mut_ptr(), n, self.out_u32.as_mut_ptr(), n, n)
                    } != 0
                    {
                        return Err("checked map oracle failed".into());
                    }
                }
            }
            KernelEntry::SpecializedUnchecked(function) => {
                for _ in 0..calls {
                    unsafe { function(self.a_u32.as_mut_ptr(), n, self.out_u32.as_mut_ptr(), n) };
                }
            }
            KernelEntry::SpecializedChecked(function) => {
                for _ in 0..calls {
                    if unsafe { function(self.a_u32.as_mut_ptr(), n, self.out_u32.as_mut_ptr(), n) }
                        != 0
                    {
                        return Err("checked specialized oracle failed".into());
                    }
                }
            }
            KernelEntry::ZipUnchecked(function) => {
                for _ in 0..calls {
                    unsafe {
                        function(
                            self.a_u32.as_mut_ptr(),
                            n,
                            self.b_u32.as_mut_ptr(),
                            n,
                            self.out_u32.as_mut_ptr(),
                            n,
                            n,
                        )
                    };
                }
            }
            KernelEntry::ZipChecked(function) => {
                for _ in 0..calls {
                    if unsafe {
                        function(
                            self.a_u32.as_mut_ptr(),
                            n,
                            self.b_u32.as_mut_ptr(),
                            n,
                            self.out_u32.as_mut_ptr(),
                            n,
                            n,
                        )
                    } != 0
                    {
                        return Err("checked zip oracle failed".into());
                    }
                }
            }
            KernelEntry::F64Unchecked(function) => {
                for _ in 0..calls {
                    unsafe {
                        function(
                            self.a_f64.as_mut_ptr(),
                            n,
                            self.out_f64.as_mut_ptr(),
                            n,
                            n,
                            1.0009765625,
                        )
                    };
                }
            }
            KernelEntry::F64Checked(function) => {
                for _ in 0..calls {
                    if unsafe {
                        function(
                            self.a_f64.as_mut_ptr(),
                            n,
                            self.out_f64.as_mut_ptr(),
                            n,
                            n,
                            1.0009765625,
                        )
                    } != 0
                    {
                        return Err("checked f64 oracle failed".into());
                    }
                }
            }
            KernelEntry::CastUnchecked(function) => {
                for _ in 0..calls {
                    unsafe {
                        function(self.a_u32.as_mut_ptr(), n, self.out_f64.as_mut_ptr(), n, n)
                    };
                }
            }
            KernelEntry::CastChecked(function) => {
                for _ in 0..calls {
                    if unsafe {
                        function(self.a_u32.as_mut_ptr(), n, self.out_f64.as_mut_ptr(), n, n)
                    } != 0
                    {
                        return Err("checked cast oracle failed".into());
                    }
                }
            }
            KernelEntry::ReductionUnchecked(function) => {
                for _ in 0..calls {
                    self.out_u32[0] = unsafe { function(self.a_u32.as_mut_ptr(), n, n) };
                }
            }
            KernelEntry::ReductionChecked(function) => {
                for _ in 0..calls {
                    if unsafe { function(self.a_u32.as_mut_ptr(), n, n, self.out_u32.as_mut_ptr()) }
                        != 0
                    {
                        return Err("checked reduction oracle failed".into());
                    }
                }
            }
            KernelEntry::SlpUnchecked(function) => {
                for _ in 0..calls {
                    unsafe {
                        function(
                            self.a_u32.as_mut_ptr(),
                            n,
                            self.b_u32.as_mut_ptr(),
                            n,
                            self.out_u32.as_mut_ptr(),
                            n,
                        )
                    };
                }
            }
            KernelEntry::SlpChecked(function) => {
                for _ in 0..calls {
                    if unsafe {
                        function(
                            self.a_u32.as_mut_ptr(),
                            n,
                            self.b_u32.as_mut_ptr(),
                            n,
                            self.out_u32.as_mut_ptr(),
                            n,
                        )
                    } != 0
                    {
                        return Err("checked SLP oracle failed".into());
                    }
                }
            }
        }
        Ok(())
    }

    fn result_digest(&self) -> String {
        let bytes: &[u8] = if matches!(self.name.as_str(), "strict_f64" | "integer_cast") {
            unsafe {
                std::slice::from_raw_parts(self.out_f64.as_ptr().cast(), self.out_f64.len() * 8)
            }
        } else if self.name == "modular_reduction" {
            unsafe { std::slice::from_raw_parts(self.out_u32.as_ptr().cast(), 4) }
        } else {
            unsafe {
                std::slice::from_raw_parts(self.out_u32.as_ptr().cast(), self.out_u32.len() * 4)
            }
        };
        format!("{:x}", Sha256::digest(bytes))
    }
}

fn work_items(name: &str) -> usize {
    match name {
        "slp_quad" => 4,
        "contract_noalias" | "contract_fixed_length" => 16,
        _ => ORACLE_LENGTH,
    }
}

#[allow(clippy::too_many_arguments)]
fn measure_compile_time(
    root: &Path,
    name: &str,
    mode: &'static str,
    source_sha256: &str,
    fixture: &Path,
    candidate: &Path,
    replay: &Path,
    checked: bool,
) -> Result<CompileTimeComparison, String> {
    let mut candidate_samples_ns = Vec::with_capacity(COMPILE_SAMPLES);
    let mut replay_v011_samples_ns = Vec::with_capacity(COMPILE_SAMPLES);
    let mut warmup_order = Vec::new();
    let mut sample_order = Vec::new();
    let mut serial = 0usize;
    for round in 0..3 {
        let order = if round % 2 == 0 { [0, 1] } else { [1, 0] };
        for channel in order {
            let output = root.join(format!("compile-{name}-{mode}-warmup-{serial}-{channel}.o"));
            compile_ck(
                if channel == 0 { candidate } else { replay },
                fixture,
                &output,
                "object",
                checked,
            )?;
            serial += 1;
        }
        warmup_order.push(order);
    }
    for round in 0..COMPILE_SAMPLES {
        let order = if round % 2 == 0 { [0, 1] } else { [1, 0] };
        for channel in order {
            let output = root.join(format!("compile-{name}-{mode}-sample-{serial}-{channel}.o"));
            let elapsed = compile_ck(
                if channel == 0 { candidate } else { replay },
                fixture,
                &output,
                "object",
                checked,
            )?;
            (if channel == 0 {
                &mut candidate_samples_ns
            } else {
                &mut replay_v011_samples_ns
            })
            .push(elapsed);
            serial += 1;
        }
        sample_order.push(order);
    }
    Ok(CompileTimeComparison {
        case_name: name.into(),
        mode,
        source_sha256: source_sha256.into(),
        candidate_samples_ns,
        replay_v011_samples_ns,
        warmup_order,
        sample_order,
    })
}

impl OracleArtifact {
    fn new(
        suite: &'static str,
        case_name: &str,
        mode: &'static str,
        channel: &'static str,
        path: &Path,
    ) -> Result<Self, String> {
        Ok(Self {
            suite,
            case_name: case_name.into(),
            mode,
            channel,
            file: path.file_name().unwrap().to_string_lossy().into_owned(),
            bytes: regular_size(path)?,
            sha256: sha256_file(path)?,
        })
    }

    fn verify(&self, root: &Path) -> Result<(), String> {
        let path = root.join(&self.file);
        if regular_size(&path)? != self.bytes || sha256_file(&path)? != self.sha256 {
            return Err(format!("oracle artifact changed: {}", self.file));
        }
        Ok(())
    }

    fn to_json(&self) -> String {
        format!(
            r#"{{"suite":"{}","case":"{}","mode":"{}","channel":"{}","file":"{}","bytes":{},"sha256":"{}"}}"#,
            self.suite,
            json_escape(&self.case_name),
            self.mode,
            self.channel,
            json_escape(&self.file),
            self.bytes,
            self.sha256
        )
    }
}

impl OracleCase {
    fn to_json(&self) -> String {
        let mut fields = vec![
            format!(r#""name":"{}""#, json_escape(&self.name)),
            "\"referenceEquivalent\":true".into(),
            "\"validDomain\":true".into(),
            format!(r#""resultDigest":"{}""#, self.result_digest),
            format!("\"batchIterations\":{}", self.batch_iterations),
            format!(
                "\"warmupOrder\":{}",
                json_sampling_order(&self.warmup_order)
            ),
            format!(
                "\"sampleOrder\":{}",
                json_sampling_order(&self.sample_order)
            ),
        ];
        for channel in 0..3 {
            fields.push(format!(
                "\"{}MedianNs\":{}",
                self.prefixes[channel], self.medians[channel]
            ));
            fields.push(format!(
                "\"{}SamplesNs\":{}",
                self.prefixes[channel],
                json_u128_array(&self.samples[channel])
            ));
        }
        format!("{{{}}}", fields.join(","))
    }
}

impl OracleSuite {
    fn to_json(&self) -> String {
        format!(
            r#"{{"mode":"{}","cases":[{}]}}"#,
            self.mode,
            self.cases
                .iter()
                .map(OracleCase::to_json)
                .collect::<Vec<_>>()
                .join(",")
        )
    }
}

impl ArtifactSizeComparison {
    fn to_json(&self) -> String {
        format!(
            r#"{{"case":"{}","mode":"{}","sourceSha256":"{}","candidateBytes":{},"replayV011Bytes":{}}}"#,
            json_escape(&self.case_name),
            self.mode,
            self.source_sha256,
            self.candidate_bytes,
            self.replay_v011_bytes
        )
    }
}

impl CompileTimeComparison {
    fn to_json(&self) -> String {
        format!(
            r#"{{"case":"{}","mode":"{}","sourceSha256":"{}","candidateMedianNs":{},"candidateSamplesNs":{},"replayV011MedianNs":{},"replayV011SamplesNs":{},"warmupOrder":{},"sampleOrder":{}}}"#,
            json_escape(&self.case_name),
            self.mode,
            self.source_sha256,
            median(&self.candidate_samples_ns),
            json_u128_array(&self.candidate_samples_ns),
            median(&self.replay_v011_samples_ns),
            json_u128_array(&self.replay_v011_samples_ns),
            json_sampling_order(&self.warmup_order),
            json_sampling_order(&self.sample_order)
        )
    }
}

impl VectorPerformanceReport {
    pub fn target_profile_json(&self) -> String {
        format!(
            r#"{{"digest":"{}","costSchema":1,"proofSchema":1,"budgetSchema":1}}"#,
            self.target_profile_digest
        )
    }

    pub fn vector_suites_json(&self) -> String {
        format!(
            "[{}]",
            self.vector_suites
                .iter()
                .map(OracleSuite::to_json)
                .collect::<Vec<_>>()
                .join(",")
        )
    }

    pub fn domain_suites_json(&self) -> String {
        format!(
            "[{}]",
            self.domain_suites
                .iter()
                .map(OracleSuite::to_json)
                .collect::<Vec<_>>()
                .join(",")
        )
    }

    pub fn identity_json(&self) -> String {
        format!(
            r#"{{"manifestSha256":"{ORACLE_MANIFEST_SHA256}","clangVersion":"22.1.8","rustVersion":"{}","fastMath":false,"contraction":false,"differentialAudit":true,"ubAudit":true}}"#,
            self.rust_version
        )
    }

    pub fn artifacts_json(&self) -> String {
        format!(
            "[{}]",
            self.artifacts
                .iter()
                .map(OracleArtifact::to_json)
                .collect::<Vec<_>>()
                .join(",")
        )
    }

    pub fn sizes_json(&self) -> String {
        format!(
            "[{}]",
            self.sizes
                .iter()
                .map(ArtifactSizeComparison::to_json)
                .collect::<Vec<_>>()
                .join(",")
        )
    }

    pub fn compile_times_json(&self) -> String {
        format!(
            "[{}]",
            self.compile_times
                .iter()
                .map(CompileTimeComparison::to_json)
                .collect::<Vec<_>>()
                .join(",")
        )
    }
}
