use std::{fs, path::Path, time::SystemTime};

pub(super) const DEFAULT_SOFT_LIMIT: u64 = 1024 * 1024 * 1024;

#[cfg(any(test, not(unix)))]
pub(super) fn enforce_soft_limit(root: &Path, limit: u64) -> std::io::Result<()> {
    enforce_soft_limit_with(root, limit, |path| fs::remove_file(path))
}

pub(super) fn enforce_soft_limit_with(
    root: &Path,
    limit: u64,
    mut remove: impl FnMut(&Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let mut entries = Vec::new();
    let mut total = 0u64;
    for candidate in fs::read_dir(root)? {
        let candidate = candidate?;
        let name = candidate.file_name().to_string_lossy().into_owned();
        if !is_cache_key(&name) {
            continue;
        }
        let metadata = fs::symlink_metadata(candidate.path())?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            continue;
        }
        total = total.saturating_add(metadata.len());
        entries.push((
            metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            name,
            candidate.path(),
            metadata.len(),
        ));
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    for (_, _, path, length) in entries {
        if total <= limit {
            break;
        }
        match remove(&path) {
            Ok(()) => total = total.saturating_sub(length),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                total = total.saturating_sub(length);
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn is_cache_key(name: &str) -> bool {
    name.len() == 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use std::{fs, thread, time::Duration};

    use super::enforce_soft_limit;

    #[test]
    fn eviction_should_remove_oldest_entry_with_deterministic_name_tiebreak() {
        let root = std::env::temp_dir().join(format!("ckc-evict-{}", std::process::id()));
        fs::create_dir_all(&root).expect("create eviction root");
        let first = "11".repeat(32);
        let second = "22".repeat(32);
        fs::write(root.join(&first), [1u8; 8]).expect("write first entry");
        thread::sleep(Duration::from_millis(5));
        fs::write(root.join(&second), [2u8; 8]).expect("write second entry");
        enforce_soft_limit(&root, 8).expect("evict to limit");
        assert!(!root.join(first).exists());
        assert!(root.join(second).is_file());
    }
}
