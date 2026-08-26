use std::{
    env, fs,
    hint::black_box,
    path::{Path, PathBuf},
    process,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use calckernel::{
    EmitCOptions, EmitLlvmOptions, EmitWasmOptions, MirModule, MirPassBoundsMode, MirPassContext,
    MirPassOverflowMode, MirPassTargetBackend, OverflowMode, SourceFile,
    build_mir_optimization_pipeline, check, emit_c_module, emit_llvm_module,
    emit_wasm_module_with_options, emit_wat_module_with_options, format_diagnostics, lower_to_mir,
    print_mir_module, run_mir_pass_pipeline,
};

const USAGE: &str = "cargo bench --bench ckc_perf -- [--quick] [--case <name>] [--task <name>] [--iterations <n>] [--warmup <n>] [--out-dir <path>]\n\nDefault outputs: build/perf/latest.summary.json and build/perf/latest.summary.md";
const CASE_MANIFEST: &str = "bench/perf/cases/native-cases.tsv";
const FIXTURE_ROOT: &str = "bench/perf/fixtures";
const DEFAULT_OUT_DIR: &str = "build/perf";

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
                other => return Err(format!("unknown argument `{other}`\n\n{USAGE}")),
            }
        }

        if config.iterations == 0 {
            return Err("--iterations must be greater than 0".to_string());
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
            name: "mir-o3",
            stage: "optimizer",
            run: run_mir_o3,
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
    let module = optimized_module(input, 0, MirPassTargetBackend::Mir)?;
    Ok(print_mir_module(&module).len())
}

fn run_mir_o3(input: &CaseInput) -> Result<usize, String> {
    let module = optimized_module(input, 3, MirPassTargetBackend::Mir)?;
    Ok(print_mir_module(&module).len())
}

fn run_emit_c_o3(input: &CaseInput) -> Result<usize, String> {
    let module = optimized_module(input, 3, MirPassTargetBackend::C)?;
    Ok(emit_c_module(
        &module,
        EmitCOptions {
            overflow_mode: OverflowMode::Unchecked,
            bounds_mode: calckernel::BoundsMode::Unchecked,
            opt_level: 3,
        },
    )
    .len())
}

fn run_emit_wat_o3(input: &CaseInput) -> Result<usize, String> {
    let module = optimized_module(input, 3, MirPassTargetBackend::Wasm)?;
    Ok(emit_wat_module_with_options(&module, EmitWasmOptions { opt_level: 3 }).len())
}

fn run_emit_wasm_o3(input: &CaseInput) -> Result<usize, String> {
    let module = optimized_module(input, 3, MirPassTargetBackend::Wasm)?;
    Ok(emit_wasm_module_with_options(&module, EmitWasmOptions { opt_level: 3 })?.len())
}

fn run_emit_llvm_o3(input: &CaseInput) -> Result<usize, String> {
    let module = optimized_module(input, 3, MirPassTargetBackend::Llvm)?;
    Ok(emit_llvm_module(
        &module,
        &EmitLlvmOptions {
            source_file_name: Some(input.path.display().to_string()),
            target_triple: None,
        },
    )
    .len())
}

fn checked_program(input: &CaseInput) -> Result<calckernel::CheckedProgram, String> {
    let source = SourceFile::new(input.path.display().to_string(), input.source_text.clone());
    let checked = check(&source);
    if !checked.diagnostics.is_empty() {
        return Err(format_diagnostics(&source, &checked.diagnostics));
    }
    Ok(checked.checked_program)
}

fn optimized_module(
    input: &CaseInput,
    opt_level: u8,
    target_backend: MirPassTargetBackend,
) -> Result<MirModule, String> {
    let checked = checked_program(input)?;
    let mir = lower_to_mir(&checked).map_err(|error| error.to_string())?;
    let pipeline = build_mir_optimization_pipeline(opt_level);
    let result = run_mir_pass_pipeline(
        mir,
        &pipeline,
        &MirPassContext {
            opt_level,
            overflow_mode: MirPassOverflowMode::Unchecked,
            bounds_mode: MirPassBoundsMode::Unchecked,
            target_backend,
            debug: Default::default(),
        },
    );
    if !result.validation_errors.is_empty() {
        return Err(result
            .validation_errors
            .into_iter()
            .map(|error| error.message)
            .collect::<Vec<_>>()
            .join("\n"));
    }
    Ok(result.module)
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
