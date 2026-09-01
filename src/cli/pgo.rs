use std::path::PathBuf;

use calckernel::{
    inspect_profile_json, inspect_profile_text, merge_profile_inputs, parse_profile,
    validate_profile_output_path,
};

use super::{args::*, output::*};

pub(super) fn run(args: &[String]) -> Result<(), String> {
    let Some(subcommand) = args.first().map(String::as_str) else {
        return Err(pgo_usage().to_string());
    };
    match subcommand {
        "merge" => run_merge(&args[1..]),
        "inspect" => run_inspect(&args[1..]),
        "build" => run_build_workflow(&args[1..]),
        _ => Err(format!(
            "Unknown pgo command: {subcommand}.\n{}",
            pgo_usage()
        )),
    }
}

fn run_merge(args: &[String]) -> Result<(), String> {
    let mut inputs = Vec::new();
    let mut output = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--out" | "-o" => {
                let flag = args[index].as_str();
                if output.is_some() {
                    return Err(format!("Option {flag} was provided more than once."));
                }
                index += 1;
                output = Some(require_pgo_value(args, index, flag)?.to_string());
            }
            flag if flag.starts_with('-') => {
                return Err(format!("Unknown option for 'pgo merge': {flag}."));
            }
            input => inputs.push(PathBuf::from(input)),
        }
        index += 1;
    }
    if inputs.is_empty() {
        return Err(format!(
            "pgo merge requires at least one raw shard.\n{}",
            pgo_usage()
        ));
    }
    let output = output.ok_or_else(|| {
        format!(
            "pgo merge requires --out <profile.ckprof>.\n{}",
            pgo_usage()
        )
    })?;
    let absolute_output = absolutize(&output);
    if inputs
        .iter()
        .map(|input| absolutize(&input.to_string_lossy()))
        .any(|input| input == absolute_output)
    {
        return Err("pgo merge output cannot alias an input shard.".to_string());
    }
    validate_profile_output_path(&absolute_output).map_err(|error| error.to_string())?;
    let result = merge_profile_inputs(&inputs).map_err(|error| error.to_string())?;
    write_bytes_atomic(&output, &result.profile_bytes)?;
    println!("OK: merged CK profile");
    println!("Wrote {}", absolute_output.display());
    println!("Merged {} shard(s)", result.profile.merged_shards);
    if result.ignored_temporary_files != 0 {
        println!(
            "Ignored {} temporary shard file(s)",
            result.ignored_temporary_files
        );
    }
    Ok(())
}

fn run_inspect(args: &[String]) -> Result<(), String> {
    let mut input = None;
    let mut json = false;
    for argument in args {
        match argument.as_str() {
            "--json" if json => {
                return Err("Option --json was provided more than once.".to_string());
            }
            "--json" => json = true,
            flag if flag.starts_with('-') => {
                return Err(format!("Unknown option for 'pgo inspect': {flag}."));
            }
            path if input.is_some() => {
                return Err(format!("Unexpected pgo inspect input: {path}."));
            }
            path => input = Some(path),
        }
    }
    let input = input.ok_or_else(|| format!("pgo inspect requires a profile.\n{}", pgo_usage()))?;
    let bytes = read_file_bytes(input)?;
    let profile = parse_profile(&bytes).map_err(|error| error.to_string())?;
    if json {
        print!(
            "{}",
            inspect_profile_json(&profile).map_err(|error| error.to_string())?
        );
    } else {
        print!(
            "{}",
            inspect_profile_text(&profile).map_err(|error| error.to_string())?
        );
    }
    Ok(())
}

fn run_build_workflow(args: &[String]) -> Result<(), String> {
    let parsed = ParsedArgs::parse("pgo-build", args)?;
    let _ = require_input(&parsed, "pgo build")?;
    let output = require_out(&parsed, "pgo build")?;
    let profile_output = parsed
        .profile_out
        .clone()
        .unwrap_or_else(|| format!("{output}.ckprof"));
    if absolutize(output) == absolutize(&profile_output) {
        return Err("--profile-out cannot alias the final artifact.".to_string());
    }
    #[cfg(feature = "native-toolchain")]
    {
        Err("pgo build is unavailable until the profile collection runtime is linked.".to_string())
    }
    #[cfg(not(feature = "native-toolchain"))]
    {
        Err("error: native toolchain unavailable: this developer build was compiled without the 'native-toolchain' feature".to_string())
    }
}

fn require_pgo_value<'a>(args: &'a [String], index: usize, flag: &str) -> Result<&'a str, String> {
    let value = args
        .get(index)
        .map(String::as_str)
        .ok_or_else(|| format!("Missing value for {flag}."))?;
    if value.starts_with('-') {
        return Err(format!("Missing value for {flag}."));
    }
    Ok(value)
}

fn pgo_usage() -> &'static str {
    "Usage:\n  ckc pgo build <file> --out <executable> [--profile-out <file.ckprof>] [-O3]\n  ckc pgo merge <shard-or-directory>... --out <file.ckprof>\n  ckc pgo inspect <file.ckprof> [--json]\n"
}
