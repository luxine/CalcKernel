use crate::KirWasmFeatures;

use wasmparser::{Validator, WasmFeatures};

pub(super) fn target_metadata(features: KirWasmFeatures, profile_sha256: &str) -> String {
    format!(
        "{{\"schema\":1,\"target\":\"wasm32\",\"features\":\"{}\",\"profile_sha256\":\"{}\"}}",
        features.as_str(),
        profile_sha256,
    )
}

pub(super) fn append_target_metadata(
    mut wat: String,
    features: KirWasmFeatures,
    profile_sha256: &str,
) -> Result<String, String> {
    let metadata = target_metadata(features, profile_sha256);
    let escaped = metadata
        .bytes()
        .map(|byte| match byte {
            b'"' => "\\22".to_string(),
            b'\\' => "\\5c".to_string(),
            0x20..=0x7e => char::from(byte).to_string(),
            _ => format!("\\{byte:02x}"),
        })
        .collect::<String>();
    let custom = format!("  (@custom \"ck.wasm.target\" \"{escaped}\")\n");
    let closing = wat
        .rfind(')')
        .ok_or_else(|| "WebAssembly WAT is missing its module terminator".to_string())?;
    wat.insert_str(closing, &custom);
    Ok(wat)
}

pub(super) fn allowed_wasm_features(features: KirWasmFeatures) -> WasmFeatures {
    let mut allowed = WasmFeatures::MVP | WasmFeatures::MULTI_VALUE;
    if features == KirWasmFeatures::Simd128 {
        allowed |= WasmFeatures::SIMD;
    }
    allowed
}

pub(super) fn validate_wasm(bytes: &[u8], features: KirWasmFeatures) -> Result<(), String> {
    Validator::new_with_features(allowed_wasm_features(features))
        .validate_all(bytes)
        .map(|_| ())
        .map_err(|error| {
            format!(
                "WebAssembly module uses features outside the {} profile: {error}",
                features.as_str()
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validate_wat(source: &str, features: KirWasmFeatures) -> Result<(), String> {
        let bytes = wat::parse_str(source).expect("injected WAT parses");
        validate_wasm(&bytes, features)
    }

    #[test]
    fn validator_allows_only_the_declared_scalar_simd_feature_set() {
        const SIMD: &str = r#"(module (func (result v128) (v128.const i32x4 1 2 3 4)))"#;
        assert!(validate_wat(SIMD, KirWasmFeatures::Baseline).is_err());
        assert!(validate_wat(SIMD, KirWasmFeatures::Simd128).is_ok());

        const RELAXED_SIMD: &str = r#"(module (func (result v128) (i8x16.relaxed_swizzle (v128.const i8x16 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0) (v128.const i8x16 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0))))"#;
        assert!(validate_wat(RELAXED_SIMD, KirWasmFeatures::Simd128).is_err());

        const SHARED_MEMORY: &str = r#"(module (memory 1 1 shared))"#;
        assert!(validate_wat(SHARED_MEMORY, KirWasmFeatures::Baseline).is_err());

        const MEMORY64: &str = r#"(module (memory i64 1))"#;
        assert!(validate_wat(MEMORY64, KirWasmFeatures::Baseline).is_err());
        assert!(validate_wat(MEMORY64, KirWasmFeatures::Simd128).is_err());
    }
}
