use calckernel::{BoundsMode, MirPassBoundsMode, MirPassDebugFlags, OverflowMode};

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
    pub(super) positionals: Vec<String>,
    pub(super) out: Option<String>,
    pub(super) overflow: Option<String>,
    pub(super) bounds: Option<String>,
    pub(super) opt_level: Option<String>,
    pub(super) target: Option<String>,
    pub(super) kind: Option<String>,
    pub(super) header: Option<String>,
    pub(super) debug: MirPassDebugFlags,
}

impl ParsedArgs {
    pub(super) fn parse(args: &[String]) -> Result<Self, String> {
        let mut parsed = Self {
            positionals: Vec::new(),
            out: None,
            overflow: None,
            bounds: None,
            opt_level: None,
            target: None,
            kind: None,
            header: None,
            debug: MirPassDebugFlags::default(),
        };
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--out" => {
                    index += 1;
                    parsed.out = Some(require_long_flag_value(args, index, "--out")?.to_string());
                }
                "-o" => {
                    index += 1;
                    parsed.out = Some(require_short_flag_value(args, index, "-o")?.to_string());
                }
                "--overflow" => {
                    index += 1;
                    parsed.overflow =
                        Some(require_long_flag_value(args, index, "--overflow")?.to_string());
                }
                "--bounds" => {
                    index += 1;
                    parsed.bounds =
                        Some(require_long_flag_value(args, index, "--bounds")?.to_string());
                }
                "--opt-level" => {
                    index += 1;
                    parsed.opt_level =
                        Some(require_long_flag_value(args, index, "--opt-level")?.to_string());
                }
                flag if flag.starts_with("-O") => {
                    parsed.opt_level = Some(flag[2..].to_string());
                }
                "--target" => {
                    index += 1;
                    parsed.target =
                        Some(require_long_flag_value(args, index, "--target")?.to_string());
                }
                "--kind" => {
                    index += 1;
                    parsed.kind = Some(require_long_flag_value(args, index, "--kind")?.to_string());
                }
                "--header" => {
                    index += 1;
                    parsed.header =
                        Some(require_long_flag_value(args, index, "--header")?.to_string());
                }
                "--print-pass-pipeline" => parsed.debug.print_pass_pipeline = true,
                "--print-mir-before-opt" => parsed.debug.print_mir_before_opt = true,
                "--print-mir-after-opt" => parsed.debug.print_mir_after_opt = true,
                flag if flag.starts_with("--") => {
                    index += 1;
                    let _ = require_long_flag_value(args, index, flag)?;
                }
                positional => parsed.positionals.push(positional.to_string()),
            }
            index += 1;
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
    args.opt_level
        .as_deref()
        .map_or(Ok(0), parse_opt_level_value)
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

pub(super) fn mir_bounds_mode(bounds_mode: BoundsMode) -> MirPassBoundsMode {
    match bounds_mode {
        BoundsMode::Unchecked => MirPassBoundsMode::Unchecked,
        BoundsMode::Checked => MirPassBoundsMode::Checked,
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

pub(super) fn unsupported_checked_llvm_error() -> String {
    "error: LLVM backend does not support --overflow checked yet.\n\
     Use --overflow unchecked, or use the C backend for checked arithmetic."
        .to_string()
}

pub(super) fn unsupported_checked_wasm_bounds_error() -> String {
    "error: WASM backend does not support --bounds checked yet.\n\
     help: use --bounds unchecked, or use emit-c/build for checked C bounds."
        .to_string()
}

pub(super) fn unsupported_checked_llvm_bounds_error() -> String {
    "error: LLVM backend does not support --bounds checked yet.\n\
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
        "  ckc emit-llvm <file> [--out <ll-file>] [--target <triple>] [--overflow unchecked] [--bounds unchecked] [--opt-level <0|1|2|3>]\n",
        "  ckc emit-wat <file> [--out <wat-file>] [--overflow unchecked] [--bounds unchecked] [--opt-level <0|1|2|3>]\n",
        "  ckc emit-wasm <file> --out <wasm-file> [--overflow unchecked] [--bounds unchecked] [--opt-level <0|1|2|3>]\n",
        "  ckc build <file> --out <output-path> [--overflow <unchecked|checked>] [--bounds <unchecked|checked>] [--opt-level <0|1|2|3>]\n",
        "  ckc build-llvm <file> --out <output-path> [--kind <dynamic|object>] [--target <triple>] [--overflow unchecked] [--bounds unchecked] [--opt-level <0|1|2|3>]\n",
        "  ckc licenses\n",
        "\n",
        "Options:\n",
        "  --overflow <unchecked|checked>    Arithmetic overflow handling mode. Default: unchecked.\n",
        "  --bounds <unchecked|checked>      Slice bounds mode. Default: unchecked; checked is C-only.\n",
        "  -o <file>                         Alias for --out <file>.\n",
        "  --opt-level <0|1|2|3>            MIR optimization level. Default: 0.\n",
        "  -O0, -O1, -O2, -O3              Alias for --opt-level.\n",
        "  --print-pass-pipeline           Print the selected MIR pass pipeline to stderr.\n",
        "  --print-mir-before-opt          Print MIR before optimization to stderr.\n",
        "  --print-mir-after-opt           Print MIR after optimization to stderr.\n",
    )
}
