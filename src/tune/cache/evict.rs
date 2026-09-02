use std::{fs, path::Path, time::SystemTime};

pub(super) fn enforce(root: &Path, limit: u64) -> Result<(), String> {
    let mut entries = Vec::new();
    let mut total = 0u64;
    for domain in ["compile", "measurement", "decision"] {
        let directory = root.join(domain);
        for entry in fs::read_dir(&directory)
            .map_err(|error| format!("read tuning cache {}: {error}", directory.display()))?
        {
            let entry = entry.map_err(|error| format!("read tuning cache entry: {error}"))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !valid_entry_name(&name) {
                continue;
            }
            let metadata = fs::symlink_metadata(entry.path())
                .map_err(|error| format!("inspect tuning cache entry: {error}"))?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                continue;
            }
            total = total.saturating_add(metadata.len());
            entries.push((
                metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                domain.to_string(),
                name,
                entry.path(),
                metadata.len(),
            ));
        }
    }
    entries.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });
    for (_, _, _, path, length) in entries {
        if total <= limit {
            break;
        }
        match fs::remove_file(&path) {
            Ok(()) => total = total.saturating_sub(length),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                total = total.saturating_sub(length);
            }
            Err(error) => {
                return Err(format!(
                    "evict tuning cache entry {}: {error}",
                    path.display()
                ));
            }
        }
    }
    if total > limit {
        return Err("tuning cache cannot satisfy hard size limit".to_string());
    }
    Ok(())
}

fn valid_entry_name(name: &str) -> bool {
    name.len() == 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
