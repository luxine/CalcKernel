use std::{env, fs, path::PathBuf, process};

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
    let path = PathBuf::from(path);
    create_parent_dirs(&path)?;
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    let temp_path = PathBuf::from(format!("{}.tmp-{}-{millis}", path.display(), process::id()));
    if let Err(error) = fs::write(&temp_path, bytes) {
        let _ = fs::remove_file(&temp_path);
        return Err(format_open_file_error(&temp_path, error));
    }
    if let Err(error) = fs::rename(&temp_path, &path) {
        let _ = fs::remove_file(&temp_path);
        return Err(format_rename_file_error(&temp_path, &path, error));
    }
    Ok(())
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
