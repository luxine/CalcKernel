use std::path::PathBuf;

#[cfg(feature = "native-toolchain")]
use std::{
    fs, process,
    sync::atomic::{AtomicU64, Ordering},
};

use calckernel::{
    inspect_profile_json, inspect_profile_text, merge_profile_inputs, parse_profile,
    validate_profile_output_path,
};

#[cfg(feature = "native-toolchain")]
use super::commands;
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
        run_native_build_workflow(&parsed, output, &profile_output)
    }
    #[cfg(not(feature = "native-toolchain"))]
    {
        Err("error: native toolchain unavailable: this developer build was compiled without the 'native-toolchain' feature".to_string())
    }
}

#[cfg(feature = "native-toolchain")]
fn run_native_build_workflow(
    parsed: &ParsedArgs,
    output: &str,
    profile_output: &str,
) -> Result<(), String> {
    let input = require_input(parsed, "pgo build")?;
    let temporary = PgoTemporaryRoot::create()?;
    let collection = temporary.path.join("shards");
    fs::create_dir(&collection).map_err(|error| format_open_file_error(&collection, error))?;
    let generation_base = temporary.path.join("training");
    let final_base = temporary.path.join("final");
    let mut generation_args = vec![
        input.to_string(),
        "--out".to_string(),
        path_argument(&generation_base)?,
        "--kind".to_string(),
        "executable".to_string(),
        "--pgo-generate".to_string(),
        path_argument(&collection)?,
        "-O3".to_string(),
    ];
    append_profile_build_modes(parsed, &mut generation_args);
    let generation = ParsedArgs::parse("build", &generation_args)?;
    commands::run_build(&generation)?;
    let generation_path = calckernel::NativeArtifactPaths::new(
        calckernel::NativePlatform::host(),
        calckernel::NativeArtifactKind::Executable,
        &generation_base,
    )
    .primary;
    let status = process::Command::new(&generation_path)
        .status()
        .map_err(|error| format!("profile training child could not start: {error}"))?;
    if !status.success() {
        return Err(match status.code() {
            Some(code) => format!("profile training child exited with status {code}"),
            None => "profile training child terminated abnormally".to_string(),
        });
    }
    let merged = merge_profile_inputs(std::slice::from_ref(&collection))
        .map_err(|error| error.to_string())?;
    if merged.profile.merged_shards != 1 {
        return Err(format!(
            "pgo build expected exactly one completed shard, observed {}",
            merged.profile.merged_shards
        ));
    }

    let mut final_args = vec![
        input.to_string(),
        "--out".to_string(),
        path_argument(&final_base)?,
        "--kind".to_string(),
        "executable".to_string(),
        "-O3".to_string(),
    ];
    append_profile_build_modes(parsed, &mut final_args);
    let final_build = ParsedArgs::parse("build", &final_args)?;
    commands::run_build(&final_build)?;
    let final_path = calckernel::NativeArtifactPaths::new(
        calckernel::NativePlatform::host(),
        calckernel::NativeArtifactKind::Executable,
        &final_base,
    )
    .primary;
    let final_bytes =
        fs::read(&final_path).map_err(|error| format_open_file_error(&final_path, error))?;
    let destination = calckernel::NativeArtifactPaths::new(
        calckernel::NativePlatform::host(),
        calckernel::NativeArtifactKind::Executable,
        &absolutize(output),
    )
    .primary;
    let profile_destination = absolutize(profile_output);
    if destination == profile_destination {
        return Err("--profile-out cannot alias the final artifact.".to_string());
    }
    validate_profile_output_path(&profile_destination).map_err(|error| error.to_string())?;
    let mut transaction = OutputTransaction::new();
    transaction.stage_executable(destination.clone(), &final_bytes)?;
    transaction.stage(profile_destination.clone(), &merged.profile_bytes)?;
    transaction.commit()?;
    println!("OK: completed CK profile training and final O3 build");
    println!("Wrote {}", destination.display());
    println!("Wrote {}", profile_destination.display());
    println!("Profile application: unweighted O3 lifecycle skeleton (stage 03)");
    Ok(())
}

#[cfg(feature = "native-toolchain")]
fn append_profile_build_modes(parsed: &ParsedArgs, output: &mut Vec<String>) {
    if let Some(value) = parsed.overflow.as_deref() {
        output.extend(["--overflow".to_string(), value.to_string()]);
    }
    if let Some(value) = parsed.bounds.as_deref() {
        output.extend(["--bounds".to_string(), value.to_string()]);
    }
    if let Some(value) = parsed.cpu {
        output.extend([
            "--cpu".to_string(),
            match value {
                CpuPolicy::Baseline => "baseline",
                CpuPolicy::Native => "native",
                CpuPolicy::Multiversion => "multiversion",
            }
            .to_string(),
        ]);
    }
    if let Some(value) = parsed.target.as_deref() {
        output.extend(["--target".to_string(), value.to_string()]);
    }
}

#[cfg(feature = "native-toolchain")]
fn path_argument(path: &std::path::Path) -> Result<String, String> {
    path.to_str()
        .map(ToString::to_string)
        .ok_or_else(|| format!("temporary PGO path is not Unicode: {}", path.display()))
}

#[cfg(feature = "native-toolchain")]
struct PgoTemporaryRoot {
    path: PathBuf,
}

#[cfg(feature = "native-toolchain")]
impl PgoTemporaryRoot {
    fn create() -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let temporary_base = fs::canonicalize(std::env::temp_dir()).map_err(|error| {
            format!("could not resolve the system temporary directory: {error}")
        })?;
        for _ in 0..128 {
            let serial = NEXT.fetch_add(1, Ordering::Relaxed);
            let path =
                temporary_base.join(format!("ckc-pgo-build-{}-{serial}", std::process::id()));
            match create_private_temporary_directory(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(format_open_file_error(&path, error)),
            }
        }
        Err("could not allocate a unique PGO build directory".to_string())
    }
}

#[cfg(feature = "native-toolchain")]
impl Drop for PgoTemporaryRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(all(feature = "native-toolchain", unix))]
fn create_private_temporary_directory(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700).create(path)
}

#[cfg(all(feature = "native-toolchain", not(unix)))]
fn create_private_temporary_directory(path: &std::path::Path) -> std::io::Result<()> {
    fs::create_dir(path)
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
