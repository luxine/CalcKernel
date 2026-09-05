// Standalone schema-1 runner for the frozen CK 0.14 tuning corpus.

mod predicated;

use std::{env, fs, path::PathBuf, time::Instant};

use calckernel::decode_input_map;
use sha2::{Digest, Sha256};

fn main() {
    if let Err(error) = dispatch() {
        eprintln!("ckc-tune-runner: {error}");
        std::process::exit(1);
    }
}

fn dispatch() -> Result<(), String> {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    if arguments == ["--ck-tune"] {
        return run_tune();
    }
    if arguments == ["--ck-predicated-tune"] {
        return run_predicated_tune();
    }
    if arguments.first().map(String::as_str) == Some("--ck-predicated-profile") {
        return run_predicated_profile(&arguments);
    }
    if arguments.first().map(String::as_str) == Some("--ck-predicated-oracle") {
        return run_predicated_oracle(&arguments);
    }
    if arguments.first().map(String::as_str) == Some("--ck-predicated-perf") {
        return run_predicated_performance(&arguments);
    }
    if arguments.first().map(String::as_str) == Some("--ck-perf") {
        return run_performance(&arguments);
    }
    Err("expected one exact supported tuning runner protocol".to_string())
}

fn run_predicated_tune() -> Result<(), String> {
    if env::var("CK_TUNE_PROTOCOL").as_deref() != Ok("1")
        || env::var("CK_TUNE_ARTIFACT_KIND").as_deref() != Ok("dynamic")
    {
        return Err("unsupported predicated tuning protocol or artifact kind".to_string());
    }
    let case_id = required("CK_TUNE_CASE")?;
    let expected = match case_id.as_str() {
        "predicated-update.search" => predicated::TRAINING,
        "predicated-update.validation" => predicated::VALIDATION,
        _ => return Err("predicated tuning case is not frozen".to_string()),
    };
    let seed = canonical_u64(&required("CK_TUNE_SEED")?, "CK_TUNE_SEED")?;
    let iterations = canonical_u64(
        &required("CK_TUNE_ITERATIONS")?,
        "CK_TUNE_ITERATIONS",
    )?;
    if seed != expected.seed {
        return Err("predicated tuning seed does not match its frozen split".to_string());
    }
    predicated::checked_invocation_bytes(expected.n, iterations)?;

    let artifact = PathBuf::from(required("CK_TUNE_ARTIFACT")?);
    let input_map_path = PathBuf::from(required("CK_TUNE_INPUT_MAP")?);
    let map = decode_input_map(&fs::read(&input_map_path).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    if map.iter().any(|entry| {
        entry.logical_path == "fixtures/tune/predicated-update-release.tsv"
            || entry.logical_path.contains("release-held-out")
    }) {
        return Err("release-held-out input is forbidden in CKTIMAP1".to_string());
    }
    let logical_path = match expected.name {
        "training" => "fixtures/tune/predicated-update-training.tsv",
        "validation" => "fixtures/tune/predicated-update-validation.tsv",
        _ => return Err("predicated tuning split is not searchable".to_string()),
    };
    let entry = map
        .iter()
        .find(|entry| entry.logical_path == logical_path)
        .ok_or_else(|| "required predicated split is absent from CKTIMAP1".to_string())?;
    let parent = input_map_path
        .parent()
        .ok_or_else(|| "input map has no parent".to_string())?;
    let input = fs::read_to_string(parent.join("inputs").join(&entry.staged_basename))
        .map_err(|error| error.to_string())?;
    parse_predicated_input(&input, expected)?;

    let library = DynamicLibrary::open(&artifact)?;
    let kernel: PredicatedKernel = unsafe { library.symbol("floyd")? };
    let mut matrices = predicated_matrices(expected, iterations)?;
    let length = predicated_length(expected.n)?;
    for matrix in &mut matrices {
        unsafe { kernel(matrix.values.as_mut_ptr(), length, expected.n) };
    }
    validate_predicated_results(&matrices, expected)?;
    let result = matrices
        .last()
        .ok_or_else(|| "predicated tuning produced no result".to_string())?
        .canonical_result_bytes()?;
    let digest = result_digest(&case_id, &result)?;
    println!(
        "CKTUNE/1 {case_id} {seed} {iterations} {iterations} {}",
        hex(&digest)
    );
    Ok(())
}

fn run_predicated_profile(arguments: &[String]) -> Result<(), String> {
    if arguments.len() != 5 {
        return Err(
            "--ck-predicated-profile requires library, flush symbol, n, and seed".to_string(),
        );
    }
    let expected = predicated::TRAINING;
    let n = canonical_u32(&arguments[3], "predicated profile n")?;
    let seed = canonical_u64(&arguments[4], "predicated profile seed")?;
    require_frozen_coordinate(expected, n, seed)?;
    validate_flush_symbol(&arguments[2])?;
    predicated::checked_invocation_bytes(n, 1)?;

    let library = DynamicLibrary::open(PathBuf::from(&arguments[1]).as_path())?;
    let kernel: PredicatedKernel = unsafe { library.symbol("floyd")? };
    let flush: unsafe extern "C" fn() -> i32 = unsafe { library.symbol(&arguments[2])? };
    let mut matrix = predicated::PredicatedMatrix::generate(n, seed)?;
    let length = predicated_length(n)?;
    unsafe { kernel(matrix.values.as_mut_ptr(), length, n) };
    validate_predicated_result(&matrix, expected)?;
    let flush_status = unsafe { flush() };
    if flush_status != 0 {
        return Err("profile flush returned a nonzero status".to_string());
    }
    println!(
        "CKPREDPROFILE/1 {n} {seed} {} {flush_status}",
        expected.expected_digest
    );
    Ok(())
}

fn run_predicated_oracle(arguments: &[String]) -> Result<(), String> {
    if arguments.len() != 4 {
        return Err("--ck-predicated-oracle requires split, n, and seed".to_string());
    }
    let expected = predicated::split(&arguments[1])
        .ok_or_else(|| "unknown predicated oracle split".to_string())?;
    let n = canonical_u32(&arguments[2], "predicated oracle n")?;
    let seed = canonical_u64(&arguments[3], "predicated oracle seed")?;
    require_frozen_coordinate(expected, n, seed)?;
    let mut matrix = predicated::PredicatedMatrix::generate(n, seed)?;
    matrix.scalar_floyd()?;
    validate_predicated_result(&matrix, expected)?;
    println!(
        "CKPREDORACLE/1 {} {n} {seed} {}",
        expected.name, expected.expected_digest
    );
    Ok(())
}

fn run_predicated_performance(arguments: &[String]) -> Result<(), String> {
    if arguments.len() != 6 {
        return Err(
            "--ck-predicated-perf requires library, split, n, seed, and iterations".to_string(),
        );
    }
    let expected = match arguments[2].as_str() {
        "validation" => predicated::VALIDATION,
        "release-held-out" => predicated::RELEASE,
        _ => return Err("predicated performance split is not measurable".to_string()),
    };
    let n = canonical_u32(&arguments[3], "predicated performance n")?;
    let seed = canonical_u64(&arguments[4], "predicated performance seed")?;
    let iterations = canonical_u64(&arguments[5], "predicated performance iterations")?;
    require_frozen_coordinate(expected, n, seed)?;
    predicated::checked_invocation_bytes(n, iterations)?;

    let library = DynamicLibrary::open(PathBuf::from(&arguments[1]).as_path())?;
    let kernel: PredicatedKernel = unsafe { library.symbol("floyd")? };
    let mut matrices = predicated_matrices(expected, iterations)?;
    let length = predicated_length(n)?;
    let timer = PredicatedTimer::start()?;
    for matrix in &mut matrices {
        unsafe { kernel(matrix.values.as_mut_ptr(), length, n) };
    }
    let elapsed_ns = timer.elapsed_ns()?;
    validate_predicated_results(&matrices, expected)?;
    println!(
        "CKPREDPERF/1 {} {n} {seed} {iterations} {iterations} {elapsed_ns} {}",
        expected.name, expected.expected_digest
    );
    Ok(())
}

type PredicatedKernel = unsafe extern "C" fn(*mut f64, u32, u32);

fn predicated_matrices(
    split: predicated::FrozenSplit,
    iterations: u64,
) -> Result<Vec<predicated::PredicatedMatrix>, String> {
    let count = usize::try_from(iterations)
        .map_err(|_| "predicated iteration count is not representable".to_string())?;
    (0..count)
        .map(|_| predicated::PredicatedMatrix::generate(split.n, split.seed))
        .collect()
}

fn predicated_length(n: u32) -> Result<u32, String> {
    n.checked_mul(n)
        .ok_or_else(|| "predicated slice length overflow".to_string())
}

fn validate_predicated_results(
    matrices: &[predicated::PredicatedMatrix],
    split: predicated::FrozenSplit,
) -> Result<(), String> {
    for matrix in matrices {
        validate_predicated_result(matrix, split)?;
    }
    Ok(())
}

fn validate_predicated_result(
    matrix: &predicated::PredicatedMatrix,
    split: predicated::FrozenSplit,
) -> Result<(), String> {
    if predicated::hex(&matrix.result_digest()?) != split.expected_digest {
        return Err("predicated Floyd result does not match the frozen scalar oracle".to_string());
    }
    Ok(())
}

fn require_frozen_coordinate(
    split: predicated::FrozenSplit,
    n: u32,
    seed: u64,
) -> Result<(), String> {
    if n != split.n || seed != split.seed {
        return Err("predicated split coordinate does not match the frozen recipe".to_string());
    }
    Ok(())
}

fn parse_predicated_input(
    input: &str,
    expected: predicated::FrozenSplit,
) -> Result<(), String> {
    if input.contains('\r') || !input.ends_with('\n') {
        return Err("predicated input is not canonical LF text".to_string());
    }
    let lines = input.split_terminator('\n').collect::<Vec<_>>();
    let expected_id = match expected.name {
        "training" => "train-floyd-128",
        "validation" => "validate-floyd-256",
        "release-held-out" => "release-floyd-1024",
        _ => return Err("predicated input split is not frozen".to_string()),
    };
    let expected_header = format!("ckc-predicated-inputs\t1\t{}", expected.name);
    let expected_row = format!(
        "predicated-update\t{expected_id}\t{}\t{}",
        expected.n, expected.seed
    );
    if lines.as_slice() != [expected_header.as_str(), expected_row.as_str()] {
        return Err("predicated input bytes do not match the frozen split".to_string());
    }
    Ok(())
}

fn canonical_u32(value: &str, field: &str) -> Result<u32, String> {
    let parsed = value
        .parse::<u32>()
        .map_err(|_| format!("invalid {field}"))?;
    if parsed.to_string() != value {
        return Err(format!("noncanonical {field}"));
    }
    Ok(parsed)
}

fn canonical_u64(value: &str, field: &str) -> Result<u64, String> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| format!("invalid {field}"))?;
    if parsed.to_string() != value {
        return Err(format!("noncanonical {field}"));
    }
    Ok(parsed)
}

fn validate_flush_symbol(symbol: &str) -> Result<(), String> {
    let Some(digest) = symbol.strip_prefix("ck_profile_flush_") else {
        return Err("profile flush symbol has the wrong prefix".to_string());
    };
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err("profile flush symbol has a noncanonical digest".to_string());
    }
    Ok(())
}

struct PredicatedTimer {
    #[cfg(target_os = "linux")]
    started_ns: u64,
    #[cfg(not(target_os = "linux"))]
    started: Instant,
}

impl PredicatedTimer {
    fn start() -> Result<Self, String> {
        Ok(Self {
            #[cfg(target_os = "linux")]
            started_ns: monotonic_raw_ns()?,
            #[cfg(not(target_os = "linux"))]
            started: Instant::now(),
        })
    }

    fn elapsed_ns(self) -> Result<u64, String> {
        #[cfg(target_os = "linux")]
        {
            monotonic_raw_ns()?
                .checked_sub(self.started_ns)
                .filter(|elapsed| *elapsed > 0)
                .ok_or_else(|| "predicated performance timer did not advance".to_string())
        }
        #[cfg(not(target_os = "linux"))]
        {
            elapsed_ns(self.started)
        }
    }
}

#[cfg(target_os = "linux")]
fn monotonic_raw_ns() -> Result<u64, String> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC_RAW, &mut time) } != 0
        || time.tv_sec < 0
        || time.tv_nsec < 0
    {
        return Err("CLOCK_MONOTONIC_RAW failed".to_string());
    }
    u64::try_from(time.tv_sec)
        .ok()
        .and_then(|seconds| seconds.checked_mul(1_000_000_000))
        .and_then(|base| u64::try_from(time.tv_nsec).ok().and_then(|nanos| base.checked_add(nanos)))
        .ok_or_else(|| "CLOCK_MONOTONIC_RAW overflow".to_string())
}

fn run_tune() -> Result<(), String> {
    if env::var("CK_TUNE_PROTOCOL").as_deref() != Ok("1")
        || env::var("CK_TUNE_ARTIFACT_KIND").as_deref() != Ok("dynamic")
    {
        return Err("unsupported tuning protocol or artifact kind".to_string());
    }
    let case_id = required("CK_TUNE_CASE")?;
    let seed = required("CK_TUNE_SEED")?
        .parse::<u64>()
        .map_err(|_| "invalid CK_TUNE_SEED".to_string())?;
    let iterations = required("CK_TUNE_ITERATIONS")?
        .parse::<u64>()
        .map_err(|_| "invalid CK_TUNE_ITERATIONS".to_string())?;
    if iterations == 0 {
        return Err("CK_TUNE_ITERATIONS must be positive".to_string());
    }
    let artifact = PathBuf::from(required("CK_TUNE_ARTIFACT")?);
    let input_map_path = PathBuf::from(required("CK_TUNE_INPUT_MAP")?);
    let map = decode_input_map(&fs::read(&input_map_path).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    let parent = input_map_path
        .parent()
        .ok_or_else(|| "input map has no parent".to_string())?;
    let split = if case_id.ends_with(".search") {
        "fixtures/pgo/training.tsv"
    } else if case_id.ends_with(".validation") {
        "fixtures/pgo/held-out.tsv"
    } else {
        return Err("case id has no frozen tuning partition".to_string());
    };
    let entry = map
        .iter()
        .find(|entry| entry.logical_path == split)
        .ok_or_else(|| "required split is absent from CKTIMAP1".to_string())?;
    let input = fs::read_to_string(parent.join("inputs").join(&entry.staged_basename))
        .map_err(|error| error.to_string())?;
    let base_case = case_id
        .split_once('.')
        .map(|(name, _)| name)
        .ok_or_else(|| "invalid case id".to_string())?;
    let provenance_case = match base_case {
        "contract-noalias" => "memory-bound",
        "contract-fixed-length" => "call-constant-length",
        name => name,
    };
    let record = parse_record(&input, provenance_case, seed)?;
    let library = DynamicLibrary::open(&artifact)?;
    let (result, _) = unsafe { execute(&library, base_case, &record, iterations)? };
    let digest = result_digest(&case_id, &result)?;
    println!(
        "CKTUNE/1 {case_id} {seed} {iterations} {iterations} {}",
        hex(&digest)
    );
    Ok(())
}

fn run_performance(arguments: &[String]) -> Result<(), String> {
    if arguments.len() != 8 {
        return Err(
            "--ck-perf requires artifact, case, case-id, length, seed, parameter, and iterations"
                .to_string(),
        );
    }
    let artifact = PathBuf::from(&arguments[1]);
    let case = &arguments[2];
    let case_id = &arguments[3];
    if case_id != &format!("{case}.validation") && case_id != &format!("{case}.release") {
        return Err("performance case id does not match its case/split".to_string());
    }
    let record = InputRecord {
        length: arguments[4]
            .parse::<u32>()
            .map_err(|_| "invalid performance input length".to_string())?,
        salt: arguments[5]
            .parse::<u64>()
            .map_err(|_| "invalid performance input seed".to_string())?,
        parameter: arguments[6].clone(),
    };
    let iterations = arguments[7]
        .parse::<u64>()
        .map_err(|_| "invalid performance iteration count".to_string())?;
    if iterations == 0 {
        return Err("performance iterations must be positive".to_string());
    }
    let library = DynamicLibrary::open(&artifact)?;
    let (result, elapsed_ns) = unsafe { execute(&library, case, &record, iterations)? };
    let digest = result_digest(case_id, &result)?;
    println!(
        "CKPERF/1 {case_id} {} {iterations} {iterations} {elapsed_ns} {}",
        record.salt,
        hex(&digest),
    );
    Ok(())
}

fn required(name: &str) -> Result<String, String> {
    env::var(name).map_err(|_| format!("missing {name}"))
}

#[derive(Clone)]
struct InputRecord {
    length: u32,
    salt: u64,
    parameter: String,
}

fn parse_record(input: &str, case: &str, seed: u64) -> Result<InputRecord, String> {
    let mut found = None;
    for line in input.lines().filter(|line| !line.is_empty() && !line.starts_with('#')) {
        if line.starts_with("ckc-pgo-inputs\t") {
            continue;
        }
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() != 5 || fields[0] != case {
            continue;
        }
        let candidate_seed = fields[3]
            .parse::<u64>()
            .map_err(|_| "invalid input seed".to_string())?;
        if candidate_seed != seed {
            continue;
        }
        if found.is_some() {
            return Err("ambiguous input record".to_string());
        }
        found = Some(InputRecord {
            length: fields[2]
                .parse::<u32>()
                .map_err(|_| "invalid input length".to_string())?,
            salt: candidate_seed,
            parameter: fields[4].to_string(),
        });
    }
    found.ok_or_else(|| "input record not found".to_string())
}

unsafe fn execute(
    library: &DynamicLibrary,
    case: &str,
    record: &InputRecord,
    iterations: u64,
) -> Result<(Vec<u8>, u64), String> {
    match case {
        "branch-layout" => {
            type Kernel = unsafe extern "C" fn(*const u64, u32, u32, u64) -> u64;
            let kernel: Kernel = unsafe { library.symbol("kernel")? };
            let value = record
                .parameter
                .parse::<u64>()
                .map_err(|_| "invalid branch parameter".to_string())?;
            let input = vec![value; usize::try_from(record.length).map_err(|_| "length")?];
            let mut result = 0;
            let started = Instant::now();
            for _ in 0..iterations {
                result = unsafe {
                    kernel(input.as_ptr(), record.length, record.length, record.salt)
                };
            }
            Ok((result.to_le_bytes().to_vec(), elapsed_ns(started)?))
        }
        "call-constant-length" => {
            type Kernel = unsafe extern "C" fn(*const u32, u32, *mut u32, u32);
            let kernel: Kernel = unsafe { library.symbol("kernel")? };
            let value = record
                .parameter
                .parse::<u32>()
                .map_err(|_| "invalid fixed parameter".to_string())?;
            let input = vec![value; 4_000];
            let mut output = vec![0_u32; 4_000];
            let started = Instant::now();
            for _ in 0..iterations {
                unsafe { kernel(input.as_ptr(), 4_000, output.as_mut_ptr(), 4_000) };
            }
            Ok((u32_bytes(&output), elapsed_ns(started)?))
        }
        "trip-unroll-simd" | "contract-noalias" | "contract-fixed-length" => {
            type Kernel = unsafe extern "C" fn(*const u32, u32, *mut u32, u32, u32);
            let kernel: Kernel = unsafe { library.symbol("kernel")? };
            let length = if case == "contract-fixed-length" {
                16
            } else {
                record.length
            };
            let input = u32_input(length, record.salt);
            let mut output = vec![0_u32; usize::try_from(length).map_err(|_| "length")?];
            let started = Instant::now();
            for _ in 0..iterations {
                unsafe {
                    kernel(
                        input.as_ptr(),
                        length,
                        output.as_mut_ptr(),
                        length,
                        length,
                    )
                };
            }
            Ok((u32_bytes(&output), elapsed_ns(started)?))
        }
        "memory-bound" => {
            type Kernel =
                unsafe extern "C" fn(*const u32, u32, *const u32, u32, *mut u32, u32, u32);
            let kernel: Kernel = unsafe { library.symbol("kernel")? };
            let left = u32_input(record.length, record.salt);
            let right = u32_input(record.length, record.salt + 17);
            let mut output = vec![0_u32; usize::try_from(record.length).map_err(|_| "length")?];
            let started = Instant::now();
            for _ in 0..iterations {
                unsafe {
                    kernel(
                        left.as_ptr(),
                        record.length,
                        right.as_ptr(),
                        record.length,
                        output.as_mut_ptr(),
                        record.length,
                        record.length,
                    )
                };
            }
            Ok((u32_bytes(&output), elapsed_ns(started)?))
        }
        "compute-bound" => {
            type Kernel = unsafe extern "C" fn(*const f64, u32, *mut f64, u32, u32, f64);
            let kernel: Kernel = unsafe { library.symbol("kernel")? };
            let factor = record
                .parameter
                .parse::<f64>()
                .map_err(|_| "invalid f64 parameter".to_string())?;
            let input = (0..record.length)
                .map(|index| {
                    (f64::from(index) - f64::from(record.length) / 2.0 + record.salt as f64) / 16.0
                        + 0.25
                })
                .collect::<Vec<_>>();
            let mut output = vec![0_f64; usize::try_from(record.length).map_err(|_| "length")?];
            let started = Instant::now();
            for _ in 0..iterations {
                unsafe {
                    kernel(
                        input.as_ptr(),
                        record.length,
                        output.as_mut_ptr(),
                        record.length,
                        record.length,
                        factor,
                    )
                };
            }
            Ok((
                output
                    .iter()
                    .flat_map(|value| value.to_bits().to_le_bytes())
                    .collect(),
                elapsed_ns(started)?,
            ))
        }
        _ => Err("unknown frozen tuning case".to_string()),
    }
}

fn elapsed_ns(started: Instant) -> Result<u64, String> {
    u64::try_from(started.elapsed().as_nanos().max(1))
        .map_err(|_| "performance elapsed time overflow".to_string())
}

fn u32_input(length: u32, salt: u64) -> Vec<u32> {
    (0..length)
        .map(|index| {
            (((u64::from(index) + salt) * 2_654_435_761) % 1_000_002 + 1) as u32
        })
        .collect()
}

fn u32_bytes(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn result_digest(case_id: &str, result: &[u8]) -> Result<[u8; 32], String> {
    let mut digest = Sha256::new();
    digest.update(b"CK-TUNE-RESULT\0");
    digest.update(1u32.to_be_bytes());
    digest.update(
        u32::try_from(case_id.len())
            .map_err(|_| "case id length overflow")?
            .to_be_bytes(),
    );
    digest.update(case_id.as_bytes());
    digest.update(
        u64::try_from(result.len())
            .map_err(|_| "result length overflow")?
            .to_be_bytes(),
    );
    digest.update(result);
    Ok(digest.finalize().into())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

struct DynamicLibrary(*mut std::ffi::c_void);

impl DynamicLibrary {
    #[cfg(unix)]
    fn open(path: &std::path::Path) -> Result<Self, String> {
        use std::ffi::CString;
        let path = CString::new(path.as_os_str().as_encoded_bytes())
            .map_err(|_| "artifact path contains NUL".to_string())?;
        let handle = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if handle.is_null() {
            return Err("failed to load tuning artifact".to_string());
        }
        Ok(Self(handle))
    }

    #[cfg(windows)]
    fn open(path: &std::path::Path) -> Result<Self, String> {
        use std::os::windows::ffi::OsStrExt;
        let mut wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
        wide.push(0);
        let handle = unsafe { windows_sys::Win32::System::LibraryLoader::LoadLibraryW(wide.as_ptr()) };
        if handle.is_null() {
            return Err("failed to load tuning artifact".to_string());
        }
        Ok(Self(handle.cast()))
    }

    unsafe fn symbol<T: Copy>(&self, name: &str) -> Result<T, String> {
        #[cfg(unix)]
        let pointer = {
            let name = std::ffi::CString::new(name).map_err(|_| "symbol contains NUL")?;
            unsafe { libc::dlsym(self.0, name.as_ptr()) }
        };
        #[cfg(windows)]
        let pointer = {
            let mut name = name.as_bytes().to_vec();
            name.push(0);
            unsafe {
                windows_sys::Win32::System::LibraryLoader::GetProcAddress(
                    self.0.cast(),
                    name.as_ptr(),
                )
                .map_or(std::ptr::null_mut(), |symbol| symbol as *mut std::ffi::c_void)
            }
        };
        if pointer.is_null() || std::mem::size_of::<T>() != std::mem::size_of_val(&pointer) {
            return Err("required tuning symbol is absent".to_string());
        }
        Ok(unsafe { std::mem::transmute_copy(&pointer) })
    }
}

impl Drop for DynamicLibrary {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::dlclose(self.0);
        }
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::Foundation::FreeLibrary(self.0.cast());
        }
    }
}
