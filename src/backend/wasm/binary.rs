use super::features::validate_wasm;

use crate::KirWasmFeatures;

pub(super) fn emit_wasm_module_from_profile_wat(
    source: &str,
    features: KirWasmFeatures,
    expected_metadata: &[u8],
) -> Result<Vec<u8>, String> {
    let bytes = encode_and_strip_names(source)?;
    validate_profile_binary(&bytes, features, expected_metadata)?;
    Ok(bytes)
}

pub(super) fn validate_profile_wat(
    source: &str,
    features: KirWasmFeatures,
    expected_metadata: &[u8],
) -> Result<(), String> {
    let bytes = encode_and_strip_names(source)?;
    validate_profile_binary(&bytes, features, expected_metadata)
}

fn validate_profile_binary(
    bytes: &[u8],
    features: KirWasmFeatures,
    expected_metadata: &[u8],
) -> Result<(), String> {
    verify_target_metadata(bytes, expected_metadata)?;
    validate_wasm(bytes, features)?;
    Ok(())
}

fn encode_and_strip_names(source: &str) -> Result<Vec<u8>, String> {
    let bytes = wat::parse_str(source).map_err(|error| error.to_string())?;
    strip_wasm_name_section(&bytes)
}

fn verify_target_metadata(bytes: &[u8], expected: &[u8]) -> Result<(), String> {
    let sections = wasmparser::Parser::new(0)
        .parse_all(bytes)
        .map(|payload| payload.map_err(|error| error.to_string()))
        .filter_map(|payload| match payload {
            Ok(wasmparser::Payload::CustomSection(section))
                if section.name() == "ck.wasm.target" =>
            {
                Some(Ok(section.data()))
            }
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if sections.len() != 1 {
        return Err(format!(
            "WebAssembly module must contain exactly one ck.wasm.target section (found {})",
            sections.len()
        ));
    }
    if sections[0] != expected {
        return Err("WebAssembly target metadata does not match the selected KIR profile".into());
    }
    Ok(())
}

fn strip_wasm_name_section(bytes: &[u8]) -> Result<Vec<u8>, String> {
    const WASM_HEADER_LEN: usize = 8;
    if bytes.len() < WASM_HEADER_LEN || &bytes[..WASM_HEADER_LEN] != b"\0asm\x01\0\0\0" {
        return Err("WAT to WASM failed: invalid WebAssembly binary header".to_string());
    }

    let mut out = bytes[..WASM_HEADER_LEN].to_vec();
    let mut offset = WASM_HEADER_LEN;
    while offset < bytes.len() {
        let section_start = offset;
        let section_id = bytes[offset];
        offset += 1;
        let (payload_len, next_offset) = read_wasm_u32(bytes, offset)?;
        offset = next_offset;
        let payload_start = offset;
        let payload_end = payload_start
            .checked_add(payload_len as usize)
            .ok_or_else(|| "WAT to WASM failed: malformed section length".to_string())?;
        if payload_end > bytes.len() {
            return Err("WAT to WASM failed: truncated section payload".to_string());
        }

        let is_name_section = section_id == 0
            && wasm_custom_section_name(&bytes[payload_start..payload_end])? == Some("name");
        if !is_name_section {
            out.extend_from_slice(&bytes[section_start..payload_end]);
        }
        offset = payload_end;
    }
    Ok(out)
}

fn wasm_custom_section_name(payload: &[u8]) -> Result<Option<&str>, String> {
    let (name_len, name_start) = read_wasm_u32(payload, 0)?;
    let name_end = name_start
        .checked_add(name_len as usize)
        .ok_or_else(|| "WAT to WASM failed: malformed custom section name".to_string())?;
    if name_end > payload.len() {
        return Err("WAT to WASM failed: truncated custom section name".to_string());
    }
    std::str::from_utf8(&payload[name_start..name_end])
        .map(Some)
        .map_err(|error| format!("WAT to WASM failed: invalid custom section name: {error}"))
}

fn read_wasm_u32(bytes: &[u8], mut offset: usize) -> Result<(u32, usize), String> {
    let mut value = 0u32;
    let mut shift = 0;
    for _ in 0..5 {
        let byte = *bytes
            .get(offset)
            .ok_or_else(|| "WAT to WASM failed: truncated LEB128 value".to_string())?;
        offset += 1;
        value |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok((value, offset));
        }
        shift += 7;
    }
    Err("WAT to WASM failed: malformed LEB128 value".to_string())
}
