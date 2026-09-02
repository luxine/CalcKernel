use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use unicode_normalization::UnicodeNormalization;

use super::schema::TUNE_MANIFEST_SCHEMA;

/// One closed workload partition role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuneCaseRole {
    Search,
    Validation,
}

/// One canonical tuning workload case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuneCase {
    pub id: String,
    pub role: TuneCaseRole,
    pub seed: u64,
    pub weight: u32,
    pub expected_digest: [u8; 32],
}

/// A validated schema-1 tuning workload manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuneManifest {
    pub(crate) runner_path: PathBuf,
    pub(crate) input_root: PathBuf,
    pub(crate) args: Vec<String>,
    pub(crate) inputs: Vec<String>,
    pub(crate) inherit_env: Vec<String>,
    pub(crate) timeout_ms: u32,
    pub(crate) cases: Vec<TuneCase>,
}

/// Closed manifest parsing and validation failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TuneManifestError {
    #[error("manifest is not valid UTF-8 TOML")]
    InvalidToml,
    #[error("unknown manifest field {0}")]
    UnknownField(String),
    #[error("missing manifest field {0}")]
    MissingField(&'static str),
    #[error("invalid manifest field {0}")]
    InvalidField(&'static str),
    #[error("invalid logical input path {0}")]
    InvalidInputPath(String),
    #[error("duplicate case id {0}")]
    DuplicateCaseId(String),
    #[error("duplicate case seed {0}")]
    DuplicateCaseSeed(u64),
    #[error("workload must contain search and validation cases")]
    MissingPartition,
    #[error("manifest resource limit exceeded for {0}")]
    ResourceLimit(&'static str),
}

impl TuneManifest {
    /// Parses the closed schema-1 TOML manifest.
    ///
    /// # Errors
    ///
    /// Rejects malformed, open, ambiguous, noncanonical, or out-of-bound input.
    pub fn parse(bytes: &[u8], manifest_path: &Path) -> Result<Self, TuneManifestError> {
        let source = std::str::from_utf8(bytes).map_err(|_| TuneManifestError::InvalidToml)?;
        let value: toml::Value = source.parse().map_err(|_| TuneManifestError::InvalidToml)?;
        let root = value.as_table().ok_or(TuneManifestError::InvalidToml)?;
        reject_unknown(root, &["schema", "runner", "case"])?;
        let schema = integer(root, "schema", "schema")?;
        if schema != i64::from(TUNE_MANIFEST_SCHEMA) {
            return Err(TuneManifestError::InvalidField("schema"));
        }
        let runner = root
            .get("runner")
            .and_then(toml::Value::as_table)
            .ok_or(TuneManifestError::MissingField("runner"))?;
        reject_unknown(
            runner,
            &[
                "path",
                "input_root",
                "args",
                "inputs",
                "inherit_env",
                "timeout_ms",
            ],
        )?;
        let runner_spelling = string(runner, "path", "runner.path")?;
        validate_text(runner_spelling, "runner.path")?;
        let base = manifest_path.parent().unwrap_or_else(|| Path::new("."));
        let runner_path = resolve_operational(base, runner_spelling);
        let input_root_spelling = optional_string(runner, "input_root", ".", "runner.input_root")?;
        validate_text(input_root_spelling, "runner.input_root")?;
        let input_root = resolve_operational(base, input_root_spelling);
        let args = optional_string_array(runner, "args", 64, "runner.args")?;
        validate_text_aggregate(&args, 65_536, "runner.args")?;
        let inputs = optional_string_array(runner, "inputs", 64, "runner.inputs")?;
        validate_text_aggregate(&inputs, 65_536, "runner.inputs")?;
        for input in &inputs {
            if !valid_logical_path(input) {
                return Err(TuneManifestError::InvalidInputPath(input.clone()));
            }
        }
        let inherit_env = optional_string_array(runner, "inherit_env", 16, "runner.inherit_env")?;
        let mut environment_names = BTreeSet::new();
        for name in &inherit_env {
            if !valid_environment_name(name) {
                return Err(TuneManifestError::InvalidField("runner.inherit_env"));
            }
            if !environment_names.insert(name.clone()) {
                return Err(TuneManifestError::InvalidField("runner.inherit_env"));
            }
        }
        let timeout = runner.get("timeout_ms").map_or(Ok(30_000), |value| {
            value
                .as_integer()
                .and_then(|value| u32::try_from(value).ok())
                .ok_or(TuneManifestError::InvalidField("runner.timeout_ms"))
        })?;
        if !(100..=120_000).contains(&timeout) {
            return Err(TuneManifestError::InvalidField("runner.timeout_ms"));
        }
        let case_values = root
            .get("case")
            .and_then(toml::Value::as_array)
            .ok_or(TuneManifestError::MissingField("case"))?;
        if case_values.is_empty() || case_values.len() > 16 {
            return Err(TuneManifestError::ResourceLimit("case"));
        }
        let mut cases = Vec::with_capacity(case_values.len());
        let mut ids = BTreeSet::new();
        let mut seeds = BTreeSet::new();
        let mut partitions = 0u8;
        for value in case_values {
            let table = value
                .as_table()
                .ok_or(TuneManifestError::InvalidField("case"))?;
            reject_unknown(table, &["id", "role", "seed", "weight", "expected_digest"])?;
            let id = string(table, "id", "case.id")?;
            if id.is_empty()
                || id.len() > 64
                || !id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
            {
                return Err(TuneManifestError::InvalidField("case.id"));
            }
            if !ids.insert(id.to_owned()) {
                return Err(TuneManifestError::DuplicateCaseId(id.to_owned()));
            }
            let role = match string(table, "role", "case.role")? {
                "search" => TuneCaseRole::Search,
                "validation" => TuneCaseRole::Validation,
                _ => return Err(TuneManifestError::InvalidField("case.role")),
            };
            partitions |= match role {
                TuneCaseRole::Search => 1,
                TuneCaseRole::Validation => 2,
            };
            let seed = unsigned_integer(table, "seed", "case.seed")?;
            if !seeds.insert(seed) {
                return Err(TuneManifestError::DuplicateCaseSeed(seed));
            }
            let weight = u32::try_from(unsigned_integer(table, "weight", "case.weight")?)
                .map_err(|_| TuneManifestError::InvalidField("case.weight"))?;
            if weight == 0 {
                return Err(TuneManifestError::InvalidField("case.weight"));
            }
            let expected_digest =
                parse_digest(string(table, "expected_digest", "case.expected_digest")?)?;
            cases.push(TuneCase {
                id: id.to_owned(),
                role,
                seed,
                weight,
                expected_digest,
            });
        }
        if partitions != 3 {
            return Err(TuneManifestError::MissingPartition);
        }
        cases.sort_by(|left, right| left.id.as_bytes().cmp(right.id.as_bytes()));
        Ok(Self {
            runner_path,
            input_root,
            args,
            inputs,
            inherit_env,
            timeout_ms: timeout,
            cases,
        })
    }

    /// Returns the configured full per-invocation timeout.
    #[must_use]
    pub const fn timeout_ms(&self) -> u32 {
        self.timeout_ms
    }

    /// Returns the canonical case-id ordered cases.
    #[must_use]
    pub fn cases(&self) -> &[TuneCase] {
        &self.cases
    }

    /// Returns every operational path that an output must not alias.
    #[must_use]
    pub fn protected_paths(&self) -> Vec<PathBuf> {
        let mut paths = vec![self.runner_path.clone(), self.input_root.clone()];
        paths.extend(self.inputs.iter().map(|input| self.input_root.join(input)));
        paths
    }
}

fn reject_unknown(table: &toml::Table, allowed: &[&str]) -> Result<(), TuneManifestError> {
    if let Some(key) = table.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(TuneManifestError::UnknownField(key.clone()));
    }
    Ok(())
}

fn integer(table: &toml::Table, key: &str, label: &'static str) -> Result<i64, TuneManifestError> {
    table
        .get(key)
        .ok_or(TuneManifestError::MissingField(label))?
        .as_integer()
        .ok_or(TuneManifestError::InvalidField(label))
}

fn unsigned_integer(
    table: &toml::Table,
    key: &str,
    label: &'static str,
) -> Result<u64, TuneManifestError> {
    u64::try_from(integer(table, key, label)?).map_err(|_| TuneManifestError::InvalidField(label))
}

fn string<'a>(
    table: &'a toml::Table,
    key: &str,
    label: &'static str,
) -> Result<&'a str, TuneManifestError> {
    table
        .get(key)
        .ok_or(TuneManifestError::MissingField(label))?
        .as_str()
        .ok_or(TuneManifestError::InvalidField(label))
}

fn optional_string<'a>(
    table: &'a toml::Table,
    key: &str,
    default: &'a str,
    label: &'static str,
) -> Result<&'a str, TuneManifestError> {
    table.get(key).map_or(Ok(default), |value| {
        value.as_str().ok_or(TuneManifestError::InvalidField(label))
    })
}

fn optional_string_array(
    table: &toml::Table,
    key: &str,
    limit: usize,
    label: &'static str,
) -> Result<Vec<String>, TuneManifestError> {
    let Some(value) = table.get(key) else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or(TuneManifestError::InvalidField(label))?;
    if values.len() > limit {
        return Err(TuneManifestError::ResourceLimit(label));
    }
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or(TuneManifestError::InvalidField(label))
        })
        .collect()
}

fn validate_text(value: &str, label: &'static str) -> Result<(), TuneManifestError> {
    if value.len() > 4_096 || value.contains('\0') || value.nfc().ne(value.chars()) {
        return Err(TuneManifestError::InvalidField(label));
    }
    Ok(())
}

fn validate_text_aggregate(
    values: &[String],
    limit: usize,
    label: &'static str,
) -> Result<(), TuneManifestError> {
    let mut bytes = 0usize;
    for value in values {
        validate_text(value, label)?;
        bytes = bytes
            .checked_add(value.len())
            .ok_or(TuneManifestError::ResourceLimit(label))?;
    }
    if bytes > limit {
        return Err(TuneManifestError::ResourceLimit(label));
    }
    Ok(())
}

fn valid_logical_path(value: &str) -> bool {
    !value.is_empty()
        && !Path::new(value).is_absolute()
        && Path::new(value).components().all(|component| {
            matches!(component, Component::Normal(_))
                && component
                    .as_os_str()
                    .to_str()
                    .is_some_and(|part| part != "." && part != "..")
        })
}

fn valid_environment_name(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn resolve_operational(base: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn parse_digest(value: &str) -> Result<[u8; 32], TuneManifestError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(TuneManifestError::InvalidField("case.expected_digest"));
    }
    let mut digest = [0u8; 32];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        digest[index] = (hex_nibble(chunk[0])? << 4) | hex_nibble(chunk[1])?;
    }
    Ok(digest)
}

fn hex_nibble(value: u8) -> Result<u8, TuneManifestError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err(TuneManifestError::InvalidField("case.expected_digest")),
    }
}
