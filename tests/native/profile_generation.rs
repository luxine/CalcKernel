use std::{ffi::OsString, fs, process::Command};

use calckernel::{
    CkProfileCounter, NativeArtifactKind, NativeArtifactPaths, NativePlatform, parse_profile_shard,
};

use super::support::temp::unique_id;

fn os(value: impl AsRef<std::ffi::OsStr>) -> OsString {
    value.as_ref().to_os_string()
}

fn fixture(source: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let root = std::env::current_dir()
        .expect("current directory")
        .join("target/profile-generation-tests")
        .join(unique_id().to_string());
    fs::create_dir_all(&root).expect("create generation fixture");
    let input = root.join("input.ck");
    fs::write(&input, source).expect("write generation fixture");
    (root, input)
}

fn generation_executable(root: &std::path::Path, input: &std::path::Path) -> std::path::PathBuf {
    let collection = root.join("shards");
    fs::create_dir(&collection).expect("create shard collection");
    let output = root.join("program");
    let build = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .args([
            os("build"),
            os(input),
            os("--kind"),
            os("executable"),
            os("--out"),
            os(&output),
            os("--pgo-generate"),
            os(&collection),
            os("-O3"),
        ])
        .env("PATH", "")
        .output()
        .expect("build profile generation executable");
    assert!(
        build.status.success(),
        "generation build failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    NativeArtifactPaths::new(
        NativePlatform::host(),
        NativeArtifactKind::Executable,
        &output,
    )
    .primary
}

fn completed_shards(collection: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut paths = fs::read_dir(collection)
        .expect("read shard collection")
        .map(|entry| entry.expect("read shard entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "ckprof-part")
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn build_library(
    root: &std::path::Path,
    input: &std::path::Path,
    generation: bool,
) -> NativeArtifactPaths {
    let output = root.join(if generation { "generation" } else { "ordinary" });
    let collection = root.join("library-shards");
    if generation {
        fs::create_dir(&collection).expect("create library shard collection");
    }
    let mut arguments = vec![
        os("build"),
        os(input),
        os("--kind"),
        os("dynamic"),
        os("--out"),
        os(&output),
        os("-O3"),
    ];
    if generation {
        arguments.extend([os("--pgo-generate"), os(&collection)]);
    }
    let build = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .args(arguments)
        .env("PATH", "")
        .output()
        .expect("build profile library");
    assert!(
        build.status.success(),
        "library build failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    NativeArtifactPaths::new(NativePlatform::host(), NativeArtifactKind::Dynamic, &output)
}

fn flush_symbol(header: &str) -> String {
    let marker = "ck_profile_flush_";
    let start = header.find(marker).expect("generation flush declaration");
    let suffix = &header[start..];
    let end = suffix.find('(').expect("flush declaration opening paren");
    let symbol = &suffix[..end];
    assert_eq!(symbol.len(), marker.len() + 64);
    assert!(
        symbol[marker.len()..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
    symbol.to_string()
}

#[test]
fn profile_generation_executable_normal_zero_return_should_publish_one_valid_shard() {
    let (root, input) =
        fixture("fn main() -> i32 { let i: u32 = 0; while i < 5 { i = i + 1; } return 0; }");
    let executable = generation_executable(&root, &input);
    let run = Command::new(executable)
        .env("PATH", "")
        .output()
        .expect("run profile generation executable");
    assert_eq!(run.status.code(), Some(0), "{:?}", run.status);

    let shards = completed_shards(&root.join("shards"));
    assert_eq!(shards.len(), 1, "completed shards: {shards:?}");
    let shard = parse_profile_shard(&fs::read(&shards[0]).expect("read completed shard"))
        .expect("parse completed shard");
    assert_ne!(shard.run_id, [0; 16]);
    assert!(
        shard.counters.iter().any(|record| match &record.counter {
            CkProfileCounter::Scalar(value) => *value != 0,
            CkProfileCounter::Histogram { buckets, .. } => buckets.iter().any(|value| *value != 0),
            CkProfileCounter::CandidateConstant {
                candidates, other, ..
            } => *other != 0 || candidates.iter().any(|value| *value != 0),
        }),
        "at least one generated counter must be observed"
    );
    fs::remove_dir_all(root).expect("remove generation fixture");
}

#[test]
fn profile_generation_executable_nonzero_return_should_not_publish_a_shard() {
    let (root, input) = fixture("fn main() -> i32 { return 7; }");
    let executable = generation_executable(&root, &input);
    let run = Command::new(executable)
        .env("PATH", "")
        .output()
        .expect("run nonzero profile generation executable");
    assert_eq!(run.status.code(), Some(7), "{:?}", run.status);
    assert!(completed_shards(&root.join("shards")).is_empty());
    fs::remove_dir_all(root).expect("remove generation fixture");
}

#[cfg(unix)]
#[test]
fn profile_generation_library_flush_should_be_exactly_once_concurrent_and_sticky() {
    use std::ffi::CString;

    let (root, input) = fixture("export fn answer() -> i32 { return 42; }");
    let paths = build_library(&root, &input, true);
    let header = fs::read_to_string(paths.header.expect("generation header"))
        .expect("read generation header");
    let flush_name = flush_symbol(&header);
    unsafe {
        let path_text = paths.primary.to_string_lossy().into_owned();
        let path = CString::new(path_text.as_bytes()).expect("library path");
        let handle = dlopen(path.as_ptr(), 2);
        assert!(!handle.is_null(), "dlopen generation library");
        let answer_name = CString::new("answer").expect("answer symbol");
        let answer_address = dlsym(handle, answer_name.as_ptr());
        assert!(!answer_address.is_null(), "dlsym answer");
        let answer: unsafe extern "C" fn() -> i32 = std::mem::transmute(answer_address);
        assert_eq!(answer(), 42);

        let flush_name = CString::new(flush_name).expect("flush symbol");
        let flush_address = dlsym(handle, flush_name.as_ptr());
        assert!(!flush_address.is_null(), "dlsym flush");
        let flush: unsafe extern "C" fn() -> i32 = std::mem::transmute(flush_address);
        let statuses = std::thread::scope(|scope| {
            let handles = (0..8)
                .map(|_| scope.spawn(move || flush()))
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|thread| thread.join().expect("flush thread"))
                .collect::<Vec<_>>()
        });
        assert!(statuses.iter().all(|status| *status == 0), "{statuses:?}");
        assert_eq!(flush(), 0, "repeat flush must return the sticky status");
        assert_eq!(dlclose(handle), 0);
    }
    let shards = completed_shards(&root.join("library-shards"));
    assert_eq!(shards.len(), 1, "completed shards: {shards:?}");
    parse_profile_shard(&fs::read(&shards[0]).expect("read library shard"))
        .expect("parse library shard");
    fs::remove_dir_all(root).expect("remove generation fixture");
}

#[test]
fn profile_generation_ordinary_library_should_contain_no_generation_control_or_runtime() {
    let (root, input) = fixture("export fn answer() -> i32 { return 42; }");
    let paths = build_library(&root, &input, false);
    let header =
        fs::read_to_string(paths.header.expect("ordinary header")).expect("read ordinary header");
    let bytes = fs::read(paths.primary).expect("read ordinary library");
    assert!(!header.contains("ck_profile_flush_"));
    for needle in [b"ck_profile_flush_".as_slice(), b"__ck_profile_".as_slice()] {
        assert!(
            !bytes.windows(needle.len()).any(|window| window == needle),
            "ordinary artifact contains {}",
            String::from_utf8_lossy(needle)
        );
    }
    fs::remove_dir_all(root).expect("remove generation fixture");
}

#[cfg(unix)]
unsafe extern "C" {
    fn dlopen(path: *const std::ffi::c_char, mode: std::ffi::c_int) -> *mut std::ffi::c_void;
    fn dlsym(
        handle: *mut std::ffi::c_void,
        symbol: *const std::ffi::c_char,
    ) -> *mut std::ffi::c_void;
    fn dlclose(handle: *mut std::ffi::c_void) -> std::ffi::c_int;
}
