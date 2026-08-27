use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::Duration,
};

use super::support::temp::unique_id;

struct Fixture {
    root: PathBuf,
    source: PathBuf,
}

impl Fixture {
    fn new(source: &str) -> Self {
        let root = std::env::temp_dir().join(format!("ckc-cache-cli-{}", unique_id()));
        fs::create_dir_all(&root).expect("create cache fixture root");
        let source_path = root.join("program.ck");
        fs::write(&source_path, source).expect("write cache fixture source");
        Self {
            root,
            source: source_path,
        }
    }

    fn cache_root(&self) -> PathBuf {
        #[cfg(target_os = "macos")]
        return self.root.join("home/Library/Caches/ckc");
        #[cfg(target_os = "linux")]
        return self.root.join("xdg/ckc");
        #[cfg(target_os = "windows")]
        return self.root.join("local-app-data/CalcKernel/cache");
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ckc"));
        command
            .env("PATH", "")
            .env_remove("XDG_CACHE_HOME")
            .env_remove("HOME")
            .env_remove("LOCALAPPDATA");
        #[cfg(target_os = "macos")]
        command.env("HOME", self.root.join("home"));
        #[cfg(target_os = "linux")]
        command.env("XDG_CACHE_HOME", self.root.join("xdg"));
        #[cfg(target_os = "windows")]
        command.env("LOCALAPPDATA", self.root.join("local-app-data"));
        command
    }

    fn run(&self, extra: &[&str]) -> Output {
        let mut command = self.command();
        command.arg("run").arg(&self.source).args(extra);
        command.output().expect("run ckc cache fixture")
    }

    fn spawn(&self) -> Child {
        let mut command = self.command();
        command
            .arg("run")
            .arg(&self.source)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command.spawn().expect("spawn ckc cache fixture")
    }
}

fn cache_entries(root: &Path) -> Vec<PathBuf> {
    let mut entries = fs::read_dir(root)
        .expect("read cache root")
        .map(|entry| entry.expect("read cache entry").path())
        .filter(|path| {
            path.file_name().is_some_and(|name| {
                let name = name.to_string_lossy();
                name.len() == 64
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            })
        })
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

fn assert_successful_program(output: &Output) {
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, b"42\n");
    assert_eq!(output.stderr, b"");
}

#[test]
fn cold_miss_and_warm_hit_should_be_stable_across_processes() {
    let fixture = Fixture::new("fn main() -> void { print_i32(42); print_newline(); }");
    assert_successful_program(&fixture.run(&[]));
    let entries = cache_entries(&fixture.cache_root());
    assert_eq!(entries.len(), 1);
    let before = fs::read(&entries[0]).expect("read cold cache entry");
    let before_modified = fs::metadata(&entries[0])
        .expect("cold cache metadata")
        .modified()
        .expect("cold modified time");

    thread::sleep(Duration::from_millis(25));
    assert_successful_program(&fixture.run(&[]));

    assert_eq!(cache_entries(&fixture.cache_root()), entries);
    assert_eq!(fs::read(&entries[0]).expect("read warm entry"), before);
    assert!(
        fs::metadata(&entries[0])
            .expect("warm cache metadata")
            .modified()
            .expect("warm modified time")
            > before_modified
    );
}

#[test]
fn cache_key_should_change_for_source_modes_and_optimization_but_not_path() {
    let fixture = Fixture::new("fn main() -> i32 { print_i32(42); print_newline(); return 0; }");
    assert_successful_program(&fixture.run(&[]));
    let identical = fixture.root.join("identical.ck");
    fs::copy(&fixture.source, &identical).expect("copy identical source");
    let mut identical_command = fixture.command();
    let identical_output = identical_command
        .args(["run", identical.to_str().expect("UTF-8 source path")])
        .output()
        .expect("run identical source at another path");
    assert_successful_program(&identical_output);
    assert_eq!(cache_entries(&fixture.cache_root()).len(), 1);

    assert_successful_program(&fixture.run(&["-O2"]));
    assert_successful_program(&fixture.run(&["--overflow", "checked"]));
    fs::write(
        &fixture.source,
        "fn main() -> i32 { print_i32(42); print_newline(); return 1; }",
    )
    .expect("mutate cache fixture source");
    let changed_source = fixture.run(&[]);
    assert_eq!(changed_source.status.code(), Some(1));
    assert_eq!(cache_entries(&fixture.cache_root()).len(), 4);
}

#[test]
fn no_cache_should_bypass_reads_writes_and_corrupt_entries() {
    let fixture = Fixture::new("fn main() -> void { print_i32(42); print_newline(); }");
    assert_successful_program(&fixture.run(&["--no-cache"]));
    assert!(!fixture.cache_root().exists());

    assert_successful_program(&fixture.run(&[]));
    let entry = cache_entries(&fixture.cache_root()).remove(0);
    fs::write(&entry, b"corrupt").expect("corrupt cache entry");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&entry, fs::Permissions::from_mode(0o600))
            .expect("protect corrupt cache entry");
    }
    assert_successful_program(&fixture.run(&["--no-cache"]));
    assert_eq!(fs::read(&entry).expect("read bypassed entry"), b"corrupt");
    assert_successful_program(&fixture.run(&[]));
    assert_ne!(fs::read(&entry).expect("read repaired entry"), b"corrupt");
}

#[cfg(unix)]
#[test]
fn unsafe_permissions_and_symlinks_should_degrade_to_cache_misses() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let fixture = Fixture::new("fn main() -> void { print_i32(42); print_newline(); }");
    fs::create_dir_all(fixture.cache_root()).expect("create unsafe cache root");
    fs::set_permissions(fixture.cache_root(), fs::Permissions::from_mode(0o777))
        .expect("make cache root unsafe");
    assert_successful_program(&fixture.run(&[]));
    assert!(cache_entries(&fixture.cache_root()).is_empty());

    fs::set_permissions(fixture.cache_root(), fs::Permissions::from_mode(0o700))
        .expect("restore cache root safety");
    assert_successful_program(&fixture.run(&[]));
    let entry = cache_entries(&fixture.cache_root()).remove(0);
    fs::remove_file(&entry).expect("remove cache entry before linking");
    let outside = fixture.root.join("outside");
    fs::write(&outside, b"outside").expect("write outside target");
    symlink(&outside, &entry).expect("link cache entry outside");
    assert_successful_program(&fixture.run(&[]));
    assert_eq!(fs::read(outside).expect("read outside target"), b"outside");
}

#[test]
fn concurrent_writers_should_converge_without_temporary_files() {
    let fixture = Fixture::new("fn main() -> i32 { print_i32(42); print_newline(); return 0; }");
    let children = (0..8).map(|_| fixture.spawn()).collect::<Vec<_>>();
    for child in children {
        assert_successful_program(&child.wait_with_output().expect("wait cache writer"));
    }
    let all = fs::read_dir(fixture.cache_root())
        .expect("read converged cache root")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect converged cache entries");
    assert_eq!(all.len(), 1, "unexpected temporary cache files");
    assert_eq!(cache_entries(&fixture.cache_root()).len(), 1);
    assert_successful_program(&fixture.run(&[]));
}

#[test]
fn cache_clean_should_remove_only_the_resolved_ckc_cache_root() {
    let fixture = Fixture::new("fn main() -> i32 { print_i32(42); print_newline(); return 0; }");
    assert_successful_program(&fixture.run(&[]));
    let sibling = fixture
        .cache_root()
        .parent()
        .expect("cache parent")
        .join("sibling-marker");
    fs::write(&sibling, b"keep").expect("write sibling marker");

    let output = fixture
        .command()
        .args(["cache", "clean"])
        .output()
        .expect("clean cache");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, b"OK: native cache cleaned\n");
    assert_eq!(output.stderr, b"");
    assert!(!fixture.cache_root().exists());
    assert_eq!(fs::read(sibling).expect("read sibling marker"), b"keep");
}

#[test]
fn missing_required_cache_base_should_disable_cache_without_failing_run() {
    let fixture = Fixture::new("fn main() -> void { print_i32(42); print_newline(); }");
    let output = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .args(["run", fixture.source.to_str().expect("UTF-8 fixture")])
        .env("PATH", "")
        .env_remove("HOME")
        .env_remove("XDG_CACHE_HOME")
        .env_remove("LOCALAPPDATA")
        .output()
        .expect("run without cache base");
    assert_successful_program(&output);
}
