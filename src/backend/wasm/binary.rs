use crate::*;

use super::{EmitWasmOptions, emit::emit_wat_module_with_options};

pub fn emit_wasm_module(module: &MirModule) -> Result<Vec<u8>, String> {
    emit_wasm_module_with_options(module, EmitWasmOptions::default())
}

pub fn emit_wasm_module_with_options(
    module: &MirModule,
    options: EmitWasmOptions,
) -> Result<Vec<u8>, String> {
    let bytes = wat::parse_str(emit_wat_module_with_options(module, options))
        .map_err(|error| error.to_string())?;
    strip_wasm_name_section(&bytes)
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
