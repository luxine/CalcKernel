use std::path::Path;

use unicode_normalization::UnicodeNormalization;

const INPUT_MAP_MAGIC: &[u8; 8] = b"CKTIMAP1";
const MAX_INPUTS: u32 = 64;
const MAX_TEXT_BYTES: usize = 4_096;

/// One canonical manifest-order CKTIMAP1 entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuneInputMapEntry {
    pub logical_path: String,
    pub staged_basename: String,
    pub bytes: u64,
    pub digest: [u8; 32],
}

/// Bounded CKTIMAP1 codec failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TuneInputMapError {
    #[error("truncated input map")]
    Truncated,
    #[error("unexpected input-map magic")]
    UnexpectedMagic,
    #[error("input-map resource limit exceeded")]
    ResourceLimit,
    #[error("invalid input-map text")]
    InvalidText,
    #[error("invalid staged input basename")]
    InvalidBasename,
    #[error("trailing input-map bytes")]
    TrailingBytes,
}

/// Encodes a canonical CKTIMAP1 record.
///
/// # Errors
///
/// Rejects excess entries and noncanonical logical or staged names.
pub fn encode_input_map(entries: &[TuneInputMapEntry]) -> Result<Vec<u8>, TuneInputMapError> {
    let limit = usize::try_from(MAX_INPUTS).map_err(|_| TuneInputMapError::ResourceLimit)?;
    if entries.len() > limit {
        return Err(TuneInputMapError::ResourceLimit);
    }
    let mut output = Vec::new();
    output.extend_from_slice(INPUT_MAP_MAGIC);
    output.extend_from_slice(
        &u32::try_from(entries.len())
            .map_err(|_| TuneInputMapError::ResourceLimit)?
            .to_be_bytes(),
    );
    for entry in entries {
        validate_entry(entry)?;
        encode_text(&mut output, &entry.logical_path)?;
        encode_text(&mut output, &entry.staged_basename)?;
        output.extend_from_slice(&entry.bytes.to_be_bytes());
        output.extend_from_slice(&entry.digest);
    }
    Ok(output)
}

/// Decodes one exact-EOF CKTIMAP1 record.
///
/// # Errors
///
/// Rejects foreign, truncated, excessive, noncanonical, or trailing input.
pub fn decode_input_map(bytes: &[u8]) -> Result<Vec<TuneInputMapEntry>, TuneInputMapError> {
    let mut cursor = Cursor::new(bytes);
    if cursor.take(8)? != INPUT_MAP_MAGIC {
        return Err(TuneInputMapError::UnexpectedMagic);
    }
    let count = cursor.u32()?;
    if count > MAX_INPUTS {
        return Err(TuneInputMapError::ResourceLimit);
    }
    let capacity = usize::try_from(count).map_err(|_| TuneInputMapError::ResourceLimit)?;
    let mut entries = Vec::with_capacity(capacity);
    for _ in 0..count {
        let logical_path = cursor.text()?;
        let staged_basename = cursor.text()?;
        let bytes = cursor.u64()?;
        let mut digest = [0u8; 32];
        digest.copy_from_slice(cursor.take(32)?);
        let entry = TuneInputMapEntry {
            logical_path,
            staged_basename,
            bytes,
            digest,
        };
        validate_entry(&entry)?;
        entries.push(entry);
    }
    if !cursor.is_finished() {
        return Err(TuneInputMapError::TrailingBytes);
    }
    Ok(entries)
}

fn validate_entry(entry: &TuneInputMapEntry) -> Result<(), TuneInputMapError> {
    validate_text(&entry.logical_path)?;
    validate_text(&entry.staged_basename)?;
    if Path::new(&entry.logical_path).is_absolute()
        || entry
            .logical_path
            .split(['/', '\\'])
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(TuneInputMapError::InvalidText);
    }
    if !valid_staged_basename(&entry.staged_basename, &entry.digest) {
        return Err(TuneInputMapError::InvalidBasename);
    }
    Ok(())
}

fn validate_text(value: &str) -> Result<(), TuneInputMapError> {
    if value.len() > MAX_TEXT_BYTES || value.contains('\0') || value.nfc().ne(value.chars()) {
        return Err(TuneInputMapError::InvalidText);
    }
    Ok(())
}

fn valid_staged_basename(value: &str, digest: &[u8; 32]) -> bool {
    if value.len() != 8 + 1 + 64 + 4 || !value.ends_with(".bin") || value.as_bytes()[8] != b'-' {
        return false;
    }
    if !value.as_bytes()[..8].iter().all(u8::is_ascii_hexdigit)
        || !value.as_bytes()[..8]
            .iter()
            .all(|byte| !byte.is_ascii_uppercase())
    {
        return false;
    }
    value.as_bytes()[9..73]
        .iter()
        .copied()
        .eq(hex(digest).bytes())
}

fn encode_text(output: &mut Vec<u8>, value: &str) -> Result<(), TuneInputMapError> {
    validate_text(value)?;
    output.extend_from_slice(
        &u32::try_from(value.len())
            .map_err(|_| TuneInputMapError::ResourceLimit)?
            .to_be_bytes(),
    );
    output.extend_from_slice(value.as_bytes());
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], TuneInputMapError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(TuneInputMapError::ResourceLimit)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(TuneInputMapError::Truncated)?;
        self.offset = end;
        Ok(value)
    }

    fn u32(&mut self) -> Result<u32, TuneInputMapError> {
        let bytes = self.take(4)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn u64(&mut self) -> Result<u64, TuneInputMapError> {
        let bytes = self.take(8)?;
        Ok(u64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    fn text(&mut self) -> Result<String, TuneInputMapError> {
        let length = usize::try_from(self.u32()?).map_err(|_| TuneInputMapError::ResourceLimit)?;
        if length > MAX_TEXT_BYTES {
            return Err(TuneInputMapError::ResourceLimit);
        }
        let value = std::str::from_utf8(self.take(length)?)
            .map_err(|_| TuneInputMapError::InvalidText)?
            .to_owned();
        validate_text(&value)?;
        Ok(value)
    }

    fn is_finished(&self) -> bool {
        self.offset == self.bytes.len()
    }
}
