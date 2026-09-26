use std::{
    collections::{HashMap, HashSet},
    fs::{self, OpenOptions},
    io::Read,
    path::{Component, Path, PathBuf},
};

use calckernel::{Declaration, SourceFile, parse};
use serde_json::{Value, json};

const MAX_ROOTS: usize = 16;
const MAX_ROOT_CANDIDATES: usize = 64;
const MAX_DIRECTORY_ENTRIES: usize = 20_000;
const MAX_CK_FILES: usize = 512;
const MAX_TOTAL_BYTES: u64 = 16 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_RESULTS: usize = 1_000;

/// Results and whether the bounded scan omitted any otherwise eligible data.
pub(super) struct WorkspaceSymbols {
    pub(super) items: Vec<Value>,
    pub(super) truncated: bool,
}

/// Workspace URI identity survives symlink retargeting while `path` is canonical for scanning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkspaceRoot {
    pub(super) identity: PathBuf,
    pub(super) path: PathBuf,
}

/// Convert an absolute local path to a canonical, percent-encoded file URI.
pub(super) fn path_to_file_uri(path: &Path) -> Option<String> {
    if !path.is_absolute() {
        return None;
    }
    let path = path.to_str()?;

    #[cfg(unix)]
    {
        Some(format!("file://{}", percent_encode_path(path.as_bytes())))
    }
    #[cfg(windows)]
    {
        let path = path.replace('\\', "/");
        let path = if path
            .get(..8)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("//?/UNC/"))
        {
            format!("//{}", &path[8..])
        } else if path.starts_with("//?/") {
            path[4..].to_owned()
        } else {
            path
        };
        if let Some(unc) = path.strip_prefix("//") {
            let (host, share_path) = unc.split_once('/')?;
            if host.is_empty() || share_path.is_empty() {
                return None;
            }
            Some(format!(
                "file://{}/{}",
                percent_encode_authority(host.as_bytes()),
                percent_encode_path(share_path.as_bytes())
            ))
        } else if path.as_bytes().get(1) == Some(&b':') && path.as_bytes()[0].is_ascii_alphabetic()
        {
            Some(format!("file:///{}", percent_encode_path(path.as_bytes())))
        } else {
            None
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        None
    }
}

/// Parse a strict absolute local file URI. No query or fragment is permitted.
pub(super) fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    if uri.contains(['?', '#']) || !uri.starts_with("file://") {
        return None;
    }
    let address = uri.strip_prefix("file://")?;
    if address.contains('\\') || address.bytes().any(|byte| byte.is_ascii_control()) {
        return None;
    }
    let (authority, encoded_path) = if address.starts_with('/') {
        ("", address)
    } else {
        let (authority, _path) = address.split_once('/')?;
        if authority.is_empty() {
            return None;
        }
        (authority, &address[authority.len()..])
    };
    let decoded_path = percent_decode_path(encoded_path)?;
    if decoded_path.contains('\0') {
        return None;
    }

    #[cfg(unix)]
    {
        if !authority.is_empty() && !authority.eq_ignore_ascii_case("localhost") {
            return None;
        }
        let path = PathBuf::from(decoded_path);
        path.is_absolute().then_some(path)
    }
    #[cfg(windows)]
    {
        if authority.is_empty() || authority.eq_ignore_ascii_case("localhost") {
            let bytes = decoded_path.as_bytes();
            if bytes.len() < 4
                || bytes[0] != b'/'
                || !bytes[1].is_ascii_alphabetic()
                || bytes[2] != b':'
                || bytes[3] != b'/'
            {
                return None;
            }
            return Some(PathBuf::from(decoded_path[1..].replace('/', "\\")));
        }
        let authority = percent_decode_authority(authority)?;
        if authority.contains(['@', ':', '/', '\\', '\0']) || !decoded_path.starts_with('/') {
            return None;
        }
        let share_path = decoded_path.trim_start_matches('/');
        if share_path.is_empty() {
            return None;
        }
        Some(PathBuf::from(format!(
            "\\\\{}\\{}",
            authority,
            share_path.replace('/', "\\")
        )))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (authority, decoded_path);
        None
    }
}

/// Resolve initial workspace roots, preferring `workspaceFolders` when present.
pub(super) fn roots_from_initialize(params: &Value) -> Vec<WorkspaceRoot> {
    if let Some(Value::Array(folders)) = params.get("workspaceFolders") {
        return canonical_roots(
            folders
                .iter()
                .take(MAX_ROOT_CANDIDATES)
                .filter_map(|folder| folder.get("uri").and_then(Value::as_str)),
        );
    }
    if params
        .get("workspaceFolders")
        .is_some_and(|folders| !folders.is_null())
    {
        return Vec::new();
    }
    params
        .get("rootUri")
        .and_then(Value::as_str)
        .map(|uri| canonical_roots(std::iter::once(uri)))
        .unwrap_or_default()
}

/// Apply an initialization root snapshot or a `workspaceFolders` change event.
pub(super) fn update_roots(roots: &mut Vec<WorkspaceRoot>, params: &Value) {
    let Some(event) = params.get("event") else {
        *roots = roots_from_initialize(params);
        return;
    };

    let removed: HashSet<PathBuf> = event
        .get("removed")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|folder| folder.get("uri").and_then(Value::as_str))
        .filter_map(file_uri_to_path)
        .map(|path| normalize_path(&path))
        .collect();
    roots.retain(|root| !removed.contains(&root.identity));

    let added = event
        .get("added")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_ROOT_CANDIDATES)
        .filter_map(|folder| folder.get("uri").and_then(Value::as_str));
    roots.extend(canonical_roots(added));
    roots.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.identity.cmp(&right.identity))
    });
    roots.dedup_by(|left, right| left.identity == right.identity);
    roots.truncate(MAX_ROOTS + 1);
}

/// Scan workspace roots and all open CK documents with bounded I/O and parsing.
pub(super) fn symbols(
    roots: &[WorkspaceRoot],
    documents: &HashMap<String, super::DocumentSnapshot>,
    query: &str,
) -> WorkspaceSymbols {
    let mut truncated = roots.len() > MAX_ROOTS;
    let mut open_documents = documents
        .iter()
        .filter_map(|(uri, document)| {
            let path = file_uri_to_path(uri)?;
            if !has_ck_extension(&path) {
                return None;
            }
            let path = canonicalize_or_normalize(&path)?;
            Some((path, uri.as_str(), document))
        })
        .collect::<Vec<_>>();
    open_documents.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(right.1)));

    let shadowed_paths: HashSet<PathBuf> = open_documents
        .iter()
        .map(|(path, _, _)| path.clone())
        .collect();
    let open_count = shadowed_paths.len();
    if open_count > MAX_CK_FILES {
        truncated = true;
    }
    let mut sources = Vec::new();
    let mut seen_paths = shadowed_paths.clone();
    let mut seen_open_paths = HashSet::new();
    for (path, uri, document) in open_documents {
        if !seen_open_paths.insert(path.clone()) {
            continue;
        }
        if seen_open_paths.len() > MAX_CK_FILES {
            break;
        }
        let Some(text) = document.text.as_deref() else {
            continue;
        };
        sources.push(Source {
            path,
            uri: uri.to_owned(),
            root: None,
            open_text: Some(text),
            is_open: true,
        });
    }

    let mut entries_seen = 0;
    let remaining_files = MAX_CK_FILES.saturating_sub(open_count);
    let mut disk_files: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut roots_to_scan = roots.iter().take(MAX_ROOTS).cloned().collect::<Vec<_>>();
    roots_to_scan.sort_by(|left, right| left.path.cmp(&right.path));
    for root in roots_to_scan {
        if open_count >= MAX_CK_FILES {
            truncated = true;
            break;
        }
        scan_root(
            &root.path,
            &shadowed_paths,
            &mut seen_paths,
            remaining_files,
            &mut entries_seen,
            &mut disk_files,
            &mut truncated,
        );
        if entries_seen >= MAX_DIRECTORY_ENTRIES || disk_files.len() >= remaining_files {
            break;
        }
    }
    disk_files.sort_by(|left, right| left.0.cmp(&right.0));
    for (path, root_for_source) in disk_files {
        let Some(uri) = path_to_file_uri(&path) else {
            continue;
        };
        sources.push(Source {
            path,
            uri,
            root: Some(root_for_source),
            open_text: None,
            is_open: false,
        });
    }

    sources.sort_by(|left, right| left.uri.cmp(&right.uri));
    let query = query.to_lowercase();
    let mut ranked = Vec::new();
    let mut parsed_bytes = 0_u64;
    for source in sources {
        let text = if source.is_open {
            let Some(text) = source.open_text else {
                continue;
            };
            let size = text.len() as u64;
            if size > MAX_FILE_BYTES {
                truncated = true;
                continue;
            }
            if parsed_bytes.saturating_add(size) > MAX_TOTAL_BYTES {
                truncated = true;
                break;
            }
            parsed_bytes = parsed_bytes.saturating_add(size);
            text.to_owned()
        } else {
            let Ok(metadata) = fs::symlink_metadata(&source.path) else {
                continue;
            };
            let file_type = metadata.file_type();
            if !file_type.is_file() || is_reparse_point(&source.path, &file_type) {
                continue;
            }
            let size = metadata.len();
            if size > MAX_FILE_BYTES {
                truncated = true;
                continue;
            }
            if parsed_bytes.saturating_add(size) > MAX_TOTAL_BYTES {
                truncated = true;
                break;
            }
            let Ok(file) = open_source_file(&source.path) else {
                continue;
            };
            let Ok(opened_metadata) = file.metadata() else {
                continue;
            };
            if !opened_metadata.is_file() || metadata_is_reparse_point(&opened_metadata) {
                continue;
            }
            let Some(root) = source.root.as_deref() else {
                continue;
            };
            let Ok(opened_path) = opened_file_path(&file) else {
                truncated = true;
                continue;
            };
            if !opened_path.starts_with(root) {
                truncated = true;
                continue;
            }
            let mut text = String::new();
            if file
                .take(MAX_FILE_BYTES + 1)
                .read_to_string(&mut text)
                .is_err()
            {
                continue;
            }
            let actual_size = text.len() as u64;
            if actual_size > MAX_FILE_BYTES {
                truncated = true;
                continue;
            }
            if parsed_bytes.saturating_add(actual_size) > MAX_TOTAL_BYTES {
                truncated = true;
                break;
            }
            parsed_bytes = parsed_bytes.saturating_add(actual_size);
            text
        };
        if text.len() as u64 > MAX_FILE_BYTES {
            truncated = true;
            continue;
        }
        if super::analysis_limit_warning(&text).is_some() {
            truncated = true;
            continue;
        }
        append_top_level_symbols(&source.uri, &text, &query, &mut ranked);
        if ranked.len() > MAX_RESULTS {
            truncated = true;
            break;
        }
    }

    ranked.sort_by(|left, right| {
        left.uri
            .cmp(&right.uri)
            .then(left.line.cmp(&right.line))
            .then(left.character.cmp(&right.character))
            .then(left.name.cmp(&right.name))
    });
    let mut seen_symbols = HashSet::new();
    ranked
        .retain(|symbol| seen_symbols.insert((symbol.uri.clone(), symbol.line, symbol.character)));
    if ranked.len() > MAX_RESULTS {
        ranked.truncate(MAX_RESULTS);
        truncated = true;
    }
    WorkspaceSymbols {
        items: ranked.into_iter().map(|symbol| symbol.value).collect(),
        truncated,
    }
}

struct Source<'a> {
    path: PathBuf,
    uri: String,
    root: Option<PathBuf>,
    open_text: Option<&'a str>,
    is_open: bool,
}

struct RankedSymbol {
    uri: String,
    line: usize,
    character: usize,
    name: String,
    value: Value,
}

fn append_top_level_symbols(uri: &str, text: &str, query: &str, output: &mut Vec<RankedSymbol>) {
    let parsed = parse(&SourceFile::new(uri, text));
    for declaration in parsed.ast.declarations {
        let (name, kind, span) = match declaration {
            Declaration::Function(function) => (function.name.name, 12, function.span),
            Declaration::Struct(structure) => (structure.name.name, 23, structure.span),
        };
        if !name.to_lowercase().contains(query) {
            continue;
        }
        let start = (
            span.start.line.saturating_sub(1),
            span.start.column.saturating_sub(1),
        );
        let end = (
            span.end.line.saturating_sub(1),
            span.end.column.saturating_sub(1),
        );
        output.push(RankedSymbol {
            uri: uri.to_owned(),
            line: start.0,
            character: start.1,
            name: name.clone(),
            value: json!({
                "name": name,
                "kind": kind,
                "location": {
                    "uri": uri,
                    "range": {
                        "start": {"line": start.0, "character": start.1},
                        "end": {"line": end.0, "character": end.1}
                    }
                }
            }),
        });
        if output.len() > MAX_RESULTS {
            return;
        }
    }
}

fn scan_root(
    root: &Path,
    shadowed: &HashSet<PathBuf>,
    seen: &mut HashSet<PathBuf>,
    file_limit: usize,
    entries_seen: &mut usize,
    files: &mut Vec<(PathBuf, PathBuf)>,
    truncated: &mut bool,
) {
    if file_limit == 0 {
        *truncated = true;
        return;
    }
    let Ok(root_metadata) = fs::symlink_metadata(root) else {
        return;
    };
    let root_type = root_metadata.file_type();
    if !root_type.is_dir() || is_reparse_point(root, &root_type) {
        return;
    }

    let mut pending = vec![root.to_path_buf()];
    'directories: while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        let mut children = Vec::new();
        let mut directory_limit_reached = false;
        for entry in entries {
            let Ok(entry) = entry else {
                continue;
            };
            if *entries_seen == MAX_DIRECTORY_ENTRIES {
                *truncated = true;
                directory_limit_reached = true;
                break;
            }
            *entries_seen += 1;
            children.push(entry);
        }
        children.sort_by_key(|entry| entry.file_name());
        for entry in children.into_iter().rev() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() || is_reparse_point(&path, &file_type) {
                continue;
            }
            if file_type.is_dir() {
                if is_excluded_directory(&entry.file_name()) {
                    continue;
                }
                pending.push(path);
                continue;
            }
            if !file_type.is_file() || !has_ck_extension(&path) {
                continue;
            }
            let Some(path) = canonicalize_or_normalize(&path) else {
                continue;
            };
            if !path.starts_with(root) || path != normalize_path(&entry.path()) {
                continue;
            }
            if shadowed.contains(&path) || !seen.insert(path.clone()) {
                continue;
            }
            if files.len() == file_limit {
                *truncated = true;
                break 'directories;
            }
            files.push((path, root.to_path_buf()));
        }
        if directory_limit_reached {
            break 'directories;
        }
    }
}

fn is_excluded_directory(name: &std::ffi::OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    matches!(
        name.to_ascii_lowercase().as_str(),
        ".git" | ".hg" | ".svn" | "node_modules" | "target" | "build" | ".worktrees"
    )
}

fn is_reparse_point(path: &Path, file_type: &fs::FileType) -> bool {
    if file_type.is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        return fs::symlink_metadata(path)
            .map(|metadata| metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
            .unwrap_or(true);
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

fn metadata_is_reparse_point(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        return metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn open_source_file(path: &Path) -> std::io::Result<fs::File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options.open(path)
}

#[cfg(target_os = "linux")]
fn opened_file_path(file: &fs::File) -> std::io::Result<PathBuf> {
    use std::os::fd::AsRawFd;

    fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))
}

#[cfg(target_os = "macos")]
fn opened_file_path(file: &fs::File) -> std::io::Result<PathBuf> {
    use std::{ffi::CStr, os::fd::AsRawFd, os::unix::ffi::OsStrExt};

    let mut buffer = [0_u8; 4096];
    // F_GETPATH writes the resolved path associated with this open file descriptor.
    let result = unsafe {
        libc::fcntl(
            file.as_raw_fd(),
            libc::F_GETPATH,
            buffer.as_mut_ptr().cast::<std::ffi::c_void>(),
        )
    };
    if result == -1 {
        return Err(std::io::Error::last_os_error());
    }
    let path = unsafe { CStr::from_ptr(buffer.as_ptr().cast()) };
    Ok(PathBuf::from(std::ffi::OsStr::from_bytes(path.to_bytes())))
}

#[cfg(windows)]
fn opened_file_path(file: &fs::File) -> std::io::Result<PathBuf> {
    use std::{
        os::windows::{ffi::OsStringExt, io::AsRawHandle},
        ptr,
    };
    use windows_sys::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{FILE_NAME_NORMALIZED, GetFinalPathNameByHandleW, VOLUME_NAME_DOS},
    };

    let handle = file.as_raw_handle() as HANDLE;
    let flags = FILE_NAME_NORMALIZED | VOLUME_NAME_DOS;
    let required = unsafe { GetFinalPathNameByHandleW(handle, ptr::null_mut(), 0, flags) };
    if required == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut buffer = vec![0_u16; required as usize];
    let written = unsafe {
        GetFinalPathNameByHandleW(handle, buffer.as_mut_ptr(), buffer.len() as u32, flags)
    };
    if written == 0 || written as usize >= buffer.len() {
        return Err(std::io::Error::last_os_error());
    }
    Ok(PathBuf::from(std::ffi::OsString::from_wide(
        &buffer[..written as usize],
    )))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn opened_file_path(_file: &fs::File) -> std::io::Result<PathBuf> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "file handle path verification is unavailable on this platform",
    ))
}

fn has_ck_extension(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "ck")
}

fn canonical_roots<'a>(uris: impl Iterator<Item = &'a str>) -> Vec<WorkspaceRoot> {
    let mut roots = Vec::new();
    let mut seen = HashSet::new();
    for uri in uris.take(MAX_ROOT_CANDIDATES) {
        let Some(identity) = file_uri_to_path(uri) else {
            continue;
        };
        let identity = normalize_path(&identity);
        if !seen.insert(identity.clone()) {
            continue;
        }
        let Ok(path) = fs::canonicalize(&identity) else {
            continue;
        };
        if !path.is_dir() {
            continue;
        }
        roots.push(WorkspaceRoot { identity, path });
        if roots.len() > MAX_ROOTS {
            break;
        }
    }
    roots.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.identity.cmp(&right.identity))
    });
    roots
}

fn canonicalize_or_normalize(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    let normalized = normalize_path(path);
    if let Ok(path) = fs::canonicalize(&normalized) {
        return Some(path);
    }

    let mut ancestor = normalized.clone();
    let mut remainder = Vec::new();
    loop {
        if let Ok(mut path) = fs::canonicalize(&ancestor) {
            for component in remainder.iter().rev() {
                path.push(component);
            }
            return Some(normalize_path(&path));
        }
        remainder.push(ancestor.file_name()?.to_os_string());
        if !ancestor.pop() {
            return None;
        }
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                let _ = normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    normalized
}

fn percent_encode_path(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len());
    for &byte in bytes {
        if is_uri_unreserved(byte) || matches!(byte, b'/' | b':') {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

#[cfg(windows)]
fn percent_encode_authority(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len());
    for &byte in bytes {
        if is_uri_unreserved(byte) {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

fn percent_decode_path(encoded: &str) -> Option<String> {
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = hex_value(*bytes.get(index + 1)?)?;
            let low = hex_value(*bytes.get(index + 2)?)?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            if !bytes[index].is_ascii()
                || bytes[index].is_ascii_control()
                || !(is_uri_unreserved(bytes[index]) || matches!(bytes[index], b'/' | b':'))
            {
                return None;
            }
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    let decoded = String::from_utf8(decoded).ok()?;
    (!decoded.contains('\0')).then_some(decoded)
}

#[cfg(windows)]
fn percent_decode_authority(encoded: &str) -> Option<String> {
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = hex_value(*bytes.get(index + 1)?)?;
            let low = hex_value(*bytes.get(index + 2)?)?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            if !is_uri_unreserved(bytes[index]) {
                return None;
            }
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

const fn is_uri_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicUsize, Ordering},
    };

    use serde_json::{Value, json};

    use super::super::DocumentSnapshot;
    use super::MAX_FILE_BYTES;
    use super::{file_uri_to_path, path_to_file_uri, roots_from_initialize, symbols, update_roots};

    static NEXT_TEMP_DIR: AtomicUsize = AtomicUsize::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let id = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("ckc-lsp-workspace-{}-{id}", std::process::id()));
            fs::create_dir_all(&path).expect("create temporary directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_ck(path: &Path, source: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create source parent");
        }
        fs::write(path, source).expect("write CK source");
    }

    fn uri(path: &Path) -> String {
        path_to_file_uri(path).expect("absolute path has a file URI")
    }

    fn workspace_root(path: &Path) -> super::WorkspaceRoot {
        let identity = super::normalize_path(path);
        let path = fs::canonicalize(path).expect("canonical workspace root");
        super::WorkspaceRoot { identity, path }
    }

    fn file_symbols(items: &[Value]) -> Vec<(String, String)> {
        items
            .iter()
            .map(|item| {
                (
                    item["name"].as_str().expect("symbol name").to_owned(),
                    item["location"]["uri"]
                        .as_str()
                        .expect("symbol URI")
                        .to_owned(),
                )
            })
            .collect()
    }

    #[test]
    fn file_uri_round_trips_unicode_spaces_hash_and_percent() {
        let directory = TempDir::new();
        let path = directory.path().join("函数 空格#百分号%.ck");
        let encoded = uri(&path);

        assert!(encoded.contains("%20"));
        assert!(encoded.contains("%23"));
        assert!(encoded.contains("%25"));
        assert_eq!(file_uri_to_path(&encoded), Some(path));
    }

    #[test]
    fn file_uri_parser_rejects_relative_malformed_and_query_uris() {
        assert_eq!(file_uri_to_path("file:relative.ck"), None);
        assert_eq!(file_uri_to_path("file:///tmp/bad%2.ck"), None);
        assert_eq!(file_uri_to_path("file:///tmp/source.ck?query"), None);
        assert_eq!(file_uri_to_path("file:///tmp/source.ck#fragment"), None);
        assert_eq!(file_uri_to_path("file:///tmp/%00.ck"), None);
        #[cfg(unix)]
        assert_eq!(file_uri_to_path("file://remote-host/tmp/source.ck"), None);
        #[cfg(unix)]
        assert_eq!(
            file_uri_to_path("file://localhost/tmp/source.ck"),
            Some(PathBuf::from("/tmp/source.ck"))
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_file_uris_round_trip_drive_and_unc_paths() {
        let drive = PathBuf::from(r"C:\Users\Lynn\函数 空格#%.ck");
        let unc = PathBuf::from(r"\\server\share\函数 空格#%.ck");
        let verbatim_drive = PathBuf::from(r"\\?\C:\Users\Lynn\sample.ck");
        let verbatim_unc = PathBuf::from(r"\\?\UNC\server\share\sample.ck");

        assert_eq!(file_uri_to_path(&uri(&drive)), Some(drive));
        assert_eq!(file_uri_to_path(&uri(&unc)), Some(unc));
        assert_eq!(uri(&verbatim_drive), "file:///C:/Users/Lynn/sample.ck");
        assert_eq!(uri(&verbatim_unc), "file://server/share/sample.ck");
    }

    #[test]
    fn initialize_workspace_folders_override_root_uri_even_when_empty() {
        let directory = TempDir::new();
        let first = directory.path().join("first");
        let fallback = directory.path().join("fallback");
        fs::create_dir_all(&first).expect("first root");
        fs::create_dir_all(&fallback).expect("fallback root");

        let params = json!({
            "workspaceFolders": [{"uri": uri(&first), "name": "first"}],
            "rootUri": uri(&fallback)
        });
        assert_eq!(roots_from_initialize(&params), vec![workspace_root(&first)]);

        let params = json!({"workspaceFolders": [], "rootUri": uri(&fallback)});
        assert!(roots_from_initialize(&params).is_empty());
    }

    #[test]
    fn initialize_uses_root_uri_when_workspace_folders_are_absent() {
        let directory = TempDir::new();
        let root = directory.path().join("root");
        fs::create_dir_all(&root).expect("workspace root");

        assert_eq!(
            roots_from_initialize(&json!({"rootUri": uri(&root)})),
            vec![workspace_root(&root)]
        );
        assert!(roots_from_initialize(&json!({"rootUri": null})).is_empty());
    }

    #[test]
    fn initialize_falls_back_to_root_uri_when_workspace_folders_are_null() {
        let directory = TempDir::new();
        let root = directory.path().join("root");
        fs::create_dir_all(&root).expect("workspace root");

        assert_eq!(
            roots_from_initialize(&json!({
                "workspaceFolders": null,
                "rootUri": uri(&root)
            })),
            vec![workspace_root(&root)]
        );
    }

    #[test]
    fn workspace_scan_caps_roots_at_sixteen_and_reports_truncation() {
        let directory = TempDir::new();
        let mut folders = Vec::new();
        let mut roots = Vec::new();
        for index in 0..17 {
            let root = directory.path().join(format!("root-{index:02}"));
            write_ck(
                &root.join("module.ck"),
                &format!("fn root_{index}() -> i32 {{ return 0; }}"),
            );
            folders.push(json!({"uri": uri(&root), "name": format!("root-{index}")}));
            roots.push(workspace_root(&root));
        }
        let params = json!({"workspaceFolders": folders});
        let resolved_roots = roots_from_initialize(&params);
        assert_eq!(resolved_roots.len(), 17);

        let result = symbols(&resolved_roots, &HashMap::new(), "");
        assert_eq!(result.items.len(), 16);
        assert!(result.truncated);
    }

    #[test]
    fn workspace_folder_changes_add_and_remove_canonical_roots() {
        let directory = TempDir::new();
        let first = directory.path().join("first");
        let second = directory.path().join("second");
        fs::create_dir_all(&first).expect("first root");
        fs::create_dir_all(&second).expect("second root");
        let first = workspace_root(&first);
        let second = workspace_root(&second);
        let mut roots = vec![first.clone()];

        update_roots(
            &mut roots,
            &json!({"event": {
                "added": [{"uri": uri(&second.identity), "name": "second"}],
                "removed": [{"uri": uri(&first.identity), "name": "first"}]
            }}),
        );

        assert_eq!(roots, vec![second]);
    }

    #[test]
    fn scans_unopened_files_in_stable_order_and_filters_by_query() {
        let directory = TempDir::new();
        let root = directory.path().join("workspace");
        write_ck(&root.join("z.ck"), "fn zebra() -> i32 { return 0; }");
        write_ck(&root.join("nested/a.ck"), "struct Item { value: i32; }");
        write_ck(
            &root.join("node_modules/hidden.ck"),
            "fn hidden() -> i32 { return 0; }",
        );
        write_ck(
            &root.join("target/hidden.ck"),
            "fn built() -> i32 { return 0; }",
        );
        let roots = vec![workspace_root(&root)];
        let documents = HashMap::new();

        let all = symbols(&roots, &documents, "");
        assert_eq!(
            file_symbols(&all.items),
            vec![
                ("Item".to_owned(), uri(&roots[0].path.join("nested/a.ck"))),
                ("zebra".to_owned(), uri(&roots[0].path.join("z.ck")))
            ]
        );
        let filtered = symbols(&roots, &documents, "BRA");
        assert_eq!(filtered.items.len(), 1);
        assert_eq!(filtered.items[0]["name"], "zebra");
    }

    #[test]
    fn open_unsaved_text_overrides_disk_and_missing_text_shadows_disk() {
        let directory = TempDir::new();
        let root = directory.path().join("workspace");
        let source = root.join("edit.ck");
        write_ck(&source, "fn on_disk() -> i32 { return 0; }");
        let roots = vec![workspace_root(&root)];
        let source_uri = uri(&source);
        let document_uri = source_uri.replace("edit.ck", "%65dit.ck");
        let mut documents = HashMap::from([(
            document_uri.clone(),
            DocumentSnapshot {
                version: 2,
                text: Some("fn unsaved() -> i32 { return 0; }".to_owned()),
            },
        )]);

        let edited = symbols(&roots, &documents, "");
        assert_eq!(edited.items.len(), 1);
        assert_eq!(edited.items[0]["name"], "unsaved");
        assert_eq!(edited.items[0]["location"]["uri"], document_uri);

        documents.get_mut(&document_uri).unwrap().text = None;
        assert!(symbols(&roots, &documents, "").items.is_empty());
    }

    #[test]
    fn open_ck_documents_outside_workspace_roots_are_included() {
        let directory = TempDir::new();
        let workspace = directory.path().join("workspace");
        let outside = directory.path().join("outside.ck");
        fs::create_dir_all(&workspace).expect("workspace root");
        let roots = vec![workspace_root(&workspace)];
        let documents = HashMap::from([(
            uri(&outside),
            DocumentSnapshot {
                version: 1,
                text: Some("fn external() -> i32 { return 0; }".to_owned()),
            },
        )]);

        let result = symbols(&roots, &documents, "external");
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0]["name"], "external");
    }

    #[cfg(unix)]
    #[test]
    fn does_not_follow_symlinked_files_or_directories() {
        use std::os::unix::fs::symlink;

        let directory = TempDir::new();
        let root = directory.path().join("workspace");
        let outside = directory.path().join("outside");
        write_ck(
            &outside.join("external.ck"),
            "fn external() -> i32 { return 0; }",
        );
        fs::create_dir_all(&root).expect("workspace root");
        symlink(&outside, root.join("linked_dir")).expect("directory symlink");
        symlink(outside.join("external.ck"), root.join("linked_file.ck")).expect("file symlink");
        let roots = vec![workspace_root(&root)];

        assert!(symbols(&roots, &HashMap::new(), "").items.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn removing_a_symlinked_root_uses_its_original_path_identity() {
        use std::os::unix::fs::symlink;

        let directory = TempDir::new();
        let target_a = directory.path().join("target-a");
        let target_b = directory.path().join("target-b");
        let alias = directory.path().join("workspace-link");
        write_ck(
            &target_a.join("child/a.ck"),
            "fn from_a() -> i32 { return 0; }",
        );
        write_ck(
            &target_b.join("child/b.ck"),
            "fn from_b() -> i32 { return 0; }",
        );
        symlink(&target_a, &alias).expect("workspace root symlink");
        let folder = alias.join("child");

        let mut roots = roots_from_initialize(&json!({"rootUri": uri(&folder)}));
        assert_eq!(roots.len(), 1);
        assert_eq!(
            roots[0].path,
            fs::canonicalize(target_a.join("child")).unwrap()
        );
        assert_eq!(
            symbols(&roots, &HashMap::new(), "").items[0]["name"],
            "from_a"
        );

        fs::remove_file(&alias).expect("remove workspace symlink");
        symlink(&target_b, &alias).expect("retarget workspace symlink");
        update_roots(
            &mut roots,
            &json!({"event": {"removed": [{"uri": uri(&folder)}], "added": []}}),
        );
        assert!(roots.is_empty());
    }

    #[test]
    fn caps_scanned_ck_files_and_workspace_symbol_results() {
        let directory = TempDir::new();
        let root = directory.path().join("workspace");
        fs::create_dir_all(&root).expect("workspace root");
        for index in 0..513 {
            write_ck(
                &root.join(format!("{index:04}.ck")),
                &format!("fn symbol_{index}() -> i32 {{ return 0; }}"),
            );
        }
        let root = workspace_root(&root);

        let limited_files = symbols(std::slice::from_ref(&root), &HashMap::new(), "");
        assert_eq!(limited_files.items.len(), 512);
        assert!(limited_files.truncated);

        let many_symbols = directory.path().join("many.ck");
        let mut source = String::new();
        for index in 0..1_001 {
            source.push_str(&format!("fn result_{index}() -> i32 {{ return 0; }}\n"));
        }
        write_ck(&many_symbols, &source);
        let open_documents = HashMap::from([(
            uri(&many_symbols),
            DocumentSnapshot {
                version: 1,
                text: Some(source),
            },
        )]);

        let limited_results = symbols(&[], &open_documents, "");
        assert_eq!(limited_results.items.len(), 1_000);
        assert!(limited_results.truncated);
    }

    #[test]
    fn workspace_symbol_ranges_use_utf16_positions_after_non_bmp_text() {
        let directory = TempDir::new();
        let path = directory.path().join("position.ck");
        let text = "// 😀\nfn after_comment() -> i32 { return 0; }";
        let documents = HashMap::from([(
            uri(&path),
            DocumentSnapshot {
                version: 1,
                text: Some(text.to_owned()),
            },
        )]);

        let result = symbols(&[], &documents, "after");
        assert_eq!(result.items[0]["location"]["range"]["start"]["line"], 1);
        assert_eq!(
            result.items[0]["location"]["range"]["start"]["character"],
            0
        );
    }

    #[test]
    fn total_workspace_source_bytes_are_bounded() {
        let directory = TempDir::new();
        let root = directory.path().join("workspace");
        fs::create_dir_all(&root).expect("workspace root");
        let mut source = "fn bounded() -> i32 { return 0; }\n//".to_owned();
        source.push_str(&"x".repeat(MAX_FILE_BYTES as usize - source.len()));
        for index in 0..5 {
            write_ck(&root.join(format!("{index}.ck")), &source);
        }
        let roots = vec![workspace_root(&root)];

        let result = symbols(&roots, &HashMap::new(), "");
        assert_eq!(result.items.len(), 4);
        assert!(result.truncated);
    }

    #[test]
    fn skips_a_source_larger_than_four_mib_without_blocking_other_files() {
        let directory = TempDir::new();
        let root = directory.path().join("workspace");
        write_ck(
            &root.join("large.ck"),
            &"x".repeat(MAX_FILE_BYTES as usize + 1),
        );
        write_ck(&root.join("small.ck"), "fn small() -> i32 { return 0; }");
        let roots = vec![workspace_root(&root)];

        let result = symbols(&roots, &HashMap::new(), "");
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0]["name"], "small");
        assert!(result.truncated);
    }
}
