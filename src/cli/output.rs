use std::{
    env, fs,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
};

pub(super) fn read_text_lossy(path: &str) -> Result<String, String> {
    let path = absolutize(path);
    let bytes = fs::read(&path).map_err(|error| format_read_file_error(&path, error))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub(super) fn format_read_file_error(path: &std::path::Path, error: std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::NotFound => {
            format_node_open_error(path, "ENOENT", "no such file or directory")
        }
        std::io::ErrorKind::PermissionDenied => {
            format_node_open_error(path, "EACCES", "permission denied")
        }
        std::io::ErrorKind::IsADirectory => {
            "EISDIR: illegal operation on a directory, read".to_string()
        }
        _ => format!("{}: {error}", path.display()),
    }
}

pub(super) fn format_open_file_error(path: &std::path::Path, error: std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::NotFound => {
            format_node_open_error(path, "ENOENT", "no such file or directory")
        }
        std::io::ErrorKind::PermissionDenied => {
            format_node_open_error(path, "EACCES", "permission denied")
        }
        std::io::ErrorKind::IsADirectory => {
            format_node_open_error(path, "EISDIR", "illegal operation on a directory")
        }
        _ => error.to_string(),
    }
}

pub(super) fn format_node_open_error(path: &std::path::Path, code: &str, message: &str) -> String {
    format!("{code}: {message}, open '{}'", path.display())
}

pub(super) fn format_rename_file_error(
    from: &std::path::Path,
    to: &std::path::Path,
    error: std::io::Error,
) -> String {
    match error.kind() {
        std::io::ErrorKind::NotFound => format!(
            "ENOENT: no such file or directory, rename '{}' -> '{}'",
            from.display(),
            to.display()
        ),
        std::io::ErrorKind::PermissionDenied => format!(
            "EACCES: permission denied, rename '{}' -> '{}'",
            from.display(),
            to.display()
        ),
        std::io::ErrorKind::IsADirectory => format!(
            "EISDIR: illegal operation on a directory, rename '{}' -> '{}'",
            from.display(),
            to.display()
        ),
        _ => error.to_string(),
    }
}

pub(super) fn write_or_print(out: Option<&str>, text: &str, label: &str) -> Result<(), String> {
    if let Some(out) = out {
        write_text(out, text)?;
        println!("OK: emitted {label}");
        println!("Wrote {}", absolutize(out).display());
    } else {
        print!("{text}");
    }
    Ok(())
}

pub(super) fn write_or_print_single_line(
    out: Option<&str>,
    text: &str,
    label: &str,
) -> Result<(), String> {
    if let Some(out) = out {
        write_text_atomic(out, text)?;
        println!("OK: emitted {label} {out}");
    } else {
        print!("{text}");
    }
    Ok(())
}

pub(super) fn write_text(path: &str, text: &str) -> Result<(), String> {
    let path = PathBuf::from(path);
    create_parent_dirs(&path)?;
    fs::write(&path, text).map_err(|error| format_open_file_error(&path, error))
}

pub(super) fn write_text_atomic(path: &str, text: &str) -> Result<(), String> {
    write_bytes_atomic(path, text.as_bytes())
}

pub(super) fn write_bytes_atomic(path: &str, bytes: &[u8]) -> Result<(), String> {
    let mut transaction = OutputTransaction::new();
    transaction.stage(PathBuf::from(path), bytes)?;
    transaction.commit()
}

/// Same-filesystem, multi-file output transaction with best-effort rollback.
pub(super) struct OutputTransaction {
    entries: Vec<PendingOutput>,
}

struct PendingOutput {
    destination: PathBuf,
    staged: PathBuf,
    backup: Option<PathBuf>,
    committed: bool,
}

impl OutputTransaction {
    pub(super) const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub(super) fn stage(&mut self, destination: PathBuf, bytes: &[u8]) -> Result<(), String> {
        if self
            .entries
            .iter()
            .any(|entry| entry.destination == destination)
        {
            return Err(format!(
                "duplicate output destination '{}': transaction rejected",
                destination.display()
            ));
        }
        create_parent_dirs(&destination)?;
        reject_symlink(&destination)?;
        let staged = unique_sibling(&destination, "stage")?;
        let write_result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&staged)
                .map_err(|error| format_open_file_error(&staged, error))?;
            file.write_all(bytes)
                .and_then(|()| file.sync_all())
                .map_err(|error| format_open_file_error(&staged, error))
        })();
        if let Err(error) = write_result {
            let _ = fs::remove_file(&staged);
            return Err(error);
        }
        self.entries.push(PendingOutput {
            destination,
            staged,
            backup: None,
            committed: false,
        });
        Ok(())
    }

    pub(super) fn commit(mut self) -> Result<(), String> {
        self.commit_inner(CommitFailure::Never)
    }

    fn commit_inner(&mut self, failure: CommitFailure) -> Result<(), String> {
        if failure == CommitFailure::BeforeCommit {
            return Err("injected pre-commit output failure".to_string());
        }
        let mut replacements = 0usize;
        for index in 0..self.entries.len() {
            let destination = self.entries[index].destination.clone();
            if let Err(error) = reject_symlink(&destination) {
                return self.rollback(error);
            }
            if destination.exists() {
                let backup = match unique_sibling(&destination, "backup") {
                    Ok(path) => path,
                    Err(error) => return self.rollback(error),
                };
                if let Err(error) = fs::rename(&destination, &backup) {
                    return self.rollback(format_rename_file_error(&destination, &backup, error));
                }
                self.entries[index].backup = Some(backup);
            }
            let staged = self.entries[index].staged.clone();
            if let Err(error) = fs::rename(&staged, &destination) {
                return self.rollback(format_rename_file_error(&staged, &destination, error));
            }
            self.entries[index].committed = true;
            replacements += 1;
            if failure == CommitFailure::AfterReplacement(replacements) {
                return self.rollback(format!(
                    "injected output failure after {replacements} replacement(s)"
                ));
            }
        }
        for entry in &mut self.entries {
            if let Some(backup) = entry.backup.take() {
                fs::remove_file(&backup).map_err(|error| format_open_file_error(&backup, error))?;
            }
        }
        Ok(())
    }

    fn rollback(&mut self, cause: String) -> Result<(), String> {
        let mut failures = Vec::new();
        for entry in self.entries.iter_mut().rev() {
            if entry.committed {
                if let Err(error) = fs::remove_file(&entry.destination)
                    && error.kind() != std::io::ErrorKind::NotFound
                {
                    failures.push(format!(
                        "remove new '{}': {error}",
                        entry.destination.display()
                    ));
                }
                entry.committed = false;
            }
            if let Some(backup) = entry.backup.take()
                && let Err(error) = fs::rename(&backup, &entry.destination)
            {
                failures.push(format!(
                    "restore '{}' from '{}': {error}",
                    entry.destination.display(),
                    backup.display()
                ));
                entry.backup = Some(backup);
            }
        }
        if failures.is_empty() {
            Err(format!("{cause}; output transaction rolled back"))
        } else {
            Err(format!(
                "{cause}; output rollback failed: {}",
                failures.join("; ")
            ))
        }
    }

    #[cfg(test)]
    fn commit_for_test(mut self, failure: CommitFailure) -> Result<(), String> {
        self.commit_inner(failure)
    }
}

impl Drop for OutputTransaction {
    fn drop(&mut self) {
        for entry in &self.entries {
            let _ = fs::remove_file(&entry.staged);
            if let Some(backup) = &entry.backup {
                let _ = fs::rename(backup, &entry.destination);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommitFailure {
    Never,
    BeforeCommit,
    AfterReplacement(usize),
}

fn reject_symlink(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "refusing to replace symlink output '{}'",
            path.display()
        )),
        Ok(metadata) if !metadata.is_file() => Err(format!(
            "refusing to replace non-file output '{}'",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format_open_file_error(path, error)),
    }
}

fn unique_sibling(destination: &Path, role: &str) -> Result<PathBuf, String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let name = destination
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("output");
    for _ in 0..128 {
        let serial = NEXT.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(".{name}.ckc-txn-{}-{serial}-{role}", process::id()));
        match fs::symlink_metadata(&candidate) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(candidate),
            Ok(_) => continue,
            Err(error) => return Err(format_open_file_error(&candidate, error)),
        }
    }
    Err(format!(
        "could not allocate transaction staging path beside '{}'",
        destination.display()
    ))
}

pub(super) fn create_parent_dirs(path: &std::path::Path) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| format_make_directory_error(parent, error))?;
    }
    Ok(())
}

pub(super) fn format_make_directory_error(path: &std::path::Path, error: std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::AlreadyExists => {
            format!("EEXIST: file already exists, mkdir '{}'", path.display())
        }
        std::io::ErrorKind::PermissionDenied => {
            format!("EACCES: permission denied, mkdir '{}'", path.display())
        }
        std::io::ErrorKind::NotADirectory => {
            format!("ENOTDIR: not a directory, mkdir '{}'", path.display())
        }
        _ => error.to_string(),
    }
}

pub(super) fn absolutize(path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

pub(super) fn default_header_file_for_c_output(c_file: &str) -> String {
    let path = PathBuf::from(c_file);
    let file_name = path
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .unwrap_or(c_file)
        .to_string();
    let base_name = file_name
        .rfind('.')
        .filter(|index| *index > 0)
        .map_or(file_name.as_str(), |index| &file_name[..index]);
    let mut header = path;
    header.set_file_name(format!("{base_name}.h"));
    header.to_string_lossy().into_owned()
}

pub(super) fn header_include_name(header: &str) -> Result<String, String> {
    PathBuf::from(header)
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("Invalid header path '{header}'."))
}

#[cfg(test)]
mod transaction_tests {
    use super::*;

    #[test]
    fn pre_commit_failure_keeps_every_destination_and_cleans_stages() {
        let root = test_root("pre-commit");
        let first = root.join("first.bin");
        let second = root.join("second.bin");
        fs::write(&first, b"old-first").expect("seed first");
        fs::write(&second, b"old-second").expect("seed second");
        let mut transaction = OutputTransaction::new();
        transaction
            .stage(first.clone(), b"new-first")
            .expect("stage first");
        transaction
            .stage(second.clone(), b"new-second")
            .expect("stage second");
        let error = transaction
            .commit_for_test(CommitFailure::BeforeCommit)
            .expect_err("inject pre-commit failure");
        assert!(error.contains("pre-commit"));
        assert_eq!(fs::read(&first).expect("read first"), b"old-first");
        assert_eq!(fs::read(&second).expect("read second"), b"old-second");
        assert_no_transaction_files(&root);
        fs::remove_dir_all(root).expect("remove root");
    }

    #[test]
    fn commit_failure_rolls_back_every_replaced_destination() {
        let root = test_root("rollback");
        let first = root.join("first.bin");
        let second = root.join("second.bin");
        fs::write(&first, b"old-first").expect("seed first");
        fs::write(&second, b"old-second").expect("seed second");
        let mut transaction = OutputTransaction::new();
        transaction
            .stage(first.clone(), b"new-first")
            .expect("stage first");
        transaction
            .stage(second.clone(), b"new-second")
            .expect("stage second");
        let error = transaction
            .commit_for_test(CommitFailure::AfterReplacement(1))
            .expect_err("inject commit failure");
        assert!(error.contains("rolled back"));
        assert_eq!(fs::read(&first).expect("read first"), b"old-first");
        assert_eq!(fs::read(&second).expect("read second"), b"old-second");
        assert_no_transaction_files(&root);
        fs::remove_dir_all(root).expect("remove root");
    }

    #[test]
    fn successful_multi_output_commit_replaces_all_and_cleans_backups() {
        let root = test_root("success");
        let first = root.join("first.bin");
        let second = root.join("second.bin");
        fs::write(&first, b"old-first").expect("seed first");
        let mut transaction = OutputTransaction::new();
        transaction
            .stage(first.clone(), b"new-first")
            .expect("stage first");
        transaction
            .stage(second.clone(), b"new-second")
            .expect("stage second");
        transaction.commit().expect("commit transaction");
        assert_eq!(fs::read(&first).expect("read first"), b"new-first");
        assert_eq!(fs::read(&second).expect("read second"), b"new-second");
        assert_no_transaction_files(&root);
        fs::remove_dir_all(root).expect("remove root");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_destination_is_rejected_without_touching_target() {
        use std::os::unix::fs::symlink;
        let root = test_root("symlink");
        let target = root.join("target.bin");
        let link = root.join("output.bin");
        fs::write(&target, b"target").expect("seed target");
        symlink(&target, &link).expect("create link");
        let mut transaction = OutputTransaction::new();
        let error = transaction
            .stage(link, b"replacement")
            .expect_err("reject symlink");
        assert!(error.contains("symlink"));
        assert_eq!(fs::read(&target).expect("read target"), b"target");
        fs::remove_dir_all(root).expect("remove root");
    }

    fn test_root(label: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = env::temp_dir().join(format!(
            "ckc-output-test-{}-{label}-{}",
            process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("create test root");
        path
    }

    fn assert_no_transaction_files(root: &Path) {
        let names = fs::read_dir(root)
            .expect("read root")
            .map(|entry| {
                entry
                    .expect("read entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        assert!(
            names.iter().all(|name| !name.contains(".ckc-txn-")),
            "transaction debris: {names:?}"
        );
    }
}
