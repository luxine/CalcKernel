use super::features::validate_wasm;
use super::final_ir::{
    FinalInstructionKind, FinalWasmFunction, FinalWasmModule, LaneOpcode, MemoryOpcode,
    WasmValueType, validate_module,
};

use crate::KirWasmFeatures;

use std::borrow::Cow;
use std::collections::HashMap;
use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, CustomSection, ExportKind, ExportSection, Function,
    FunctionSection, GlobalSection, GlobalType, Ieee64, Instruction, MemArg, MemorySection,
    MemoryType, Module, TypeSection, ValType,
};

/// Encodes the final typed module directly. This path deliberately has no WAT
/// printer/parser dependency; generated instructions are lowered straight to
/// wasm-encoder operations and the owned metadata payload becomes one custom
/// section.
pub(super) fn encode_final_module(
    source: &FinalWasmModule,
    features: KirWasmFeatures,
    expected_metadata: &[u8],
) -> Result<Vec<u8>, String> {
    validate_module(source)?;
    if source.target_metadata.as_deref() != Some(expected_metadata) {
        return Err("WebAssembly target metadata does not match the selected KIR profile".into());
    }
    if expected_metadata.is_empty() {
        return Err("WebAssembly target metadata payload must not be empty".into());
    }

    let function_indices = source
        .functions
        .iter()
        .enumerate()
        .map(|(index, function)| (function.name.as_str(), index as u32))
        .collect::<HashMap<_, _>>();
    let mut signatures = Vec::<(Vec<ValType>, Vec<ValType>)>::new();
    let mut signature_indices = HashMap::<(Vec<ValType>, Vec<ValType>), u32>::new();
    let mut function_type_indices = Vec::with_capacity(source.functions.len());
    for function in &source.functions {
        let signature = (
            function
                .params
                .iter()
                .map(|local| encoder_type(local.ty))
                .collect(),
            function.results.iter().copied().map(encoder_type).collect(),
        );
        function_type_indices.push(intern_signature(
            signature,
            &mut signatures,
            &mut signature_indices,
        ));
    }
    for function in &source.functions {
        for instruction in &function.body {
            let results = match &instruction.kind {
                FinalInstructionKind::Block { results, .. }
                | FinalInstructionKind::Loop { results, .. }
                | FinalInstructionKind::If { results, .. } => results,
                _ => continue,
            };
            if results.len() > 1 {
                let signature = (
                    Vec::new(),
                    results.iter().copied().map(encoder_type).collect(),
                );
                intern_signature(signature, &mut signatures, &mut signature_indices);
            }
        }
    }

    let mut types = TypeSection::new();
    for (params, results) in &signatures {
        types
            .ty()
            .function(params.iter().copied(), results.iter().copied());
    }

    let mut functions = FunctionSection::new();
    for type_index in &function_type_indices {
        functions.function(*type_index);
    }

    let mut memories = MemorySection::new();
    memories.memory(MemoryType {
        minimum: u64::from(source.memory_minimum),
        maximum: None,
        memory64: false,
        shared: false,
        page_size_log2: None,
    });

    let mut globals = GlobalSection::new();
    globals.global(
        GlobalType {
            val_type: ValType::I32,
            mutable: false,
            shared: false,
        },
        &ConstExpr::i32_const(0),
    );

    let mut exports = ExportSection::new();
    exports.export("memory", ExportKind::Memory, 0);
    exports.export("__ck_heap_base", ExportKind::Global, 0);
    let mut export_names = std::collections::HashSet::new();
    export_names.insert("memory".to_string());
    export_names.insert("__ck_heap_base".to_string());
    for (index, function) in source.functions.iter().enumerate() {
        if let Some(name) = &function.export_name {
            if !export_names.insert(name.clone()) {
                return Err(format!("duplicate WebAssembly export name: {name}"));
            }
            exports.export(name, ExportKind::Func, index as u32);
        }
    }

    let mut code = CodeSection::new();
    for function in &source.functions {
        let mut body = Function::new(local_groups(function));
        encode_function_body(&mut body, function, &function_indices, &signature_indices)?;
        body.instruction(&Instruction::End);
        code.function(&body);
    }

    let mut module = Module::new();
    if !signatures.is_empty() {
        module.section(&types);
    }
    if !source.functions.is_empty() {
        module.section(&functions);
    }
    module.section(&memories);
    module.section(&globals);
    module.section(&exports);
    if !source.functions.is_empty() {
        module.section(&code);
    }
    module.section(&CustomSection {
        name: Cow::Borrowed("ck.wasm.target"),
        data: Cow::Borrowed(expected_metadata),
    });

    let bytes = module.finish();
    validate_profile_binary(&bytes, features, expected_metadata)?;
    Ok(bytes)
}

fn intern_signature(
    signature: (Vec<ValType>, Vec<ValType>),
    signatures: &mut Vec<(Vec<ValType>, Vec<ValType>)>,
    indices: &mut HashMap<(Vec<ValType>, Vec<ValType>), u32>,
) -> u32 {
    if let Some(index) = indices.get(&signature) {
        return *index;
    }
    let index = signatures.len() as u32;
    signatures.push(signature.clone());
    indices.insert(signature, index);
    index
}

fn encoder_type(ty: WasmValueType) -> ValType {
    match ty {
        WasmValueType::I32 => ValType::I32,
        WasmValueType::I64 => ValType::I64,
        WasmValueType::F32 => ValType::F32,
        WasmValueType::F64 => ValType::F64,
        WasmValueType::V128 => ValType::V128,
    }
}

fn local_groups(function: &FinalWasmFunction) -> Vec<(u32, ValType)> {
    let mut groups = Vec::<(u32, ValType)>::new();
    for local in &function.locals {
        let ty = encoder_type(local.ty);
        if let Some((count, prior_type)) = groups.last_mut()
            && *prior_type == ty
        {
            *count += 1;
        } else {
            groups.push((1, ty));
        }
    }
    groups
}

fn encode_function_body(
    body: &mut Function,
    function: &FinalWasmFunction,
    function_indices: &HashMap<&str, u32>,
    signatures: &HashMap<(Vec<ValType>, Vec<ValType>), u32>,
) -> Result<(), String> {
    let mut local_indices = HashMap::<&str, u32>::new();
    for (index, local) in function.params.iter().enumerate() {
        local_indices.insert(&local.name, index as u32);
    }
    for (index, local) in function.locals.iter().enumerate() {
        let full_index = function
            .params
            .len()
            .checked_add(index)
            .ok_or("too many WebAssembly locals")?;
        local_indices.insert(
            &local.name,
            u32::try_from(full_index).map_err(|_| "too many WebAssembly locals")?,
        );
    }
    let mut labels = Vec::<Option<String>>::new();
    for instruction in &function.body {
        let encoded = match &instruction.kind {
            FinalInstructionKind::Simple(opcode) => opcode.encoder_instruction(),
            FinalInstructionKind::I32Const(value) => Instruction::I32Const(*value),
            FinalInstructionKind::I64Const(value) => Instruction::I64Const(*value),
            FinalInstructionKind::F64Const(bits) => Instruction::F64Const(Ieee64::new(*bits)),
            FinalInstructionKind::V128Const(bytes) => Instruction::V128Const(v128_bits(bytes)?),
            FinalInstructionKind::LocalGet(name) => {
                Instruction::LocalGet(*local_index(&local_indices, name)?)
            }
            FinalInstructionKind::LocalSet(name) => {
                Instruction::LocalSet(*local_index(&local_indices, name)?)
            }
            FinalInstructionKind::LocalTee(name) => {
                Instruction::LocalTee(*local_index(&local_indices, name)?)
            }
            FinalInstructionKind::Call(name) => Instruction::Call(
                *function_indices
                    .get(name.as_str())
                    .ok_or_else(|| format!("call references unknown generated function ${name}"))?,
            ),
            FinalInstructionKind::Block { label, results } => {
                let ty = block_type(results, signatures)?;
                labels.push(label.clone());
                Instruction::Block(ty)
            }
            FinalInstructionKind::Loop { label, results } => {
                let ty = block_type(results, signatures)?;
                labels.push(label.clone());
                Instruction::Loop(ty)
            }
            FinalInstructionKind::If { label, results } => {
                let ty = block_type(results, signatures)?;
                labels.push(label.clone());
                Instruction::If(ty)
            }
            FinalInstructionKind::Else => Instruction::Else,
            FinalInstructionKind::End => {
                labels.pop().ok_or_else(|| {
                    format!("unmatched end in generated function ${}", function.name)
                })?;
                Instruction::End
            }
            FinalInstructionKind::Br(name) => {
                Instruction::Br(label_depth(&labels, name, function)?)
            }
            FinalInstructionKind::BrIf(name) => {
                Instruction::BrIf(label_depth(&labels, name, function)?)
            }
            FinalInstructionKind::BrTable { targets, default } => {
                let depths = targets
                    .iter()
                    .map(|target| label_depth(&labels, target, function))
                    .collect::<Result<Vec<_>, _>>()?;
                let default = label_depth(&labels, default, function)?;
                Instruction::BrTable(depths.into(), default)
            }
            FinalInstructionKind::Load {
                opcode,
                offset,
                align,
            } => {
                if opcode.is_store() {
                    return Err(format!(
                        "store opcode appears in a load instruction in ${}",
                        function.name
                    ));
                }
                memory_instruction(*opcode, *offset, *align)?
            }
            FinalInstructionKind::Store {
                opcode,
                offset,
                align,
            } => {
                if !opcode.is_store() {
                    return Err(format!(
                        "load opcode appears in a store instruction in ${}",
                        function.name
                    ));
                }
                memory_instruction(*opcode, *offset, *align)?
            }
            FinalInstructionKind::MemorySize => Instruction::MemorySize(0),
            FinalInstructionKind::MemoryGrow => Instruction::MemoryGrow(0),
            FinalInstructionKind::MemoryCopy => Instruction::MemoryCopy {
                src_mem: 0,
                dst_mem: 0,
            },
            FinalInstructionKind::MemoryFill => Instruction::MemoryFill(0),
            FinalInstructionKind::Lane { opcode, lane } => lane_instruction(*opcode, *lane),
            FinalInstructionKind::Shuffle(lanes) => Instruction::I8x16Shuffle(*lanes),
        };
        body.instruction(&encoded);
    }
    if !labels.is_empty() {
        return Err(format!(
            "unclosed structured control in generated function ${}",
            function.name
        ));
    }
    Ok(())
}

fn local_index<'a>(indices: &'a HashMap<&'a str, u32>, name: &str) -> Result<&'a u32, String> {
    indices
        .get(name)
        .ok_or_else(|| format!("instruction references unknown generated local ${name}"))
}

fn label_depth(
    labels: &[Option<String>],
    name: &str,
    function: &FinalWasmFunction,
) -> Result<u32, String> {
    labels
        .iter()
        .rposition(|label| label.as_deref() == Some(name))
        .map(|index| {
            u32::try_from(labels.len() - 1 - index).expect("control nesting bounded by module size")
        })
        .ok_or_else(|| {
            format!(
                "branch references unknown active label ${name} in function ${}",
                function.name
            )
        })
}

fn block_type(
    results: &[WasmValueType],
    signatures: &HashMap<(Vec<ValType>, Vec<ValType>), u32>,
) -> Result<BlockType, String> {
    match results {
        [] => Ok(BlockType::Empty),
        [result] => Ok(BlockType::Result(encoder_type(*result))),
        _ => {
            let key = (
                Vec::new(),
                results.iter().copied().map(encoder_type).collect(),
            );
            signatures
                .get(&key)
                .copied()
                .map(BlockType::FunctionType)
                .ok_or_else(|| "missing deduplicated WebAssembly block signature".into())
        }
    }
}

fn v128_bits(bytes: &[u8]) -> Result<i128, String> {
    let bytes: [u8; 16] = bytes
        .try_into()
        .map_err(|_| "generated v128 constant must have 16 bytes")?;
    Ok(i128::from_le_bytes(bytes))
}

fn memarg(offset: u64, align: u32) -> MemArg {
    MemArg {
        offset,
        align,
        memory_index: 0,
    }
}

fn memory_instruction(
    opcode: MemoryOpcode,
    offset: u64,
    align: u32,
) -> Result<Instruction<'static>, String> {
    let arg = memarg(offset, align);
    Ok(match opcode {
        MemoryOpcode::I32Load => Instruction::I32Load(arg),
        MemoryOpcode::I64Load => Instruction::I64Load(arg),
        MemoryOpcode::F32Load => Instruction::F32Load(arg),
        MemoryOpcode::F64Load => Instruction::F64Load(arg),
        MemoryOpcode::I32Load8S => Instruction::I32Load8S(arg),
        MemoryOpcode::I32Load8U => Instruction::I32Load8U(arg),
        MemoryOpcode::I32Load16S => Instruction::I32Load16S(arg),
        MemoryOpcode::I32Load16U => Instruction::I32Load16U(arg),
        MemoryOpcode::I64Load8S => Instruction::I64Load8S(arg),
        MemoryOpcode::I64Load8U => Instruction::I64Load8U(arg),
        MemoryOpcode::I64Load16S => Instruction::I64Load16S(arg),
        MemoryOpcode::I64Load16U => Instruction::I64Load16U(arg),
        MemoryOpcode::I64Load32S => Instruction::I64Load32S(arg),
        MemoryOpcode::I64Load32U => Instruction::I64Load32U(arg),
        MemoryOpcode::I32Store => Instruction::I32Store(arg),
        MemoryOpcode::I64Store => Instruction::I64Store(arg),
        MemoryOpcode::F32Store => Instruction::F32Store(arg),
        MemoryOpcode::F64Store => Instruction::F64Store(arg),
        MemoryOpcode::I32Store8 => Instruction::I32Store8(arg),
        MemoryOpcode::I32Store16 => Instruction::I32Store16(arg),
        MemoryOpcode::I64Store8 => Instruction::I64Store8(arg),
        MemoryOpcode::I64Store16 => Instruction::I64Store16(arg),
        MemoryOpcode::I64Store32 => Instruction::I64Store32(arg),
        MemoryOpcode::V128Load => Instruction::V128Load(arg),
        MemoryOpcode::V128Load8x8S => Instruction::V128Load8x8S(arg),
        MemoryOpcode::V128Load8x8U => Instruction::V128Load8x8U(arg),
        MemoryOpcode::V128Load16x4S => Instruction::V128Load16x4S(arg),
        MemoryOpcode::V128Load16x4U => Instruction::V128Load16x4U(arg),
        MemoryOpcode::V128Load32x2S => Instruction::V128Load32x2S(arg),
        MemoryOpcode::V128Load32x2U => Instruction::V128Load32x2U(arg),
        MemoryOpcode::V128Load8Splat => Instruction::V128Load8Splat(arg),
        MemoryOpcode::V128Load16Splat => Instruction::V128Load16Splat(arg),
        MemoryOpcode::V128Load32Splat => Instruction::V128Load32Splat(arg),
        MemoryOpcode::V128Load64Splat => Instruction::V128Load64Splat(arg),
        MemoryOpcode::V128Load32Zero => Instruction::V128Load32Zero(arg),
        MemoryOpcode::V128Load64Zero => Instruction::V128Load64Zero(arg),
        MemoryOpcode::V128Store => Instruction::V128Store(arg),
    })
}

fn lane_instruction(opcode: LaneOpcode, lane: u8) -> Instruction<'static> {
    use Instruction::*;
    match opcode {
        LaneOpcode::I8x16ExtractLaneS => I8x16ExtractLaneS(lane),
        LaneOpcode::I8x16ExtractLaneU => I8x16ExtractLaneU(lane),
        LaneOpcode::I8x16ReplaceLane => I8x16ReplaceLane(lane),
        LaneOpcode::I16x8ExtractLaneS => I16x8ExtractLaneS(lane),
        LaneOpcode::I16x8ExtractLaneU => I16x8ExtractLaneU(lane),
        LaneOpcode::I16x8ReplaceLane => I16x8ReplaceLane(lane),
        LaneOpcode::I32x4ExtractLane => I32x4ExtractLane(lane),
        LaneOpcode::I32x4ReplaceLane => I32x4ReplaceLane(lane),
        LaneOpcode::I64x2ExtractLane => I64x2ExtractLane(lane),
        LaneOpcode::I64x2ReplaceLane => I64x2ReplaceLane(lane),
        LaneOpcode::F32x4ExtractLane => F32x4ExtractLane(lane),
        LaneOpcode::F32x4ReplaceLane => F32x4ReplaceLane(lane),
        LaneOpcode::F64x2ExtractLane => F64x2ExtractLane(lane),
        LaneOpcode::F64x2ReplaceLane => F64x2ReplaceLane(lane),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::wasm::final_ir::FinalWasmBuilder;

    const METADATA: &[u8] = br#"{"schema":2,"target":"wasm32","features":"baseline"}"#;
    const MODULE_PREFIX: &str = "(module\n  (memory (export \"memory\") 1)\n  (global (export \"__ck_heap_base\") i32 (i32.const 0))\n";

    fn final_module(source: &str) -> FinalWasmModule {
        let mut builder = FinalWasmBuilder::new();
        builder.push_str(source);
        let mut module = builder.finish().expect("generated final IR");
        module.set_target_metadata(METADATA.to_vec()).unwrap();
        module
    }

    #[test]
    fn direct_writer_encodes_bulk_memory_control_flow_and_multi_value_functions() {
        let source = format!(
            "{MODULE_PREFIX}  (func $copy (export \"copy\")\n    (param $dst i32)\n    (param $src i32)\n    (param $len i32)\n    local.get $dst\n    local.get $src\n    local.get $len\n    memory.copy\n    local.get $dst\n    i32.const 65\n    i32.const 4\n    memory.fill\n    block $exit\n    loop $again\n    local.get $len\n    i32.eqz\n    br_if $exit\n    local.get $len\n    br_table $again $exit\n    br $again\n    end\n    end\n  )\n  (func $slice (export \"slice\")\n    (param $data i32)\n    (param $length i32)\n    (result i32 i32)\n    local.get $data\n    local.get $length\n  )\n)\n"
        );
        let module = final_module(&source);
        let bytes = encode_final_module(&module, KirWasmFeatures::Baseline, METADATA).unwrap();
        assert_eq!(&bytes[..8], b"\0asm\x01\0\0\0");
        verify_target_metadata(&bytes, METADATA).unwrap();
        validate_wasm(&bytes, KirWasmFeatures::Baseline).unwrap();
        assert_eq!(
            wasmparser::Parser::new(0)
                .parse_all(&bytes)
                .filter_map(|payload| match payload.unwrap() {
                    wasmparser::Payload::CustomSection(section) if section.name() == "name" => {
                        Some(())
                    }
                    _ => None,
                })
                .count(),
            0,
            "direct output omits the name section"
        );
    }

    #[test]
    fn direct_writer_handles_simd_memory_and_deduplicates_signatures() {
        let metadata = br#"{"schema":2,"target":"wasm32","features":"simd128"}"#;
        let source = format!(
            "{MODULE_PREFIX}  (func $a (export \"a\")\n    (param $x i32)\n    (result i64)\n    i64.const 18446744073709551615\n  )\n  (func $b (export \"b\")\n    (param $x i32)\n    (result i64)\n    i64.const -1\n  )\n  (func $simd (export \"simd\")\n    (param $address i32)\n    (result i32)\n    local.get $address\n    v128.load64_zero offset=0 align=8\n    i32x4.extract_lane 0\n  )\n  (func $convert (export \"convert\")\n    (result f64)\n    v128.const i32x4 1 2 3 4\n    f64x2.convert_low_i32x4_s\n    f64x2.extract_lane 0\n  )\n  (func $vector_nan32 (export \"vector_nan32\")\n    (result v128)\n    v128.const f32x4 nan:0x123 -nan:0x1 nan:canonical -0.0\n  )\n  (func $vector_nan64 (export \"vector_nan64\")\n    (result v128)\n    v128.const f64x2 nan:0x123 -nan:0x1\n  )\n)\n"
        );
        let mut module = final_module_without_metadata(&source);
        module.set_target_metadata(metadata.to_vec()).unwrap();
        let bytes = encode_final_module(&module, KirWasmFeatures::Simd128, metadata).unwrap();
        let type_count = wasmparser::Parser::new(0)
            .parse_all(&bytes)
            .find_map(|payload| match payload.unwrap() {
                wasmparser::Payload::TypeSection(reader) => Some(reader.count()),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            type_count, 4,
            "equal scalar and vector signatures share one type each"
        );
        let oracle = wat::parse_str(module.to_wat()).unwrap();
        assert_eq!(operator_lists(&bytes), operator_lists(&oracle));
        validate_wasm(&bytes, KirWasmFeatures::Simd128).unwrap();
    }

    #[test]
    fn direct_writer_encodes_empty_export_module_and_requires_target_metadata() {
        let source = format!("{MODULE_PREFIX})\n");
        let mut module = FinalWasmBuilder::new();
        module.push_str(&source);
        let module = module.finish().unwrap();
        assert!(encode_final_module(&module, KirWasmFeatures::Baseline, METADATA).is_err());

        let mut module = module;
        module.set_target_metadata(METADATA.to_vec()).unwrap();
        let bytes = encode_final_module(&module, KirWasmFeatures::Baseline, METADATA).unwrap();
        validate_wasm(&bytes, KirWasmFeatures::Baseline).unwrap();
        assert_eq!(
            wasmparser::Parser::new(0)
                .parse_all(&bytes)
                .filter_map(|payload| match payload.unwrap() {
                    wasmparser::Payload::FunctionSection(reader) => Some(reader.count()),
                    _ => None,
                })
                .sum::<u32>(),
            0
        );
    }

    #[test]
    fn direct_writer_preserves_high_integer_and_float_immediate_bits() {
        let source = format!(
            "{MODULE_PREFIX}  (func $high32 (export \"high32\")\n    (result i32)\n    i32.const 4294967295\n  )\n  (func $high64 (export \"high64\")\n    (result i64)\n    i64.const 18446744073709551615\n  )\n  (func $negative_zero (export \"negative_zero\")\n    (result f64)\n    f64.const -0.0\n  )\n  (func $nan_payload (export \"nan_payload\")\n    (result f64)\n    f64.const nan:0x123\n  )\n)\n"
        );
        let module = final_module(&source);
        let direct = encode_final_module(&module, KirWasmFeatures::Baseline, METADATA).unwrap();
        let oracle = wat::parse_str(module.to_wat()).unwrap();
        assert_eq!(operator_lists(&direct), operator_lists(&oracle));
    }

    fn operator_lists(bytes: &[u8]) -> Vec<Vec<String>> {
        wasmparser::Parser::new(0)
            .parse_all(bytes)
            .filter_map(|payload| match payload.unwrap() {
                wasmparser::Payload::CodeSectionEntry(body) => Some(
                    body.get_operators_reader()
                        .unwrap()
                        .into_iter()
                        .map(|operator| format!("{:?}", operator.unwrap()))
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .collect()
    }

    fn final_module_without_metadata(source: &str) -> FinalWasmModule {
        let mut builder = FinalWasmBuilder::new();
        builder.push_str(source);
        builder.finish().expect("generated final IR")
    }
}
