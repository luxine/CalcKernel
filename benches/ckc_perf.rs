use std::{
    env, fs,
    hint::black_box,
    path::{Path, PathBuf},
    process,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(feature = "native-toolchain")]
use std::process::Command;

#[cfg(feature = "native-toolchain")]
use sha2::{Digest, Sha256};

use calckernel::{
    BoundsMode, EmitWasmOptions, KirBoundsMode, KirBuildConfig, KirConsumer, KirOptimizationLevel,
    KirOverflowMode, KirPassManagerResult, KirSanitizerMode, OverflowMode, SourceFile,
    build_kir_module, check, emit_c_kir_module_with_contracts, emit_wasm_kir_module,
    emit_wat_kir_module, format_diagnostics, import_contract_facts, lower_to_mir, print_kir_module,
    print_mir_module, run_kir_pass_pipeline,
};

#[cfg(feature = "native-toolchain")]
use calckernel::{
    EmitLlvmOptions, NativeContext, NativeCpu, NativeOptimizationLevel, NativeTarget,
    link_native_dynamic_library, lower_native_kir_module,
};

const USAGE: &str = "cargo bench --features native-toolchain --bench ckc_perf -- [--quick] [--case <name>] [--task <name>] [--iterations <n>] [--warmup <n>] [--cpu baseline|native] [--out-dir <path>]\n\nDefault outputs: build/perf/latest.summary.json, build/perf/latest.summary.md, and target/ckc-perf/results.json";
const CASE_MANIFEST: &str = "benches/cases/native-cases.tsv";
const FIXTURE_ROOT: &str = "benches/fixtures";
#[cfg(feature = "native-toolchain")]
const NATIVE_RUNTIME_FIXTURE_ROOT: &str = "tests/fixtures/performance/native";
const DEFAULT_OUT_DIR: &str = "build/perf";
#[cfg(feature = "native-toolchain")]
const NATIVE_RESULTS_PATH: &str = "target/ckc-perf/results.json";
#[cfg(feature = "native-toolchain")]
const V0_10_BASELINE_PATH: &str = "benches/baselines/v0_10_compiler.toml";
#[cfg(feature = "native-toolchain")]
const SAMPLE_REPETITIONS: usize = 7;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let config = Config::parse(env::args().skip(1))?;
    if config.help {
        println!("{USAGE}");
        return Ok(());
    }

    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let cases = read_cases(&repo_root.join(CASE_MANIFEST))?;
    let tasks = benchmark_tasks();
    let selected_cases = filter_cases(&cases, config.case_filter.as_deref())?;
    let selected_tasks = filter_tasks(&tasks, config.task_filter.as_deref())?;

    println!(
        "Running {} case(s) x {} task(s), warmup={}, iterations={}",
        selected_cases.len(),
        selected_tasks.len(),
        config.warmup,
        config.iterations
    );

    let mut results = Vec::new();
    for case in selected_cases {
        let input = CaseInput::load(&repo_root, case)?;
        for task in &selected_tasks {
            let result = measure(&input, task, &config)?;
            println!(
                "{}/{} median={:.3}ms p95={:.3}ms",
                result.case_name,
                result.task_name,
                nanos_to_millis(result.median_ns),
                nanos_to_millis(result.p95_ns)
            );
            results.push(result);
        }
    }

    let metadata = SummaryMetadata::new(config.iterations, config.warmup);
    let summary = Summary { metadata, results };
    let out_dir = repo_root.join(&config.out_dir);
    fs::create_dir_all(&out_dir)
        .map_err(|error| format!("failed to create {}: {error}", out_dir.display()))?;

    let json_path = out_dir.join("latest.summary.json");
    let markdown_path = out_dir.join("latest.summary.md");
    fs::write(&json_path, summary.to_json())
        .map_err(|error| format!("failed to write {}: {error}", json_path.display()))?;
    fs::write(&markdown_path, summary.to_markdown())
        .map_err(|error| format!("failed to write {}: {error}", markdown_path.display()))?;

    #[cfg(feature = "native-toolchain")]
    {
        let baseline = CompilerBaseline::load(&repo_root.join(V0_10_BASELINE_PATH))?;
        let runtime = measure_native_runtime(&repo_root, &config, &baseline, &cases)?;
        let runtime_path = repo_root.join(NATIVE_RESULTS_PATH);
        if let Some(parent) = runtime_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
        }
        fs::write(&runtime_path, runtime.to_json())
            .map_err(|error| format!("failed to write {}: {error}", runtime_path.display()))?;
        println!("Wrote {}", relative_to_repo(&repo_root, &runtime_path));
    }

    println!("Wrote {}", relative_to_repo(&repo_root, &json_path));
    println!("Wrote {}", relative_to_repo(&repo_root, &markdown_path));
    Ok(())
}

#[derive(Debug, Clone)]
struct Config {
    help: bool,
    iterations: usize,
    warmup: usize,
    case_filter: Option<String>,
    task_filter: Option<String>,
    out_dir: PathBuf,
    cpu_policy: CpuPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CpuPolicy {
    Baseline,
    Native,
}

#[cfg(feature = "native-toolchain")]
#[derive(Debug, Clone)]
struct CompilerBaseline {
    commit: String,
    compiler_identity: String,
    llvm_version: String,
    harness: String,
    statistics: String,
    source_digest_count: usize,
    source_digests: Vec<(String, String)>,
    runtime: Vec<BaselineRuntime>,
    optimizer: Vec<BaselineOptimizer>,
}

#[cfg(feature = "native-toolchain")]
#[derive(Debug, Clone)]
struct BaselineRuntime {
    target: String,
    cpu: String,
    mode: String,
    case_name: String,
    median_ns: u128,
}

#[cfg(feature = "native-toolchain")]
#[derive(Debug, Clone)]
struct BaselineOptimizer {
    target: String,
    case_name: String,
    median_ns: u128,
}

#[cfg(feature = "native-toolchain")]
impl CompilerBaseline {
    fn load(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        let scalar = |key: &str| baseline_value(&text, key);
        let runtime = text
            .split("[[runtime]]")
            .skip(1)
            .map(|block| {
                let block = baseline_section(block);
                Ok(BaselineRuntime {
                    target: baseline_value(block, "target")?,
                    cpu: baseline_value(block, "cpu")?,
                    mode: baseline_value(block, "mode")?,
                    case_name: baseline_value(block, "case")?,
                    median_ns: baseline_value(block, "median_ns")?
                        .parse()
                        .map_err(|error| format!("invalid runtime median_ns: {error}"))?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let optimizer = text
            .split("[[optimizer]]")
            .skip(1)
            .map(|block| {
                let block = baseline_section(block);
                Ok(BaselineOptimizer {
                    target: baseline_value(block, "target")?,
                    case_name: baseline_value(block, "case")?,
                    median_ns: baseline_value(block, "median_ns")?
                        .parse()
                        .map_err(|error| format!("invalid optimizer median_ns: {error}"))?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let source_specs = [
            (
                "branch_mix",
                "source_digest_branch_mix",
                &["tests/fixtures/performance/native/branch_mix.ck"][..],
            ),
            (
                "integer_accumulate",
                "source_digest_integer_accumulate",
                &["tests/fixtures/performance/native/integer_accumulate.ck"][..],
            ),
            (
                "proof_loop",
                "source_digest_proof_loop",
                &[
                    "tests/fixtures/performance/native/proof_loop.ck",
                    "benches/fixtures/proof_loop.ck",
                ][..],
            ),
            (
                "remainder_chain",
                "source_digest_remainder_chain",
                &["tests/fixtures/performance/native/remainder_chain.ck"][..],
            ),
            (
                "pricing",
                "source_digest_pricing",
                &["benches/fixtures/pricing_helpers.ck"][..],
            ),
            (
                "pricing_soa",
                "source_digest_pricing_soa",
                &["benches/fixtures/pricing_soa.ck"][..],
            ),
            (
                "f64_kernels",
                "source_digest_f64_kernels",
                &["benches/fixtures/f64_kernels.ck"][..],
            ),
            (
                "example_pricing",
                "source_digest_example_pricing",
                &["examples/applications/pricing.ck"][..],
            ),
            (
                "example_dijkstra",
                "source_digest_example_dijkstra",
                &["examples/applications/dijkstra.ck"][..],
            ),
        ];
        let repo_root = path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .ok_or_else(|| format!("baseline path has no repository root: {}", path.display()))?;
        let mut source_digests = Vec::new();
        for (name, key, relative_paths) in source_specs {
            let expected = scalar(key)?;
            if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(format!("v0.10 baseline {key} is not a SHA-256 digest"));
            }
            for relative_path in relative_paths {
                let source_path = repo_root.join(relative_path);
                let actual = sha256_file(&source_path)?;
                if actual != expected {
                    return Err(format!(
                        "v0.10 baseline digest mismatch for {relative_path}: expected {expected}, got {actual}"
                    ));
                }
            }
            source_digests.push((name.to_string(), expected));
        }
        let baseline = Self {
            commit: scalar("commit")?,
            compiler_identity: scalar("compiler_identity")?,
            llvm_version: scalar("llvm_version")?,
            harness: scalar("harness")?,
            statistics: scalar("statistics")?,
            source_digest_count: text
                .lines()
                .filter(|line| line.trim_start().starts_with("source_digest_"))
                .count(),
            source_digests,
            runtime,
            optimizer,
        };
        if baseline.commit != "df816502876fba41676f9ebc190e4fadd18cd5a5"
            || baseline.compiler_identity
                != "calckernel 0.10.0 (df816502876fba41676f9ebc190e4fadd18cd5a5)"
            || baseline.llvm_version != "22.1.8"
            || baseline.source_digest_count != source_specs.len()
            || baseline.source_digests.len() != source_specs.len()
        {
            return Err("v0.10 baseline identity or source digest set is incomplete".to_string());
        }
        Ok(baseline)
    }

    fn runtime_median(
        &self,
        cpu_policy: CpuPolicy,
        mode: &str,
        case_name: &str,
    ) -> Result<u128, String> {
        let target = host_target_name();
        let cpu = cpu_policy_name(cpu_policy);
        self.runtime
            .iter()
            .find(|entry| {
                entry.target == target
                    && entry.cpu == cpu
                    && entry.mode == mode
                    && entry.case_name == case_name
            })
            .map(|entry| entry.median_ns)
            .ok_or_else(|| {
                format!("v0.10 baseline is missing runtime {target}/{cpu}/{mode}/{case_name}")
            })
    }

    fn optimizer_median(&self, case_name: &str) -> Result<u128, String> {
        let target = host_target_name();
        self.optimizer
            .iter()
            .find(|entry| entry.target == target && entry.case_name == case_name)
            .map(|entry| entry.median_ns)
            .ok_or_else(|| format!("v0.10 baseline is missing optimizer {target}/{case_name}"))
    }
}

#[cfg(feature = "native-toolchain")]
fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("failed to hash {}: {error}", path.display()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

#[cfg(feature = "native-toolchain")]
fn baseline_section(block: &str) -> &str {
    block.split("[[").next().unwrap_or(block)
}

#[cfg(feature = "native-toolchain")]
fn baseline_value(text: &str, key: &str) -> Result<String, String> {
    text.lines()
        .map(str::trim)
        .find_map(|line| {
            let value = line.strip_prefix(key)?.strip_prefix(" = ")?.trim();
            Some(value.trim_matches('"').to_string())
        })
        .ok_or_else(|| format!("v0.10 baseline is missing `{key}`"))
}

#[cfg(feature = "native-toolchain")]
fn host_target_name() -> String {
    format!("{}-{}", env::consts::OS, env::consts::ARCH)
}

#[cfg(feature = "native-toolchain")]
fn cpu_policy_name(policy: CpuPolicy) -> &'static str {
    match policy {
        CpuPolicy::Baseline => "baseline",
        CpuPolicy::Native => "native",
    }
}

impl Config {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut config = Self {
            help: false,
            iterations: 20,
            warmup: 3,
            case_filter: None,
            task_filter: None,
            out_dir: PathBuf::from(DEFAULT_OUT_DIR),
            cpu_policy: CpuPolicy::Baseline,
        };
        let mut args = args.into_iter();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-h" | "--help" => config.help = true,
                "--bench" => {}
                "--quick" => {
                    config.iterations = 5;
                    config.warmup = 1;
                }
                "--iterations" => {
                    config.iterations = parse_count("--iterations", args.next())?;
                }
                "--warmup" => {
                    config.warmup = parse_count("--warmup", args.next())?;
                }
                "--case" => config.case_filter = Some(require_value("--case", args.next())?),
                "--task" => config.task_filter = Some(require_value("--task", args.next())?),
                "--out-dir" => {
                    config.out_dir = PathBuf::from(require_value("--out-dir", args.next())?);
                }
                "--cpu" => {
                    config.cpu_policy = match require_value("--cpu", args.next())?.as_str() {
                        "baseline" => CpuPolicy::Baseline,
                        "native" => CpuPolicy::Native,
                        value => {
                            return Err(format!("--cpu must be baseline or native, got {value}"));
                        }
                    };
                }
                other => return Err(format!("unknown argument `{other}`\n\n{USAGE}")),
            }
        }

        if config.iterations < 3 {
            return Err("--iterations must be at least 3 for stability checks".to_string());
        }
        if config.warmup == 0 {
            return Err("--warmup must be greater than 0".to_string());
        }
        Ok(config)
    }
}

fn require_value(flag: &str, value: Option<String>) -> Result<String, String> {
    value.ok_or_else(|| format!("{flag} requires a value"))
}

fn parse_count(flag: &str, value: Option<String>) -> Result<usize, String> {
    let raw = require_value(flag, value)?;
    raw.parse::<usize>()
        .map_err(|error| format!("{flag} must be an integer: {error}"))
}

#[derive(Debug, Clone)]
struct Case {
    name: String,
    path: PathBuf,
}

#[derive(Debug)]
struct CaseInput {
    name: String,
    path: PathBuf,
    source_text: String,
}

impl CaseInput {
    fn load(repo_root: &Path, case: &Case) -> Result<Self, String> {
        let path = repo_root.join(&case.path);
        let source_text = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        Ok(Self {
            name: case.name.clone(),
            path: case.path.clone(),
            source_text,
        })
    }
}

fn read_cases(path: &Path) -> Result<Vec<Case>, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let mut cases = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let mut parts = line.split('\t');
        let Some(name) = parts.next() else {
            continue;
        };
        let Some(case_path) = parts.next() else {
            return Err(format!(
                "{}:{} must contain `<name>\\t<path>`",
                path.display(),
                index + 1
            ));
        };
        if parts.next().is_some() {
            return Err(format!(
                "{}:{} has too many tab-separated fields",
                path.display(),
                index + 1
            ));
        }
        if !case_path.starts_with(FIXTURE_ROOT) && !case_path.starts_with("examples/") {
            return Err(format!(
                "{}:{} path must live under `{FIXTURE_ROOT}` or `examples/`",
                path.display(),
                index + 1
            ));
        }
        cases.push(Case {
            name: name.to_string(),
            path: PathBuf::from(case_path),
        });
    }

    if cases.is_empty() {
        return Err(format!("{} did not define any cases", path.display()));
    }
    Ok(cases)
}

fn filter_cases<'a>(cases: &'a [Case], filter: Option<&str>) -> Result<Vec<&'a Case>, String> {
    let selected = cases
        .iter()
        .filter(|case| filter.is_none_or(|filter| case.name == filter))
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return Err(format!(
            "no benchmark case matched `{}`",
            filter.unwrap_or("")
        ));
    }
    Ok(selected)
}

fn filter_tasks<'a>(tasks: &'a [Task], filter: Option<&str>) -> Result<Vec<&'a Task>, String> {
    let selected = tasks
        .iter()
        .filter(|task| filter.is_none_or(|filter| task.name == filter))
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return Err(format!(
            "no benchmark task matched `{}`",
            filter.unwrap_or("")
        ));
    }
    Ok(selected)
}

#[derive(Clone, Copy)]
struct Task {
    name: &'static str,
    stage: &'static str,
    run: fn(&CaseInput) -> Result<usize, String>,
}

fn benchmark_tasks() -> Vec<Task> {
    vec![
        Task {
            name: "check",
            stage: "frontend",
            run: run_check,
        },
        Task {
            name: "mir-o0",
            stage: "mir",
            run: run_mir_o0,
        },
        Task {
            name: "kir-o3",
            stage: "kir-optimizer",
            run: run_kir_o3,
        },
        Task {
            name: "emit-c-o3",
            stage: "c-backend",
            run: run_emit_c_o3,
        },
        Task {
            name: "emit-wat-o3",
            stage: "wasm-backend",
            run: run_emit_wat_o3,
        },
        Task {
            name: "emit-wasm-o3",
            stage: "wasm-backend",
            run: run_emit_wasm_o3,
        },
        Task {
            name: "emit-llvm-o3",
            stage: "llvm-backend",
            run: run_emit_llvm_o3,
        },
    ]
}

fn measure(input: &CaseInput, task: &Task, config: &Config) -> Result<BenchmarkResult, String> {
    for _ in 0..config.warmup {
        black_box((task.run)(input)?);
    }

    let mut samples = Vec::with_capacity(config.iterations);
    let mut output_units = 0usize;
    for _ in 0..config.iterations {
        let start = Instant::now();
        output_units = black_box((task.run)(input)?);
        samples.push(start.elapsed().as_nanos());
    }
    samples.sort_unstable();

    Ok(BenchmarkResult {
        case_name: input.name.clone(),
        case_path: input.path.display().to_string(),
        task_name: task.name.to_string(),
        stage: task.stage.to_string(),
        iterations: config.iterations,
        warmup: config.warmup,
        min_ns: samples[0],
        median_ns: percentile(&samples, 50),
        p95_ns: percentile(&samples, 95),
        mean_ns: samples.iter().sum::<u128>() / samples.len() as u128,
        output_units,
    })
}

fn percentile(sorted: &[u128], percentile: usize) -> u128 {
    let index = ((sorted.len() * percentile).div_ceil(100)).saturating_sub(1);
    sorted[index.min(sorted.len() - 1)]
}

fn run_check(input: &CaseInput) -> Result<usize, String> {
    let checked = checked_program(input)?;
    Ok(checked.functions.len() + checked.structs.len())
}

fn run_mir_o0(input: &CaseInput) -> Result<usize, String> {
    let checked = checked_program(input)?;
    let module = lower_to_mir(&checked).map_err(|error| error.to_string())?;
    Ok(print_mir_module(&module).len())
}

fn run_kir_o3(input: &CaseInput) -> Result<usize, String> {
    let (_, result) = compile_kir(
        input,
        3,
        KirConsumer::NativeLibrary,
        OverflowMode::Unchecked,
        BoundsMode::Unchecked,
    )?;
    Ok(print_kir_module(verified_artifact(&result)?).len())
}

fn run_emit_c_o3(input: &CaseInput) -> Result<usize, String> {
    let (_, result) = compile_kir(
        input,
        3,
        KirConsumer::C,
        OverflowMode::Unchecked,
        BoundsMode::Unchecked,
    )?;
    Ok(emit_c_kir_module_with_contracts(
        verified_artifact(&result)?,
        result.contract_facts.as_ref(),
    )?
    .len())
}

fn run_emit_wat_o3(input: &CaseInput) -> Result<usize, String> {
    let (_, result) = compile_kir(
        input,
        3,
        KirConsumer::WebAssembly,
        OverflowMode::Unchecked,
        BoundsMode::Unchecked,
    )?;
    Ok(emit_wat_kir_module(
        verified_artifact(&result)?,
        EmitWasmOptions { opt_level: 3 },
    )?
    .len())
}

fn run_emit_wasm_o3(input: &CaseInput) -> Result<usize, String> {
    let (_, result) = compile_kir(
        input,
        3,
        KirConsumer::WebAssembly,
        OverflowMode::Unchecked,
        BoundsMode::Unchecked,
    )?;
    Ok(emit_wasm_kir_module(
        verified_artifact(&result)?,
        EmitWasmOptions { opt_level: 3 },
    )?
    .len())
}

#[cfg(feature = "native-toolchain")]
fn run_emit_llvm_o3(input: &CaseInput) -> Result<usize, String> {
    let (_, result) = compile_kir(
        input,
        3,
        KirConsumer::NativeLibrary,
        OverflowMode::Unchecked,
        BoundsMode::Unchecked,
    )?;
    let context = NativeContext::new().map_err(|error| error.to_string())?;
    let target = NativeTarget::host().map_err(|error| error.to_string())?;
    let text = lower_native_kir_module(
        &context,
        &target,
        &result,
        &EmitLlvmOptions {
            source_file_name: Some(input.path.display().to_string()),
            target_triple: None,
        },
    )
    .map_err(|error| error.to_string())?
    .verify()
    .map_err(|error| error.to_string())?
    .audit()
    .map_err(|error| error.to_string())?
    .optimize(&target, NativeOptimizationLevel::O3)
    .map_err(|error| error.to_string())?
    .to_ir_string()
    .map_err(|error| error.to_string())?;
    Ok(text.len())
}

#[cfg(not(feature = "native-toolchain"))]
fn run_emit_llvm_o3(_input: &CaseInput) -> Result<usize, String> {
    Err("emit-llvm-o3 requires --features native-toolchain".to_string())
}

fn checked_program(input: &CaseInput) -> Result<calckernel::CheckedProgram, String> {
    let source = SourceFile::new(input.path.display().to_string(), input.source_text.clone());
    let checked = check(&source);
    if !checked.diagnostics.is_empty() {
        return Err(format_diagnostics(&source, &checked.diagnostics));
    }
    Ok(checked.checked_program)
}

fn compile_kir(
    input: &CaseInput,
    opt_level: u8,
    consumer: KirConsumer,
    overflow_mode: OverflowMode,
    bounds_mode: BoundsMode,
) -> Result<(calckernel::CheckedProgram, KirPassManagerResult), String> {
    let checked = checked_program(input)?;
    let mir = lower_to_mir(&checked).map_err(|error| error.to_string())?;
    let kir = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer,
            overflow_mode: match overflow_mode {
                OverflowMode::Unchecked => KirOverflowMode::Unchecked,
                OverflowMode::Checked => KirOverflowMode::Checked,
            },
            bounds_mode: match bounds_mode {
                BoundsMode::Unchecked => KirBoundsMode::Unchecked,
                BoundsMode::Checked => KirBoundsMode::Checked,
            },
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .map_err(|error| error.to_string())?;
    let contracts = import_contract_facts(&kir, &checked, 0).map_err(|error| error.to_string())?;
    let level = match opt_level {
        0 => KirOptimizationLevel::O0,
        1 => KirOptimizationLevel::O1,
        2 => KirOptimizationLevel::O2,
        3 => KirOptimizationLevel::O3,
        _ => return Err("optimization level is outside 0..=3".to_string()),
    };
    let result = run_kir_pass_pipeline(kir, level, Some(&contracts));
    if !result.errors.is_empty() {
        return Err(format!(
            "KIR verification failed: {}",
            result.errors.join("; ")
        ));
    }
    verified_artifact(&result)?;
    Ok((checked, result))
}

fn verified_artifact(result: &KirPassManagerResult) -> Result<&calckernel::KirModule, String> {
    result
        .artifact
        .as_ref()
        .ok_or_else(|| "KIR pipeline did not produce a verified artifact".to_string())
}

#[cfg(feature = "native-toolchain")]
fn measure_optimizer_comparisons(
    repo_root: &Path,
    cases: &[Case],
    config: &Config,
    baseline: &CompilerBaseline,
) -> Result<Vec<OptimizerComparison>, String> {
    let mut comparisons = Vec::new();
    for case in cases {
        let input = CaseInput::load(repo_root, case)?;
        for _ in 0..config.warmup {
            black_box(time_kir_optimization(&input)?);
        }
        let mut samples = Vec::with_capacity(config.iterations);
        for _ in 0..config.iterations {
            samples.push(time_kir_optimization(&input)?);
        }
        let median_ns = median(&samples);
        comparisons.push(OptimizerComparison {
            case_name: case.name.clone(),
            kir_median_ns: median_ns,
            v0_10_mir_median_ns: baseline.optimizer_median(&case.name)?,
        });
    }
    Ok(comparisons)
}

#[cfg(feature = "native-toolchain")]
fn time_kir_optimization(input: &CaseInput) -> Result<u128, String> {
    let checked = checked_program(input)?;
    let mir = lower_to_mir(&checked).map_err(|error| error.to_string())?;
    let kir = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::NativeLibrary,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .map_err(|error| error.to_string())?;
    let contracts = import_contract_facts(&kir, &checked, 0).map_err(|error| error.to_string())?;
    let start = Instant::now();
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O3, Some(&contracts));
    let elapsed = start.elapsed().as_nanos().max(1);
    if !result.errors.is_empty() || result.artifact.is_none() {
        return Err(format!(
            "KIR optimization timing failed verification: {}",
            result.errors.join("; ")
        ));
    }
    Ok(elapsed)
}

#[cfg(feature = "native-toolchain")]
fn measure_native_runtime(
    repo_root: &Path,
    config: &Config,
    baseline: &CompilerBaseline,
    compiler_cases: &[Case],
) -> Result<NativeRuntimeReport, String> {
    let clang = clang_oracle()?;
    let clang_version = clang_version(&clang)?;
    let fixture_root = repo_root.join(NATIVE_RUNTIME_FIXTURE_ROOT);
    let mut fixtures = fs::read_dir(&fixture_root)
        .map_err(|error| format!("failed to read {}: {error}", fixture_root.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to enumerate native performance fixtures: {error}"))?;
    fixtures.retain(|path| path.extension().and_then(|value| value.to_str()) == Some("ck"));
    fixtures.sort();
    if fixtures.len() < 3 {
        return Err(
            "native runtime performance suite requires at least three CK kernels".to_string(),
        );
    }

    let root = runtime_temp_dir()?;
    fs::create_dir(&root)
        .map_err(|error| format!("failed to create {}: {error}", root.display()))?;
    let result = (|| {
        let mut suites = Vec::new();
        for checked in [false, true] {
            let mut cases = Vec::new();
            for fixture in &fixtures {
                cases.push(measure_native_case(
                    fixture, &root, &clang, checked, config, baseline,
                )?);
            }
            suites.push(NativeRuntimeSuite {
                mode: if checked { "checked" } else { "unchecked" },
                cases,
            });
        }
        Ok(NativeRuntimeReport {
            cpu_policy: config.cpu_policy,
            clang_version,
            warmup: config.warmup,
            suites,
            baseline: baseline.clone(),
            optimizer: measure_optimizer_comparisons(repo_root, compiler_cases, config, baseline)?,
        })
    })();
    let _ = fs::remove_dir_all(&root);
    result
}

#[cfg(feature = "native-toolchain")]
fn measure_native_case(
    fixture: &Path,
    root: &Path,
    clang: &Path,
    checked: bool,
    config: &Config,
    baseline: &CompilerBaseline,
) -> Result<NativeRuntimeCase, String> {
    let name = fixture
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            format!(
                "performance fixture name is not UTF-8: {}",
                fixture.display()
            )
        })?;
    let source_text = fs::read_to_string(fixture)
        .map_err(|error| format!("failed to read {}: {error}", fixture.display()))?;
    let input = CaseInput {
        name: name.to_string(),
        path: fixture.to_path_buf(),
        source_text,
    };
    let overflow_mode = if checked {
        OverflowMode::Checked
    } else {
        OverflowMode::Unchecked
    };
    let bounds_mode = if checked {
        BoundsMode::Checked
    } else {
        BoundsMode::Unchecked
    };
    let suffix = if checked { "checked" } else { "unchecked" };
    let batch_iterations = if config.iterations <= 5 {
        200_000
    } else {
        20_000_000
    };
    let seed = 17i64;
    let proof_values = (name == "proof_loop").then(|| {
        (0..batch_iterations)
            .map(|index| index % 4_093 - 2_046)
            .collect::<Vec<_>>()
    });

    let native_compile_start = Instant::now();
    let (_, native_kir) = compile_kir(
        &input,
        3,
        KirConsumer::NativeLibrary,
        overflow_mode,
        bounds_mode,
    )?;
    let context = NativeContext::new().map_err(|error| error.to_string())?;
    let target = NativeTarget::host_with_cpu(match config.cpu_policy {
        CpuPolicy::Baseline => NativeCpu::Baseline,
        CpuPolicy::Native => NativeCpu::Native,
    })
    .map_err(|error| error.to_string())?;
    let object = target
        .emit_object(
            lower_native_kir_module(&context, &target, &native_kir, &EmitLlvmOptions::default())
                .map_err(|error| error.to_string())?
                .verify()
                .map_err(|error| error.to_string())?
                .audit()
                .map_err(|error| error.to_string())?
                .optimize(&target, NativeOptimizationLevel::O3)
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
    let native = link_native_dynamic_library(&object, &["kernel".to_string()])
        .map_err(|error| error.to_string())?;
    let native_compile_ns = native_compile_start.elapsed().as_nanos();
    let native_path = root.join(format!("{name}-{suffix}-native{}", dynamic_suffix()));
    fs::write(&native_path, native.as_bytes())
        .map_err(|error| format!("failed to write {}: {error}", native_path.display()))?;

    let clang_compile_start = Instant::now();
    let (_, c_kir) = compile_kir(&input, 3, KirConsumer::C, overflow_mode, bounds_mode)?;
    let c_source = emit_c_kir_module_with_contracts(
        verified_artifact(&c_kir)?,
        c_kir.contract_facts.as_ref(),
    )?;
    let c_path = root.join(format!("{name}-{suffix}.c"));
    let clang_path = root.join(format!("{name}-{suffix}-clang{}", dynamic_suffix()));
    fs::write(&c_path, c_source)
        .map_err(|error| format!("failed to write {}: {error}", c_path.display()))?;
    compile_strict_clang_library(clang, &c_path, &clang_path, config.cpu_policy)?;
    let clang_c_compile_ns = clang_compile_start.elapsed().as_nanos();

    let native_artifact_bytes = native.as_bytes().len();
    let clang_c_artifact_bytes = usize::try_from(
        fs::metadata(&clang_path)
            .map_err(|error| format!("failed to inspect {}: {error}", clang_path.display()))?
            .len(),
    )
    .map_err(|_| "Clang artifact length exceeds usize".to_string())?;

    let native_cold_start = Instant::now();
    let native_library = DynamicLibrary::open(&native_path)?;
    let native_result = unsafe {
        call_kernel(
            &native_library,
            checked,
            batch_iterations,
            seed,
            proof_values.as_deref(),
        )?
    };
    let native_cold_ns = native_cold_start.elapsed().as_nanos();

    let clang_c_cold_start = Instant::now();
    let clang_library = DynamicLibrary::open(&clang_path)?;
    let clang_result = unsafe {
        call_kernel(
            &clang_library,
            checked,
            batch_iterations,
            seed,
            proof_values.as_deref(),
        )?
    };
    let clang_c_cold_ns = clang_c_cold_start.elapsed().as_nanos();
    let reference_equivalent = native_result == clang_result;
    if !reference_equivalent {
        return Err(format!(
            "{name}/{suffix} result mismatch: native={native_result}, clang={clang_result}"
        ));
    }

    for _ in 0..config.warmup {
        black_box(unsafe {
            call_kernel(
                &native_library,
                checked,
                batch_iterations,
                seed,
                proof_values.as_deref(),
            )?
        });
        black_box(unsafe {
            call_kernel(
                &clang_library,
                checked,
                batch_iterations,
                seed,
                proof_values.as_deref(),
            )?
        });
    }
    let mut native_samples_ns = Vec::with_capacity(config.iterations);
    let mut clang_c_samples_ns = Vec::with_capacity(config.iterations);
    for index in 0..config.iterations {
        if index % 2 == 0 {
            native_samples_ns.push(measure_kernel_sample(
                &native_library,
                checked,
                batch_iterations,
                seed,
                native_result,
                proof_values.as_deref(),
            )?);
            clang_c_samples_ns.push(measure_kernel_sample(
                &clang_library,
                checked,
                batch_iterations,
                seed,
                clang_result,
                proof_values.as_deref(),
            )?);
        } else {
            clang_c_samples_ns.push(measure_kernel_sample(
                &clang_library,
                checked,
                batch_iterations,
                seed,
                clang_result,
                proof_values.as_deref(),
            )?);
            native_samples_ns.push(measure_kernel_sample(
                &native_library,
                checked,
                batch_iterations,
                seed,
                native_result,
                proof_values.as_deref(),
            )?);
        }
    }
    let native_median_ns = median(&native_samples_ns);
    let clang_c_median_ns = median(&clang_c_samples_ns);

    Ok(NativeRuntimeCase {
        name: name.to_string(),
        reference_equivalent,
        native_compile_ns,
        clang_c_compile_ns,
        native_cold_ns,
        clang_c_cold_ns,
        native_samples_ns,
        clang_c_samples_ns,
        native_median_ns,
        clang_c_median_ns,
        peak_memory_bytes: peak_memory_bytes(),
        native_artifact_bytes,
        clang_c_artifact_bytes,
        batch_iterations,
        result: native_result,
        v0_10_median_ns: baseline.runtime_median(
            config.cpu_policy,
            if checked { "checked" } else { "unchecked" },
            name,
        )?,
        proof_loop: name == "proof_loop",
    })
}

#[cfg(feature = "native-toolchain")]
fn measure_kernel_call(
    library: &DynamicLibrary,
    checked: bool,
    iterations: i64,
    seed: i64,
    expected: i64,
    proof_values: Option<&[i64]>,
) -> Result<u128, String> {
    let start = Instant::now();
    let actual = unsafe {
        call_kernel(
            library,
            checked,
            black_box(iterations),
            black_box(seed),
            proof_values,
        )?
    };
    let elapsed = start.elapsed().as_nanos().max(1);
    if black_box(actual) != expected {
        return Err(format!(
            "warm kernel result changed: {actual} != {expected}"
        ));
    }
    Ok(elapsed)
}

#[cfg(feature = "native-toolchain")]
fn measure_kernel_sample(
    library: &DynamicLibrary,
    checked: bool,
    iterations: i64,
    seed: i64,
    expected: i64,
    proof_values: Option<&[i64]>,
) -> Result<u128, String> {
    let mut minimum = u128::MAX;
    for _ in 0..SAMPLE_REPETITIONS {
        minimum = minimum.min(measure_kernel_call(
            library,
            checked,
            iterations,
            seed,
            expected,
            proof_values,
        )?);
    }
    Ok(minimum)
}

#[cfg(feature = "native-toolchain")]
unsafe fn call_kernel(
    library: &DynamicLibrary,
    checked: bool,
    iterations: i64,
    seed: i64,
    proof_values: Option<&[i64]>,
) -> Result<i64, String> {
    if let Some(values) = proof_values {
        let len = u32::try_from(values.len())
            .map_err(|_| "proof-loop input exceeds the CK slice length limit".to_string())?;
        let data = values.as_ptr().cast_mut();
        if checked {
            type Kernel = unsafe extern "C" fn(*mut i64, u32, i64, *mut i64) -> i32;
            let kernel: Kernel = unsafe { library.symbol("kernel")? };
            let mut result = 0i64;
            let status = unsafe { kernel(data, len, seed, &mut result) };
            if status != 0 {
                return Err(format!(
                    "checked proof-loop performance kernel returned status {status}"
                ));
            }
            return Ok(result);
        }
        type Kernel = unsafe extern "C" fn(*mut i64, u32, i64) -> i64;
        let kernel: Kernel = unsafe { library.symbol("kernel")? };
        return Ok(unsafe { kernel(data, len, seed) });
    }
    if checked {
        type Kernel = unsafe extern "C" fn(i64, i64, *mut i64) -> i32;
        let kernel: Kernel = unsafe { library.symbol("kernel")? };
        let mut result = 0i64;
        let status = unsafe { kernel(iterations, seed, &mut result) };
        if status != 0 {
            return Err(format!(
                "checked performance kernel returned status {status}"
            ));
        }
        Ok(result)
    } else {
        type Kernel = unsafe extern "C" fn(i64, i64) -> i64;
        let kernel: Kernel = unsafe { library.symbol("kernel")? };
        Ok(unsafe { kernel(iterations, seed) })
    }
}

#[cfg(feature = "native-toolchain")]
fn compile_strict_clang_library(
    clang: &Path,
    source: &Path,
    output: &Path,
    cpu_policy: CpuPolicy,
) -> Result<(), String> {
    let mut command = Command::new(clang);
    command.args([
        "-std=c11",
        "-O3",
        "-fno-fast-math",
        "-ffp-contract=off",
        "-fno-unwind-tables",
        "-fno-asynchronous-unwind-tables",
        "-falign-functions=64",
        "-fuse-ld=lld",
        "-nostdlib",
    ]);
    match cpu_policy {
        CpuPolicy::Baseline => {
            command.arg("-mcpu=generic");
        }
        CpuPolicy::Native => {
            command.arg("-march=native");
        }
    }
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
    let output_result = command
        .arg(source)
        .arg("-o")
        .arg(output)
        .output()
        .map_err(|error| format!("failed to execute pinned Clang oracle: {error}"))?;
    if !output_result.status.success() {
        return Err(format!(
            "strict Clang C O3 failed for {}: {}",
            source.display(),
            String::from_utf8_lossy(&output_result.stderr)
        ));
    }
    Ok(())
}

#[cfg(feature = "native-toolchain")]
fn clang_oracle() -> Result<PathBuf, String> {
    let path = env::var_os("CKC_CLANG_ORACLE")
        .map(PathBuf::from)
        .ok_or_else(|| "native runtime benchmark requires CKC_CLANG_ORACLE".to_string())?;
    if !path.is_file() {
        return Err(format!(
            "CKC_CLANG_ORACLE is not a file: {}",
            path.display()
        ));
    }
    Ok(path)
}

#[cfg(feature = "native-toolchain")]
fn clang_version(clang: &Path) -> Result<String, String> {
    let output = Command::new(clang)
        .arg("--version")
        .output()
        .map_err(|error| format!("failed to inspect pinned Clang oracle: {error}"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() || !text.contains("22.1.8") {
        return Err(format!(
            "CKC_CLANG_ORACLE must report pinned Clang 22.1.8, got {text:?}"
        ));
    }
    Ok("22.1.8".to_string())
}

#[cfg(feature = "native-toolchain")]
fn runtime_temp_dir() -> Result<PathBuf, String> {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock error: {error}"))?
        .as_nanos();
    Ok(env::temp_dir().join(format!("ckc-native-perf-{}-{unique}", process::id())))
}

#[cfg(feature = "native-toolchain")]
fn median(samples: &[u128]) -> u128 {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    sorted[sorted.len() / 2]
}

#[cfg(feature = "native-toolchain")]
const fn dynamic_suffix() -> &'static str {
    if cfg!(target_os = "windows") {
        ".dll"
    } else if cfg!(target_os = "macos") {
        ".dylib"
    } else {
        ".so"
    }
}

#[cfg(feature = "native-toolchain")]
struct NativeRuntimeReport {
    cpu_policy: CpuPolicy,
    clang_version: String,
    warmup: usize,
    suites: Vec<NativeRuntimeSuite>,
    baseline: CompilerBaseline,
    optimizer: Vec<OptimizerComparison>,
}

#[cfg(feature = "native-toolchain")]
struct NativeRuntimeSuite {
    mode: &'static str,
    cases: Vec<NativeRuntimeCase>,
}

#[cfg(feature = "native-toolchain")]
struct NativeRuntimeCase {
    name: String,
    reference_equivalent: bool,
    native_compile_ns: u128,
    clang_c_compile_ns: u128,
    native_cold_ns: u128,
    clang_c_cold_ns: u128,
    native_samples_ns: Vec<u128>,
    clang_c_samples_ns: Vec<u128>,
    native_median_ns: u128,
    clang_c_median_ns: u128,
    peak_memory_bytes: u64,
    native_artifact_bytes: usize,
    clang_c_artifact_bytes: usize,
    batch_iterations: i64,
    result: i64,
    v0_10_median_ns: u128,
    proof_loop: bool,
}

#[cfg(feature = "native-toolchain")]
struct OptimizerComparison {
    case_name: String,
    kir_median_ns: u128,
    v0_10_mir_median_ns: u128,
}

#[cfg(feature = "native-toolchain")]
impl NativeRuntimeReport {
    fn to_json(&self) -> String {
        let mut output = String::new();
        output.push_str("{\n  \"schemaVersion\": 4,\n");
        output.push_str(&format!(
            "  \"cpuPolicy\": \"{}\",\n",
            match self.cpu_policy {
                CpuPolicy::Baseline => "baseline",
                CpuPolicy::Native => "native",
            }
        ));
        output.push_str("  \"fastMath\": false,\n");
        let source_digests = self
            .baseline
            .source_digests
            .iter()
            .map(|(name, digest)| format!("\"{}\":\"{}\"", json_escape(name), json_escape(digest)))
            .collect::<Vec<_>>()
            .join(",");
        output.push_str(&format!(
            "  \"clangVersion\": \"{}\",\n  \"warmup\": {},\n  \"sampleRepetitions\": {},\n  \"baselineV010\": {{ \"commit\": \"{}\", \"compilerIdentity\": \"{}\", \"llvmVersion\": \"{}\", \"target\": \"{}\", \"harness\": \"{}\", \"statistics\": \"{}\", \"sourceDigestCount\": {}, \"sourceDigests\": {{{}}} }},\n  \"suites\": [\n",
            json_escape(&self.clang_version),
            self.warmup,
            SAMPLE_REPETITIONS,
            json_escape(&self.baseline.commit),
            json_escape(&self.baseline.compiler_identity),
            json_escape(&self.baseline.llvm_version),
            json_escape(&host_target_name()),
            json_escape(&self.baseline.harness),
            json_escape(&self.baseline.statistics),
            self.baseline.source_digest_count,
            source_digests,
        ));
        for (suite_index, suite) in self.suites.iter().enumerate() {
            output.push_str(&format!(
                "    {{ \"mode\": \"{}\", \"cases\": [\n",
                suite.mode
            ));
            for (case_index, case) in suite.cases.iter().enumerate() {
                output.push_str(&case.to_json());
                if case_index + 1 < suite.cases.len() {
                    output.push(',');
                }
                output.push('\n');
            }
            output.push_str("    ] }");
            if suite_index + 1 < self.suites.len() {
                output.push(',');
            }
            output.push('\n');
        }
        output.push_str("  ],\n  \"optimizerComparisons\": [\n");
        for (index, comparison) in self.optimizer.iter().enumerate() {
            output.push_str(&format!(
                "    {{ \"case\": \"{}\", \"kirMedianNs\": {}, \"v010MirMedianNs\": {} }}",
                json_escape(&comparison.case_name),
                comparison.kir_median_ns,
                comparison.v0_10_mir_median_ns,
            ));
            if index + 1 < self.optimizer.len() {
                output.push(',');
            }
            output.push('\n');
        }
        output.push_str("  ]\n}\n");
        output
    }
}

#[cfg(feature = "native-toolchain")]
impl NativeRuntimeCase {
    fn to_json(&self) -> String {
        format!(
            "      {{ \"name\": \"{}\", \"referenceEquivalent\": {}, \"nativeCompileNs\": {}, \"clangCCompileNs\": {}, \"nativeColdNs\": {}, \"clangCColdNs\": {}, \"nativeMedianNs\": {}, \"clangCMedianNs\": {}, \"v010MedianNs\": {}, \"proofLoop\": {}, \"nativeSamplesNs\": {}, \"clangCSamplesNs\": {}, \"peakMemoryBytes\": {}, \"nativeArtifactBytes\": {}, \"clangCArtifactBytes\": {}, \"batchIterations\": {}, \"result\": {} }}",
            json_escape(&self.name),
            self.reference_equivalent,
            self.native_compile_ns,
            self.clang_c_compile_ns,
            self.native_cold_ns,
            self.clang_c_cold_ns,
            self.native_median_ns,
            self.clang_c_median_ns,
            self.v0_10_median_ns,
            self.proof_loop,
            json_u128_array(&self.native_samples_ns),
            json_u128_array(&self.clang_c_samples_ns),
            self.peak_memory_bytes,
            self.native_artifact_bytes,
            self.clang_c_artifact_bytes,
            self.batch_iterations,
            self.result,
        )
    }
}

#[cfg(feature = "native-toolchain")]
fn json_u128_array(values: &[u128]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(u128::to_string)
            .collect::<Vec<_>>()
            .join(",")
    )
}

#[cfg(feature = "native-toolchain")]
struct DynamicLibrary {
    handle: *mut std::ffi::c_void,
}

#[cfg(feature = "native-toolchain")]
impl DynamicLibrary {
    fn open(path: &Path) -> Result<Self, String> {
        platform_loader::open(path)
    }

    unsafe fn symbol<T: Copy>(&self, name: &str) -> Result<T, String> {
        let address = platform_loader::symbol(self.handle, name)?;
        if std::mem::size_of::<T>() != std::mem::size_of_val(&address) {
            return Err("loaded symbol has an incompatible pointer size".to_string());
        }
        Ok(unsafe {
            // SAFETY: The caller selects the C ABI function type matching the
            // checked Native C ABI fixture signature.
            std::mem::transmute_copy(&address)
        })
    }
}

#[cfg(feature = "native-toolchain")]
impl Drop for DynamicLibrary {
    fn drop(&mut self) {
        platform_loader::close(self.handle);
    }
}

#[cfg(all(feature = "native-toolchain", unix))]
mod platform_loader {
    use std::{ffi::CString, path::Path};

    pub(super) fn open(path: &Path) -> Result<super::DynamicLibrary, String> {
        let path_text = path.to_string_lossy();
        let path = CString::new(path_text.as_bytes())
            .map_err(|_| "dynamic-library path contains NUL".to_string())?;
        let handle = unsafe {
            // SAFETY: The path is NUL-terminated and live for this call.
            dlopen(path.as_ptr(), 2)
        };
        if handle.is_null() {
            return Err(format!("dlopen failed for {path_text}"));
        }
        Ok(super::DynamicLibrary { handle })
    }

    pub(super) fn symbol(
        handle: *mut std::ffi::c_void,
        name: &str,
    ) -> Result<*mut std::ffi::c_void, String> {
        let name = CString::new(name).map_err(|_| "symbol name contains NUL".to_string())?;
        let address = unsafe {
            // SAFETY: The library handle is live and the name is
            // NUL-terminated for this lookup.
            dlsym(handle, name.as_ptr())
        };
        if address.is_null() {
            Err(format!(
                "missing performance symbol {}",
                name.to_string_lossy()
            ))
        } else {
            Ok(address)
        }
    }

    pub(super) fn close(handle: *mut std::ffi::c_void) {
        let _ = unsafe {
            // SAFETY: DynamicLibrary owns the live handle and closes it once.
            dlclose(handle)
        };
    }

    unsafe extern "C" {
        fn dlopen(path: *const std::ffi::c_char, mode: std::ffi::c_int) -> *mut std::ffi::c_void;
        fn dlsym(
            handle: *mut std::ffi::c_void,
            name: *const std::ffi::c_char,
        ) -> *mut std::ffi::c_void;
        fn dlclose(handle: *mut std::ffi::c_void) -> std::ffi::c_int;
    }
}

#[cfg(all(feature = "native-toolchain", windows))]
mod platform_loader {
    use std::{ffi::CString, os::windows::ffi::OsStrExt, path::Path};

    pub(super) fn open(path: &Path) -> Result<super::DynamicLibrary, String> {
        let wide = path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let handle = unsafe {
            // SAFETY: The path is NUL-terminated and live for this call.
            LoadLibraryW(wide.as_ptr())
        };
        if handle.is_null() {
            return Err(format!("LoadLibraryW failed for {}", path.display()));
        }
        Ok(super::DynamicLibrary { handle })
    }

    pub(super) fn symbol(
        handle: *mut std::ffi::c_void,
        name: &str,
    ) -> Result<*mut std::ffi::c_void, String> {
        let name = CString::new(name).map_err(|_| "symbol name contains NUL".to_string())?;
        let address = unsafe {
            // SAFETY: The library handle is live and the name is
            // NUL-terminated for this lookup.
            GetProcAddress(handle, name.as_ptr())
        };
        if address.is_null() {
            Err(format!(
                "missing performance symbol {}",
                name.to_string_lossy()
            ))
        } else {
            Ok(address)
        }
    }

    pub(super) fn close(handle: *mut std::ffi::c_void) {
        let _ = unsafe {
            // SAFETY: DynamicLibrary owns the live handle and closes it once.
            FreeLibrary(handle)
        };
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryW(path: *const u16) -> *mut std::ffi::c_void;
        fn GetProcAddress(handle: *mut std::ffi::c_void, name: *const i8) -> *mut std::ffi::c_void;
        fn FreeLibrary(handle: *mut std::ffi::c_void) -> i32;
    }
}

#[cfg(all(feature = "native-toolchain", unix))]
fn peak_memory_bytes() -> u64 {
    #[repr(C)]
    struct TimeValue {
        seconds: isize,
        microseconds: isize,
    }
    #[repr(C)]
    struct ResourceUsage {
        user: TimeValue,
        system: TimeValue,
        max_rss: isize,
        remaining: [isize; 13],
    }
    let mut usage = ResourceUsage {
        user: TimeValue {
            seconds: 0,
            microseconds: 0,
        },
        system: TimeValue {
            seconds: 0,
            microseconds: 0,
        },
        max_rss: 0,
        remaining: [0; 13],
    };
    let status = unsafe {
        // SAFETY: `usage` matches the 64-bit release-host rusage layout and is
        // writable for the synchronous query.
        getrusage(0, (&mut usage as *mut ResourceUsage).cast())
    };
    if status != 0 || usage.max_rss <= 0 {
        return 1;
    }
    let value = usage.max_rss as u64;
    if cfg!(target_os = "macos") {
        value
    } else {
        value.saturating_mul(1024)
    }
}

#[cfg(all(feature = "native-toolchain", unix))]
unsafe extern "C" {
    fn getrusage(who: i32, usage: *mut core::ffi::c_void) -> i32;
}

#[cfg(all(feature = "native-toolchain", windows))]
fn peak_memory_bytes() -> u64 {
    #[repr(C)]
    struct ProcessMemoryCounters {
        size: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }
    let mut counters = ProcessMemoryCounters {
        size: std::mem::size_of::<ProcessMemoryCounters>() as u32,
        page_fault_count: 0,
        peak_working_set_size: 0,
        working_set_size: 0,
        quota_peak_paged_pool_usage: 0,
        quota_paged_pool_usage: 0,
        quota_peak_non_paged_pool_usage: 0,
        quota_non_paged_pool_usage: 0,
        pagefile_usage: 0,
        peak_pagefile_usage: 0,
    };
    let status = unsafe {
        // SAFETY: The process pseudo-handle is valid and `counters` is
        // writable for its declared size.
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&mut counters as *mut ProcessMemoryCounters).cast(),
            counters.size,
        )
    };
    if status == 0 {
        1
    } else {
        counters.peak_working_set_size as u64
    }
}

#[cfg(all(feature = "native-toolchain", windows))]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcess() -> *mut core::ffi::c_void;
}

#[cfg(all(feature = "native-toolchain", windows))]
#[link(name = "psapi")]
unsafe extern "system" {
    fn GetProcessMemoryInfo(
        process: *mut core::ffi::c_void,
        counters: *mut core::ffi::c_void,
        size: u32,
    ) -> i32;
}

#[derive(Debug)]
struct Summary {
    metadata: SummaryMetadata,
    results: Vec<BenchmarkResult>,
}

impl Summary {
    fn to_json(&self) -> String {
        let mut out = String::new();
        out.push_str("{\n");
        out.push_str("  \"schemaVersion\": 1,\n");
        out.push_str("  \"command\": \"cargo bench --bench ckc_perf\",\n");
        out.push_str(&format!(
            "  \"generatedAtUnixSeconds\": {},\n",
            self.metadata.generated_at_unix_seconds
        ));
        out.push_str(&format!(
            "  \"target\": \"{}-{}\",\n",
            json_escape(self.metadata.os),
            json_escape(self.metadata.arch)
        ));
        out.push_str(&format!(
            "  \"iterations\": {},\n",
            self.metadata.iterations
        ));
        out.push_str(&format!("  \"warmup\": {},\n", self.metadata.warmup));
        out.push_str("  \"results\": [\n");
        for (index, result) in self.results.iter().enumerate() {
            out.push_str(&result.to_json());
            if index + 1 < self.results.len() {
                out.push(',');
            }
            out.push('\n');
        }
        out.push_str("  ]\n");
        out.push_str("}\n");
        out
    }

    fn to_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str("# CKC Native Performance Summary\n\n");
        out.push_str(&format!(
            "- Command: `cargo bench --bench ckc_perf`\n- Target: `{}-{}`\n- Iterations: `{}`\n- Warmup: `{}`\n\n",
            self.metadata.os, self.metadata.arch, self.metadata.iterations, self.metadata.warmup
        ));
        out.push_str(
            "| Case | Task | Stage | Median ms | P95 ms | Min ms | Mean ms | Output units |\n",
        );
        out.push_str("| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |\n");
        for result in &self.results {
            out.push_str(&format!(
                "| `{}` | `{}` | `{}` | {:.3} | {:.3} | {:.3} | {:.3} | {} |\n",
                result.case_name,
                result.task_name,
                result.stage,
                nanos_to_millis(result.median_ns),
                nanos_to_millis(result.p95_ns),
                nanos_to_millis(result.min_ns),
                nanos_to_millis(result.mean_ns),
                result.output_units
            ));
        }
        out
    }
}

#[derive(Debug)]
struct SummaryMetadata {
    generated_at_unix_seconds: u64,
    os: &'static str,
    arch: &'static str,
    iterations: usize,
    warmup: usize,
}

impl SummaryMetadata {
    fn new(iterations: usize, warmup: usize) -> Self {
        Self {
            generated_at_unix_seconds: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |duration| duration.as_secs()),
            os: env::consts::OS,
            arch: env::consts::ARCH,
            iterations,
            warmup,
        }
    }
}

#[derive(Debug)]
struct BenchmarkResult {
    case_name: String,
    case_path: String,
    task_name: String,
    stage: String,
    iterations: usize,
    warmup: usize,
    min_ns: u128,
    median_ns: u128,
    p95_ns: u128,
    mean_ns: u128,
    output_units: usize,
}

impl BenchmarkResult {
    fn to_json(&self) -> String {
        format!(
            "    {{ \"case\": \"{}\", \"path\": \"{}\", \"task\": \"{}\", \"stage\": \"{}\", \"iterations\": {}, \"warmup\": {}, \"minNs\": {}, \"medianNs\": {}, \"p95Ns\": {}, \"meanNs\": {}, \"outputUnits\": {} }}",
            json_escape(&self.case_name),
            json_escape(&self.case_path),
            json_escape(&self.task_name),
            json_escape(&self.stage),
            self.iterations,
            self.warmup,
            self.min_ns,
            self.median_ns,
            self.p95_ns,
            self.mean_ns,
            self.output_units
        )
    }
}

fn nanos_to_millis(nanos: u128) -> f64 {
    nanos as f64 / 1_000_000.0
}

fn json_escape(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

fn relative_to_repo(repo_root: &Path, path: &Path) -> String {
    path.strip_prefix(repo_root)
        .unwrap_or(path)
        .display()
        .to_string()
}
