use calckernel::{BoundsMode, KirConsumer, OverflowMode, TuneBudget};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ArtifactKind {
    Executable,
    Dynamic,
    Static,
    Object,
}

impl ArtifactKind {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "executable" => Ok(Self::Executable),
            "dynamic" => Ok(Self::Dynamic),
            "static" => Ok(Self::Static),
            "object" => Ok(Self::Object),
            _ => Err(format!(
                "Invalid value for --kind: {value}. Expected 'executable', 'dynamic', 'static', or 'object'."
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CpuPolicy {
    Baseline,
    Native,
    Multiversion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EmitKirConsumer {
    Inspection,
    C,
    WebAssembly,
    NativeLibrary,
    NativeExecutable,
}

impl EmitKirConsumer {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "inspection" => Ok(Self::Inspection),
            "c" => Ok(Self::C),
            "wasm" => Ok(Self::WebAssembly),
            "native-library" => Ok(Self::NativeLibrary),
            "native-executable" => Ok(Self::NativeExecutable),
            _ => Err(format!(
                "Invalid value for --consumer: {value}. Expected 'inspection', 'c', 'wasm', 'native-library', or 'native-executable'."
            )),
        }
    }

    pub(super) const fn kir_consumer(self) -> KirConsumer {
        match self {
            Self::Inspection => KirConsumer::Inspection,
            Self::C => KirConsumer::C,
            Self::WebAssembly => KirConsumer::WebAssembly,
            Self::NativeLibrary => KirConsumer::NativeLibrary,
            Self::NativeExecutable => KirConsumer::NativeExecutable,
        }
    }

    const fn is_native(self) -> bool {
        matches!(self, Self::NativeLibrary | Self::NativeExecutable)
    }
}

impl CpuPolicy {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "baseline" => Ok(Self::Baseline),
            "native" => Ok(Self::Native),
            "multiversion" => Ok(Self::Multiversion),
            _ => Err(format!(
                "Invalid value for --cpu: {value}. Expected 'baseline', 'native', or 'multiversion'."
            )),
        }
    }
}

pub(super) fn require_input<'args>(
    args: &'args ParsedArgs,
    command: &str,
) -> Result<&'args str, String> {
    if args.positionals.len() != 1 {
        return Err(format!("Usage error for '{command}'.\n{}\n", usage()));
    }
    Ok(&args.positionals[0])
}

pub(super) fn require_out<'args>(
    args: &'args ParsedArgs,
    command: &str,
) -> Result<&'args str, String> {
    args.out
        .as_deref()
        .ok_or_else(|| format!("Usage error for '{command}': missing --out.\n{}\n", usage()))
}

#[derive(Debug, Clone)]
pub(super) struct ParsedArgs {
    command: String,
    pub(super) positionals: Vec<String>,
    pub(super) out: Option<String>,
    pub(super) overflow: Option<String>,
    pub(super) bounds: Option<String>,
    pub(super) opt_level: Option<String>,
    pub(super) target: Option<String>,
    pub(super) kind: Option<ArtifactKind>,
    pub(super) cpu: Option<CpuPolicy>,
    pub(super) consumer: Option<EmitKirConsumer>,
    pub(super) header: Option<String>,
    pub(super) profile_out: Option<String>,
    pub(super) pgo_generate: Option<String>,
    pub(super) pgo_use: Option<String>,
    pub(super) tune_use: Option<String>,
    pub(super) tune_config: Option<String>,
    pub(super) tune_budget: Option<TuneBudget>,
    pub(super) tune_out: Option<String>,
    pub(super) no_tune_cache: bool,
    pub(super) no_cache: bool,
    pub(super) print_facts: bool,
    pub(super) print_effect_summaries: bool,
    pub(super) explain_optimization: bool,
    pub(super) sanitize_contracts: bool,
}

impl ParsedArgs {
    pub(super) fn parse(command: &str, args: &[String]) -> Result<Self, String> {
        let mut parsed = Self {
            command: command.to_string(),
            positionals: Vec::new(),
            out: None,
            overflow: None,
            bounds: None,
            opt_level: None,
            target: None,
            kind: None,
            cpu: None,
            consumer: None,
            header: None,
            profile_out: None,
            pgo_generate: None,
            pgo_use: None,
            tune_use: None,
            tune_config: None,
            tune_budget: None,
            tune_out: None,
            no_tune_cache: false,
            no_cache: false,
            print_facts: false,
            print_effect_summaries: false,
            explain_optimization: false,
            sanitize_contracts: false,
        };
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--out" => {
                    require_allowed(command, "--out")?;
                    index += 1;
                    let value = require_long_flag_value(args, index, "--out")?.to_string();
                    set_once(&mut parsed.out, value, "--out")?;
                }
                "-o" => {
                    require_allowed(command, "--out")?;
                    index += 1;
                    parsed.out = Some(require_short_flag_value(args, index, "-o")?.to_string());
                }
                "--overflow" => {
                    require_allowed(command, "--overflow")?;
                    index += 1;
                    parsed.overflow =
                        Some(require_long_flag_value(args, index, "--overflow")?.to_string());
                }
                "--bounds" => {
                    require_allowed(command, "--bounds")?;
                    index += 1;
                    parsed.bounds =
                        Some(require_long_flag_value(args, index, "--bounds")?.to_string());
                }
                "--opt-level" => {
                    require_allowed(command, "--opt-level")?;
                    index += 1;
                    parsed.opt_level =
                        Some(require_long_flag_value(args, index, "--opt-level")?.to_string());
                }
                flag if flag.starts_with("-O") => {
                    require_allowed(command, "--opt-level")?;
                    parsed.opt_level = Some(flag[2..].to_string());
                }
                "--target" => {
                    require_allowed(command, "--target")?;
                    index += 1;
                    parsed.target =
                        Some(require_long_flag_value(args, index, "--target")?.to_string());
                }
                "--kind" => {
                    require_allowed(command, "--kind")?;
                    index += 1;
                    let value =
                        ArtifactKind::parse(require_long_flag_value(args, index, "--kind")?)?;
                    set_once(&mut parsed.kind, value, "--kind")?;
                }
                "--cpu" => {
                    require_allowed(command, "--cpu")?;
                    index += 1;
                    let value = CpuPolicy::parse(require_long_flag_value(args, index, "--cpu")?)?;
                    set_once(&mut parsed.cpu, value, "--cpu")?;
                }
                "--consumer" => {
                    require_allowed(command, "--consumer")?;
                    index += 1;
                    parsed.consumer = Some(EmitKirConsumer::parse(require_long_flag_value(
                        args,
                        index,
                        "--consumer",
                    )?)?);
                }
                "--header" => {
                    require_allowed(command, "--header")?;
                    index += 1;
                    parsed.header =
                        Some(require_long_flag_value(args, index, "--header")?.to_string());
                }
                "--profile-out" => {
                    require_allowed(command, "--profile-out")?;
                    index += 1;
                    let value = require_long_flag_value(args, index, "--profile-out")?.to_string();
                    set_once(&mut parsed.profile_out, value, "--profile-out")?;
                }
                "--pgo-generate" => {
                    require_allowed(command, "--pgo-generate")?;
                    index += 1;
                    let value = require_long_flag_value(args, index, "--pgo-generate")?.to_string();
                    set_once(&mut parsed.pgo_generate, value, "--pgo-generate")?;
                }
                "--pgo-use" => {
                    require_allowed(command, "--pgo-use")?;
                    index += 1;
                    let value = require_long_flag_value(args, index, "--pgo-use")?.to_string();
                    set_once(&mut parsed.pgo_use, value, "--pgo-use")?;
                }
                "--tune-use" => {
                    require_allowed(command, "--tune-use")?;
                    index += 1;
                    let value = require_long_flag_value(args, index, "--tune-use")?.to_string();
                    set_once(&mut parsed.tune_use, value, "--tune-use")?;
                }
                "--config" => {
                    require_allowed(command, "--config")?;
                    index += 1;
                    let value = require_long_flag_value(args, index, "--config")?.to_string();
                    set_once(&mut parsed.tune_config, value, "--config")?;
                }
                "--budget" => {
                    require_allowed(command, "--budget")?;
                    index += 1;
                    let value = match require_long_flag_value(args, index, "--budget")? {
                        "quick" => TuneBudget::Quick,
                        "standard" => TuneBudget::Standard,
                        "thorough" => TuneBudget::Thorough,
                        other => {
                            return Err(format!(
                                "Invalid value for --budget: {other}. Expected 'quick', 'standard', or 'thorough'."
                            ));
                        }
                    };
                    set_once(&mut parsed.tune_budget, value, "--budget")?;
                }
                "--tune-out" => {
                    require_allowed(command, "--tune-out")?;
                    index += 1;
                    let value = require_long_flag_value(args, index, "--tune-out")?.to_string();
                    set_once(&mut parsed.tune_out, value, "--tune-out")?;
                }
                "--no-tune-cache" => {
                    require_allowed(command, "--no-tune-cache")?;
                    if parsed.no_tune_cache {
                        return Err(
                            "Option --no-tune-cache was provided more than once.".to_string()
                        );
                    }
                    parsed.no_tune_cache = true;
                }
                "--no-cache" => {
                    require_allowed(command, "--no-cache")?;
                    parsed.no_cache = true;
                }
                "--print-facts" => {
                    require_allowed(command, "--inspection")?;
                    parsed.print_facts = true;
                }
                "--print-effect-summaries" => {
                    require_allowed(command, "--inspection")?;
                    parsed.print_effect_summaries = true;
                }
                "--explain-optimization" => {
                    require_allowed(command, "--inspection")?;
                    parsed.explain_optimization = true;
                }
                "--sanitize-contracts" => {
                    require_allowed(command, "--sanitize-contracts")?;
                    parsed.sanitize_contracts = true;
                }
                flag if flag.starts_with('-') => return Err(format!("Unknown option: {flag}.")),
                positional => parsed.positionals.push(positional.to_string()),
            }
            index += 1;
        }
        if parsed.sanitize_contracts
            && command == "build"
            && parsed.kind != Some(ArtifactKind::Executable)
        {
            return Err(
                "Option --sanitize-contracts requires 'build --kind executable'.".to_string(),
            );
        }
        if parsed.sanitize_contracts && command == "build-llvm" {
            return Err("Option --sanitize-contracts is not valid for 'build-llvm'.".to_string());
        }
        if command == "emit-kir" {
            let consumer = parsed.consumer.unwrap_or(EmitKirConsumer::Inspection);
            if parsed.cpu.is_some() && !consumer.is_native() {
                return Err(
                    "Option --cpu is valid only with a Native emit-kir consumer.".to_string(),
                );
            }
            if consumer.is_native() && parsed.cpu.is_none() {
                parsed.cpu = Some(CpuPolicy::Baseline);
            }
        }
        validate_profile_options(&parsed)?;
        validate_tune_options(&parsed)?;
        Ok(parsed)
    }
}

pub(super) fn parse_opt_level_value(value: &str) -> Result<u8, String> {
    match value {
        "0" => Ok(0),
        "1" => Ok(1),
        "2" => Ok(2),
        "3" => Ok(3),
        other => Err(format!(
            "Invalid optimization level: {other}. Expected 0, 1, 2, or 3."
        )),
    }
}

pub(super) fn parse_opt_level(args: &ParsedArgs) -> Result<u8, String> {
    args.opt_level.as_deref().map_or_else(
        || Ok(default_opt_level(&args.command)),
        parse_opt_level_value,
    )
}

fn default_opt_level(command: &str) -> u8 {
    if matches!(
        command,
        "run" | "build" | "build-llvm" | "pgo-build" | "tune-build"
    ) {
        3
    } else {
        0
    }
}

fn require_allowed(command: &str, flag: &str) -> Result<(), String> {
    let allowed = match flag {
        "--out" => matches!(
            command,
            "emit-mir"
                | "emit-kir"
                | "emit-c"
                | "emit-wat"
                | "emit-wasm"
                | "emit-llvm"
                | "build"
                | "build-llvm"
                | "pgo-build"
                | "tune-build"
        ),
        "--overflow" | "--bounds" => matches!(
            command,
            "emit-kir"
                | "emit-c"
                | "emit-wat"
                | "emit-wasm"
                | "emit-llvm"
                | "build"
                | "build-llvm"
                | "run"
                | "pgo-build"
                | "tune-build"
        ),
        "--opt-level" => matches!(
            command,
            "emit-mir"
                | "emit-kir"
                | "emit-c"
                | "emit-wat"
                | "emit-wasm"
                | "emit-llvm"
                | "build"
                | "build-llvm"
                | "run"
                | "pgo-build"
                | "tune-build"
        ),
        "--target" => matches!(command, "emit-llvm" | "build" | "build-llvm" | "tune-build"),
        "--kind" => matches!(command, "build" | "build-llvm" | "tune-build"),
        "--cpu" => matches!(command, "build" | "emit-kir" | "pgo-build" | "tune-build"),
        "--consumer" => command == "emit-kir",
        "--header" => command == "emit-c",
        "--no-cache" => command == "run",
        "--inspection" => matches!(
            command,
            "emit-kir"
                | "emit-c"
                | "emit-wat"
                | "emit-wasm"
                | "emit-llvm"
                | "build"
                | "build-llvm"
                | "run"
        ),
        "--sanitize-contracts" => {
            matches!(
                command,
                "run" | "build" | "build-llvm" | "pgo-build" | "tune-build"
            )
        }
        "--profile-out" => command == "pgo-build",
        "--pgo-generate" => matches!(command, "build" | "tune-build"),
        "--pgo-use" => matches!(command, "build" | "emit-kir" | "tune-build"),
        "--tune-use" => command == "build",
        "--config" | "--budget" | "--tune-out" | "--no-tune-cache" => command == "tune-build",
        _ => false,
    };
    if allowed {
        Ok(())
    } else {
        Err(format!("Option {flag} is not valid for '{command}'."))
    }
}

fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> Result<(), String> {
    if slot.is_some() {
        return Err(format!("Option {flag} was provided more than once."));
    }
    *slot = Some(value);
    Ok(())
}

fn validate_profile_options(args: &ParsedArgs) -> Result<(), String> {
    if args.pgo_generate.is_some() && args.pgo_use.is_some() {
        return Err("Options --pgo-generate and --pgo-use are mutually exclusive.".to_string());
    }
    let profile_requested = args.pgo_generate.is_some() || args.pgo_use.is_some();
    if args.sanitize_contracts
        && (profile_requested
            || args.command == "pgo-build"
            || args.cpu == Some(CpuPolicy::Multiversion))
    {
        return Err(
            "Contract sanitizer mode is incompatible with PGO and multiversioning.".to_string(),
        );
    }
    if args.pgo_generate.is_some() && args.kind == Some(ArtifactKind::Object) {
        return Err("Profile generation does not support --kind object.".to_string());
    }
    if args.command == "emit-kir" && args.pgo_use.is_some() {
        let consumer = args.consumer.unwrap_or(EmitKirConsumer::Inspection);
        if !consumer.is_native() {
            return Err("Option --pgo-use requires a Native emit-kir consumer.".to_string());
        }
    }
    let level = parse_opt_level(args)?;
    if profile_requested && level < 2 {
        return Err("Profile generation and use require -O2 or -O3.".to_string());
    }
    if args.cpu == Some(CpuPolicy::Multiversion) {
        if level != 3 {
            return Err("CPU multiversioning requires -O3.".to_string());
        }
        if args.command == "build" && args.kind == Some(ArtifactKind::Object) {
            return Err("CPU multiversioning does not support --kind object.".to_string());
        }
    }
    Ok(())
}

fn validate_tune_options(args: &ParsedArgs) -> Result<(), String> {
    if args.command == "tune-build" {
        if !matches!(
            args.kind,
            Some(ArtifactKind::Executable | ArtifactKind::Dynamic)
        ) {
            return Err("'tune build' requires --kind executable or --kind dynamic.".to_string());
        }
        if args.cpu != Some(CpuPolicy::Native) {
            return Err("'tune build' requires --cpu native.".to_string());
        }
        if parse_opt_level(args)? != 3 {
            return Err("'tune build' requires -O3.".to_string());
        }
        if args.pgo_generate.is_some() {
            return Err("'tune build' does not support --pgo-generate.".to_string());
        }
        if args.sanitize_contracts {
            return Err("'tune build' does not support --sanitize-contracts.".to_string());
        }
    }
    if args.tune_use.is_some() {
        if args.pgo_generate.is_some() {
            return Err(
                "Options --tune-use and --pgo-generate are mutually exclusive.".to_string(),
            );
        }
        if args.sanitize_contracts || args.cpu == Some(CpuPolicy::Multiversion) {
            return Err(
                "Tune replay is incompatible with sanitizer and multiversion modes.".to_string(),
            );
        }
        if !matches!(
            args.kind,
            Some(ArtifactKind::Executable | ArtifactKind::Dynamic)
        ) {
            return Err("Tune replay requires --kind executable or --kind dynamic.".to_string());
        }
        if args.cpu != Some(CpuPolicy::Native) {
            return Err("Tune replay requires --cpu native.".to_string());
        }
        if parse_opt_level(args)? != 3 {
            return Err("Tune replay requires -O3.".to_string());
        }
    }
    Ok(())
}

pub(super) fn parse_overflow_mode(args: &ParsedArgs) -> Result<OverflowMode, String> {
    match args.overflow.as_deref().unwrap_or("unchecked") {
        "unchecked" => Ok(OverflowMode::Unchecked),
        "checked" => Ok(OverflowMode::Checked),
        other => Err(format!(
            "Invalid value for --overflow: {other}. Expected 'unchecked' or 'checked'."
        )),
    }
}

pub(super) fn parse_bounds_mode(args: &ParsedArgs) -> Result<BoundsMode, String> {
    match args.bounds.as_deref().unwrap_or("unchecked") {
        "unchecked" => Ok(BoundsMode::Unchecked),
        "checked" => Ok(BoundsMode::Checked),
        other => Err(format!(
            "Invalid value for --bounds: {other}. Expected 'unchecked' or 'checked'."
        )),
    }
}

pub(super) fn bounds_mode_name(bounds_mode: BoundsMode) -> &'static str {
    match bounds_mode {
        BoundsMode::Unchecked => "unchecked",
        BoundsMode::Checked => "checked",
    }
}

pub(super) fn unsupported_checked_wasm_error() -> String {
    "error: WASM backend does not support --overflow checked yet.\n\
     help: use --overflow unchecked, or use emit-c/build for checked C output."
        .to_string()
}

pub(super) fn unsupported_checked_wasm_bounds_error() -> String {
    "error: WASM backend does not support --bounds checked yet.\n\
     help: use --bounds unchecked, or use emit-c/build for checked C bounds."
        .to_string()
}

pub(super) fn require_long_flag_value<'args>(
    args: &'args [String],
    index: usize,
    flag: &str,
) -> Result<&'args str, String> {
    let Some(value) = args.get(index).map(String::as_str) else {
        return Err(format!("Missing value for {flag}."));
    };
    if value.starts_with("--") {
        return Err(format!("Missing value for {flag}."));
    }
    Ok(value)
}

pub(super) fn require_short_flag_value<'args>(
    args: &'args [String],
    index: usize,
    flag: &str,
) -> Result<&'args str, String> {
    let Some(value) = args.get(index).map(String::as_str) else {
        return Err(format!("Missing value for {flag}."));
    };
    if value.starts_with('-') {
        return Err(format!("Missing value for {flag}."));
    }
    Ok(value)
}

pub(super) fn usage() -> &'static str {
    concat!(
        "Usage:\n",
        "  ckc check <file>\n",
        "  ckc emit-c <file> --out <c-file> [--header <h-file>] [--overflow <unchecked|checked>] [--bounds <unchecked|checked>] [--opt-level <0|1|2|3>]\n",
        "  ckc emit-mir <file> [--out <mir-file>] [--opt-level <0|1|2|3>]\n",
        "  ckc emit-kir <file> [--out <kir-file>] [--consumer <inspection|c|wasm|native-library|native-executable>] [--cpu <baseline|native|multiversion>] [--pgo-use <file.ckprof>] [--overflow <unchecked|checked>] [--bounds <unchecked|checked>] [--opt-level <0|1|2|3>] [inspection options]\n",
        "  ckc emit-llvm <file> [--out <ll-file>] [--target <host-triple>] [--overflow <unchecked|checked>] [--bounds <unchecked|checked>] [--opt-level <0|1|2|3>]\n",
        "  ckc emit-wat <file> [--out <wat-file>] [--overflow unchecked] [--bounds unchecked] [--opt-level <0|1|2|3>]\n",
        "  ckc emit-wasm <file> --out <wasm-file> [--overflow unchecked] [--bounds unchecked] [--opt-level <0|1|2|3>]\n",
        "  ckc build <file> --out <output-path> [--kind <executable|dynamic|static|object>] [--overflow <unchecked|checked>] [--bounds <unchecked|checked>] [--cpu <baseline|native|multiversion>] [--pgo-generate <directory>|--pgo-use <file.ckprof>] [--tune-use <decision.cktune>] [-O0|-O1|-O2|-O3] [--sanitize-contracts]\n",
        "  ckc tune build <file> --config <workload.cktune.toml> --out <artifact> --kind <executable|dynamic> --cpu native -O3 [--budget <quick|standard|thorough>] [--tune-out <decision.cktune>] [--no-tune-cache]\n",
        "  ckc tune inspect <decision.cktune> [--json]\n",
        "  ckc build-llvm <file> --out <output-path> [--kind <dynamic|object>] [native build options]\n",
        "  ckc pgo build <file> --out <executable> [--profile-out <file.ckprof>] [-O3]\n",
        "  ckc pgo merge <shard-or-directory>... --out <file.ckprof>\n",
        "  ckc pgo inspect <file.ckprof> [--json]\n",
        "  ckc run <file> [-O0|-O1|-O2|-O3] [--overflow <unchecked|checked>] [--bounds <unchecked|checked>] [--no-cache] [--sanitize-contracts]\n",
        "  ckc cache clean\n",
        "  ckc licenses\n",
        "\n",
        "Options:\n",
        "  --overflow <unchecked|checked>    Arithmetic overflow handling mode. Default: unchecked.\n",
        "  --bounds <unchecked|checked>      Slice bounds mode. Default: unchecked.\n",
        "  -o <file>                         Alias for --out <file>.\n",
        "  --opt-level <0|1|2|3>            KIR and backend optimization level.\n",
        "  --consumer <consumer>             Consumer profile for emit-kir. Default: inspection.\n",
        "  --cpu <baseline|native|multiversion> CPU policy for build or Native emit-kir.\n",
        "  --pgo-generate <directory>         Build a temporary Native collection artifact.\n",
        "  --pgo-use <file.ckprof>            Apply a validated CK workload profile.\n",
        "  --profile-out <file.ckprof>        Output profile for 'pgo build'.\n",
        "  --tune-use <decision.cktune>       Replay one exact validated tuning decision.\n",
        "  --budget <preset>                  Tuning budget: quick, standard, or thorough.\n",
        "  --tune-out <decision.cktune>       Explicit tuning-decision output path.\n",
        "  --no-tune-cache                    Force a fresh offline tuning session.\n",
        "  -O0, -O1, -O2, -O3              Alias for --opt-level.\n",
        "  --print-facts                   Print deterministic verified KIR facts to stderr.\n",
        "  --print-effect-summaries        Print deterministic effect summaries to stderr.\n",
        "  --explain-optimization          Explain removed and retained KIR checks to stderr.\n",
        "  --sanitize-contracts            Check unsafe contracts in run/executable builds.\n",
    )
}

#[cfg(test)]
mod tests {
    use super::{CpuPolicy, ParsedArgs, parse_opt_level};

    #[test]
    fn execution_commands_default_to_o3_and_inspection_defaults_to_o0() {
        for command in ["run", "build", "build-llvm"] {
            let parsed = ParsedArgs::parse(command, &[]).expect("parse execution command");
            assert_eq!(parse_opt_level(&parsed), Ok(3), "{command}");
        }
        for command in [
            "emit-mir",
            "emit-kir",
            "emit-c",
            "emit-wat",
            "emit-wasm",
            "emit-llvm",
        ] {
            let parsed = ParsedArgs::parse(command, &[]).expect("parse inspection command");
            assert_eq!(parse_opt_level(&parsed), Ok(0), "{command}");
        }
    }

    #[test]
    fn explicit_optimization_level_overrides_every_default() {
        for command in ["run", "build", "emit-llvm"] {
            for level in 0..=3 {
                let parsed = ParsedArgs::parse(command, &[format!("-O{level}")])
                    .expect("parse optimization override");
                assert_eq!(parse_opt_level(&parsed), Ok(level), "{command} -O{level}");
            }
        }
    }

    #[test]
    fn pgo_cli_parser_should_reject_mutually_exclusive_profile_modes() {
        let error = ParsedArgs::parse(
            "build",
            &[
                "input.ck".to_string(),
                "--pgo-generate".to_string(),
                "profiles".to_string(),
                "--pgo-use".to_string(),
                "input.ckprof".to_string(),
            ],
        )
        .expect_err("reject mutually exclusive profile modes");

        assert!(error.contains("mutually exclusive"), "{error}");
    }

    #[test]
    fn pgo_cli_parser_should_reject_generation_object() {
        let error = ParsedArgs::parse(
            "build",
            &[
                "--kind".to_string(),
                "object".to_string(),
                "--pgo-generate".to_string(),
                "profiles".to_string(),
            ],
        )
        .expect_err("reject object generation");

        assert!(error.contains("does not support --kind object"), "{error}");
    }

    #[test]
    fn pgo_cli_parser_should_reject_profile_use_below_o2() {
        let error = ParsedArgs::parse(
            "build",
            &[
                "--pgo-use".to_string(),
                "input.ckprof".to_string(),
                "-O1".to_string(),
            ],
        )
        .expect_err("reject O1 profile use");

        assert!(error.contains("require -O2 or -O3"), "{error}");
    }

    #[test]
    fn pgo_cli_parser_should_reject_multiversion_below_o3() {
        let error = ParsedArgs::parse(
            "build",
            &[
                "--cpu".to_string(),
                "multiversion".to_string(),
                "-O2".to_string(),
            ],
        )
        .expect_err("reject O2 multiversioning");

        assert!(error.contains("requires -O3"), "{error}");
    }

    #[test]
    fn pgo_cli_parser_should_reject_sanitized_profile_use() {
        let error = ParsedArgs::parse(
            "build",
            &[
                "--kind".to_string(),
                "executable".to_string(),
                "--sanitize-contracts".to_string(),
                "--pgo-use".to_string(),
                "input.ckprof".to_string(),
            ],
        )
        .expect_err("reject sanitizer with profile use");

        assert!(error.contains("incompatible with PGO"), "{error}");
    }

    #[test]
    fn pgo_cli_parser_should_require_native_emit_kir_consumer() {
        let error = ParsedArgs::parse(
            "emit-kir",
            &[
                "--consumer".to_string(),
                "c".to_string(),
                "--pgo-use".to_string(),
                "input.ckprof".to_string(),
            ],
        )
        .expect_err("reject portable profile use");

        assert!(error.contains("Native emit-kir consumer"), "{error}");
    }

    #[test]
    fn pgo_cli_parser_should_accept_multiversion_o3_and_profile_output() {
        let parsed = ParsedArgs::parse(
            "pgo-build",
            &[
                "input.ck".to_string(),
                "--out".to_string(),
                "app".to_string(),
                "--profile-out".to_string(),
                "app.ckprof".to_string(),
                "--cpu".to_string(),
                "multiversion".to_string(),
            ],
        )
        .expect("accept O3 pgo convenience command");

        assert_eq!(parsed.cpu, Some(CpuPolicy::Multiversion));
        assert_eq!(parsed.profile_out.as_deref(), Some("app.ckprof"));
        assert_eq!(parse_opt_level(&parsed), Ok(3));
    }
}
