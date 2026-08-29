use calckernel::{BoundsMode, MirPassDebugFlags, OverflowMode};

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
}

impl CpuPolicy {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "baseline" => Ok(Self::Baseline),
            "native" => Ok(Self::Native),
            _ => Err(format!(
                "Invalid value for --cpu: {value}. Expected 'baseline' or 'native'."
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
    pub(super) header: Option<String>,
    pub(super) no_cache: bool,
    pub(super) print_facts: bool,
    pub(super) print_effect_summaries: bool,
    pub(super) explain_optimization: bool,
    pub(super) sanitize_contracts: bool,
    pub(super) debug: MirPassDebugFlags,
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
            header: None,
            no_cache: false,
            print_facts: false,
            print_effect_summaries: false,
            explain_optimization: false,
            sanitize_contracts: false,
            debug: MirPassDebugFlags::default(),
        };
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--out" => {
                    require_allowed(command, "--out")?;
                    index += 1;
                    parsed.out = Some(require_long_flag_value(args, index, "--out")?.to_string());
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
                    parsed.kind = Some(ArtifactKind::parse(require_long_flag_value(
                        args, index, "--kind",
                    )?)?);
                }
                "--cpu" => {
                    require_allowed(command, "--cpu")?;
                    index += 1;
                    parsed.cpu = Some(CpuPolicy::parse(require_long_flag_value(
                        args, index, "--cpu",
                    )?)?);
                }
                "--header" => {
                    require_allowed(command, "--header")?;
                    index += 1;
                    parsed.header =
                        Some(require_long_flag_value(args, index, "--header")?.to_string());
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
                "--print-pass-pipeline" => {
                    require_allowed(command, "--debug")?;
                    parsed.debug.print_pass_pipeline = true;
                }
                "--print-mir-before-opt" => {
                    require_allowed(command, "--debug")?;
                    parsed.debug.print_mir_before_opt = true;
                }
                "--print-mir-after-opt" => {
                    require_allowed(command, "--debug")?;
                    parsed.debug.print_mir_after_opt = true;
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
    if matches!(command, "run" | "build" | "build-llvm") {
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
        ),
        "--target" => matches!(command, "emit-llvm" | "build" | "build-llvm"),
        "--kind" => matches!(command, "build" | "build-llvm"),
        "--cpu" => command == "build",
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
        "--sanitize-contracts" => matches!(command, "run" | "build" | "build-llvm"),
        "--debug" => matches!(
            command,
            "emit-mir" | "emit-c" | "emit-wat" | "emit-wasm" | "emit-llvm" | "build" | "build-llvm"
        ),
        _ => false,
    };
    if allowed {
        Ok(())
    } else {
        Err(format!("Option {flag} is not valid for '{command}'."))
    }
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
        "  ckc emit-kir <file> [--out <kir-file>] [--overflow <unchecked|checked>] [--bounds <unchecked|checked>] [--opt-level <0|1|2|3>] [inspection options]\n",
        "  ckc emit-llvm <file> [--out <ll-file>] [--target <host-triple>] [--overflow <unchecked|checked>] [--bounds <unchecked|checked>] [--opt-level <0|1|2|3>]\n",
        "  ckc emit-wat <file> [--out <wat-file>] [--overflow unchecked] [--bounds unchecked] [--opt-level <0|1|2|3>]\n",
        "  ckc emit-wasm <file> --out <wasm-file> [--overflow unchecked] [--bounds unchecked] [--opt-level <0|1|2|3>]\n",
        "  ckc build <file> --out <output-path> [--kind <executable|dynamic|static|object>] [--overflow <unchecked|checked>] [--bounds <unchecked|checked>] [--cpu <baseline|native>] [-O0|-O1|-O2|-O3] [--sanitize-contracts]\n",
        "  ckc build-llvm <file> --out <output-path> [--kind <dynamic|object>] [native build options]\n",
        "  ckc run <file> [-O0|-O1|-O2|-O3] [--overflow <unchecked|checked>] [--bounds <unchecked|checked>] [--no-cache] [--sanitize-contracts]\n",
        "  ckc cache clean\n",
        "  ckc licenses\n",
        "\n",
        "Options:\n",
        "  --overflow <unchecked|checked>    Arithmetic overflow handling mode. Default: unchecked.\n",
        "  --bounds <unchecked|checked>      Slice bounds mode. Default: unchecked.\n",
        "  -o <file>                         Alias for --out <file>.\n",
        "  --opt-level <0|1|2|3>            MIR and LLVM optimization level.\n",
        "  -O0, -O1, -O2, -O3              Alias for --opt-level.\n",
        "  --print-pass-pipeline           Print the selected MIR pass pipeline to stderr.\n",
        "  --print-mir-before-opt          Print MIR before optimization to stderr.\n",
        "  --print-mir-after-opt           Print MIR after optimization to stderr.\n",
        "  --print-facts                   Print deterministic verified KIR facts to stderr.\n",
        "  --print-effect-summaries        Print deterministic effect summaries to stderr.\n",
        "  --explain-optimization          Explain removed and retained KIR checks to stderr.\n",
        "  --sanitize-contracts            Check unsafe contracts in run/executable builds.\n",
    )
}

#[cfg(test)]
mod tests {
    use super::{ParsedArgs, parse_opt_level};

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
}
