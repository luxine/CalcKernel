use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use crate::{
    MirBinaryOp, MirBlock, MirCastOp, MirCompareOp, MirFunction, MirInstruction, MirModule,
    MirPlace, MirPrimitiveTypeName, MirTerminator, MirType, MirUnaryOp, MirValue,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverflowMode {
    Unchecked,
    Checked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundsMode {
    Unchecked,
    Checked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmitCOptions {
    pub overflow_mode: OverflowMode,
    pub bounds_mode: BoundsMode,
    pub opt_level: u8,
}

impl Default for EmitCOptions {
    fn default() -> Self {
        Self {
            overflow_mode: OverflowMode::Unchecked,
            bounds_mode: BoundsMode::Unchecked,
            opt_level: 0,
        }
    }
}

#[must_use]
pub fn emit_c_module(module: &MirModule, options: EmitCOptions) -> String {
    if use_planned_c_emitter(module, options) {
        return emit_planned_c_module(module, options, None);
    }
    let mut out = String::new();
    out.push_str("#include <stdbool.h>\n#include <stdint.h>\n");
    if options.overflow_mode == OverflowMode::Checked {
        out.push_str(
            "#include <stddef.h>\n\n\
             typedef int32_t CK_Status;\n\n\
             #define CK_OK ((CK_Status)0)\n\
             #define CK_ERR_OVERFLOW ((CK_Status)1)\n\
             #define CK_ERR_DIV_BY_ZERO ((CK_Status)2)\n\
             #define CK_ERR_NULL_POINTER ((CK_Status)3)\n\
             #define CK_ERR_OUT_OF_BOUNDS ((CK_Status)4)\n\n",
        );
    } else {
        out.push('\n');
    }

    for struct_info in &module.structs {
        out.push_str(&format!("typedef struct {} {{\n", struct_info.name));
        for field in &struct_info.fields {
            out.push_str(&format!("  {} {};\n", c_type(&field.type_node), field.name));
        }
        out.push_str(&format!("}} {};\n\n", struct_info.name));
    }

    for function in &module.functions {
        let signature = if options.overflow_mode == OverflowMode::Checked {
            checked_c_signature(function)
        } else {
            c_signature(function)
        };
        out.push_str(&format!("{signature};\n"));
    }
    if !module.functions.is_empty() {
        out.push('\n');
    }

    for (index, function) in module.functions.iter().enumerate() {
        if options.overflow_mode == OverflowMode::Checked {
            emit_checked_c_function(&mut out, function, options.opt_level);
        } else {
            emit_c_function(&mut out, function);
        }
        if index + 1 < module.functions.len() {
            out.push('\n');
        }
    }
    out
}

#[must_use]
pub fn emit_c_module_with_header(
    module: &MirModule,
    options: EmitCOptions,
    header_file_name: &str,
) -> String {
    if use_planned_c_emitter(module, options) {
        return emit_planned_c_module(module, options, Some(header_file_name));
    }
    let mut out = String::new();
    out.push_str(&format!(
        "#include \"{}\"\n\n",
        escape_c_include_path(header_file_name)
    ));

    for (index, function) in module.functions.iter().enumerate() {
        if options.overflow_mode == OverflowMode::Checked {
            emit_checked_c_function(&mut out, function, options.opt_level);
        } else {
            emit_c_function(&mut out, function);
        }
        if index + 1 < module.functions.len() {
            out.push('\n');
        }
    }
    out
}

#[must_use]
pub fn emit_c_header(module: &MirModule, options: EmitCOptions) -> String {
    if use_planned_c_emitter(module, options) {
        return emit_planned_c_header(module, options);
    }
    let mut out = String::new();
    out.push_str("#pragma once\n\n");
    out.push_str("#include <stdint.h>\n#include <stdbool.h>\n");
    if options.overflow_mode == OverflowMode::Checked {
        out.push_str("#include <stddef.h>\n");
    }
    out.push_str(
        "\n#if defined(_WIN32) || defined(__CYGWIN__)\n  #ifdef CK_BUILD_DLL\n    #define CK_API __declspec(dllexport)\n  #else\n    #define CK_API __declspec(dllimport)\n  #endif\n#else\n  #define CK_API __attribute__((visibility(\"default\")))\n#endif\n",
    );
    if options.overflow_mode == OverflowMode::Checked {
        out.push_str(
            "\ntypedef int32_t CK_Status;\n\n#define CK_OK ((CK_Status)0)\n#define CK_ERR_OVERFLOW ((CK_Status)1)\n#define CK_ERR_DIV_BY_ZERO ((CK_Status)2)\n#define CK_ERR_NULL_POINTER ((CK_Status)3)\n#define CK_ERR_OUT_OF_BOUNDS ((CK_Status)4)\n",
        );
    }
    out.push_str("\n#ifdef __cplusplus\nextern \"C\" {\n#endif\n");

    for struct_info in &module.structs {
        out.push_str(&format!("\ntypedef struct {} {{\n", struct_info.name));
        for field in &struct_info.fields {
            out.push_str(&format!("  {} {};\n", c_type(&field.type_node), field.name));
        }
        out.push_str(&format!("}} {};\n", struct_info.name));
    }

    for function in module.functions.iter().filter(|function| function.exported) {
        let signature = if options.overflow_mode == OverflowMode::Checked {
            c_export_signature_checked(function)
        } else {
            c_export_signature(function)
        };
        out.push_str(&format!("\nCK_API {signature};\n"));
    }

    out.push_str("\n#ifdef __cplusplus\n}\n#endif\n");
    out
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmitLlvmOptions {
    pub source_file_name: Option<String>,
    pub target_triple: Option<String>,
}

#[must_use]
pub fn emit_llvm_module(module: &MirModule, options: &EmitLlvmOptions) -> String {
    let mut out = String::new();
    out.push_str("; ModuleID = 'calckernel'\n");
    out.push_str(&format!(
        "source_filename = \"{}\"\n",
        llvm_escape_string(&llvm_source_file_name(options.source_file_name.as_deref()))
    ));
    if let Some(target) = &options.target_triple {
        out.push_str(&format!(
            "target triple = \"{}\"\n",
            llvm_escape_string(target)
        ));
    }
    if !module.structs.is_empty() || !module.functions.is_empty() {
        out.push('\n');
    }

    for struct_info in &module.structs {
        out.push_str(&format!(
            "%struct.{} = type {{ {} }}\n\n",
            struct_info.name,
            struct_info
                .fields
                .iter()
                .map(|field| llvm_storage_type(&field.type_node))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    let layout = LlvmStructLayout::new(module);
    for (index, function) in module.functions.iter().enumerate() {
        emit_llvm_function(&mut out, function, &layout);
        if index + 1 < module.functions.len() {
            out.push('\n');
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EmitWasmOptions {
    pub opt_level: u8,
}

#[must_use]
pub fn emit_wat_module(module: &MirModule) -> String {
    emit_wat_module_with_options(module, EmitWasmOptions::default())
}

#[must_use]
pub fn emit_wat_module_with_options(module: &MirModule, options: EmitWasmOptions) -> String {
    let layout = WasmStructLayout::new(module);
    let mut out = String::new();
    out.push_str("(module\n");
    out.push_str("  (memory (export \"memory\") 1)\n");
    out.push_str("  (global (export \"__ck_heap_base\") i32 (i32.const 0))\n");
    for function in &module.functions {
        out.push('\n');
        emit_wat_function(&mut out, function, &layout, options);
    }
    out.push_str(")\n");
    out
}

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

#[derive(Debug, Clone)]
struct WasmFieldLayout {
    offset: usize,
}

#[derive(Debug, Clone)]
struct WasmStructLayout {
    fields: std::collections::HashMap<String, std::collections::HashMap<String, WasmFieldLayout>>,
    sizes: std::collections::HashMap<String, usize>,
}

impl WasmStructLayout {
    fn new(module: &MirModule) -> Self {
        let mut fields = std::collections::HashMap::new();
        let mut sizes = std::collections::HashMap::new();
        for struct_info in &module.structs {
            let mut offset = 0;
            let mut align = 1;
            let mut field_map = std::collections::HashMap::new();
            for field in &struct_info.fields {
                let field_align = wasm_align_of(&field.type_node, &sizes);
                let field_size = wasm_size_of(&field.type_node, &sizes);
                offset = align_to(offset, field_align);
                field_map.insert(field.name.clone(), WasmFieldLayout { offset });
                offset += field_size;
                align = align.max(field_align);
            }
            fields.insert(struct_info.name.clone(), field_map);
            sizes.insert(struct_info.name.clone(), align_to(offset, align));
        }
        Self { fields, sizes }
    }

    fn field_offset(&self, struct_name: &str, field_name: &str) -> usize {
        self.fields
            .get(struct_name)
            .and_then(|fields| fields.get(field_name))
            .map(|field| field.offset)
            .unwrap_or_else(|| panic!("unknown WASM struct field {struct_name}.{field_name}"))
    }

    fn size_of(&self, type_node: &MirType) -> usize {
        wasm_size_of(type_node, &self.sizes)
    }

    fn align_of(&self, type_node: &MirType) -> usize {
        wasm_align_of(type_node, &self.sizes)
    }
}

fn emit_wat_function(
    out: &mut String,
    function: &MirFunction,
    layout: &WasmStructLayout,
    options: EmitWasmOptions,
) {
    if wasm_function_uses_slices(function) {
        emit_wat_slice_function(out, function, layout, options);
        return;
    }
    let export = if function.exported {
        format!(" (export \"{}\")", function.name)
    } else {
        String::new()
    };
    out.push_str(&format!("  (func ${}{}\n", function.name, export));
    for param in &function.params {
        out.push_str(&format!(
            "    (param ${} {})\n",
            param.name,
            wasm_type(&param.type_node)
        ));
    }
    if !matches!(function.return_type, MirType::Void) {
        out.push_str(&format!(
            "    (result {})\n",
            wasm_type(&function.return_type)
        ));
    }

    let mut locals = HashSet::new();
    for local in &function.locals {
        if locals.insert(local.name.clone()) {
            out.push_str(&format!(
                "    (local ${} {})\n",
                local.name,
                wasm_type(&local.type_node)
            ));
        }
    }
    for (name, type_node) in collect_temps(function) {
        if locals.insert(name.clone()) {
            out.push_str(&format!("    (local ${name} {})\n", wasm_type(&type_node)));
        }
    }

    if function.blocks.len() == 1 {
        for instruction in &function.blocks[0].instructions {
            emit_wat_instruction(out, instruction, layout, 4);
        }
        emit_wat_terminator(out, &function.blocks[0].terminator, None, 4);
    } else if let Some(loop_context) = (options.opt_level >= 3)
        .then(|| detect_simple_wasm_while(function))
        .flatten()
    {
        emit_structured_wasm_while(out, &loop_context, layout);
    } else {
        emit_dispatched_wasm_function(out, function, layout);
    }
    out.push_str("  )\n");
}

#[derive(Debug, Clone)]
enum WasmPhysicalValue {
    Scalar(String),
    Slice { data: String, len: String },
}

#[derive(Debug)]
struct WasmFunctionPlan {
    values: HashMap<String, WasmPhysicalValue>,
    block_local: String,
    return_scalar: String,
    return_data: String,
    return_len: String,
    address_local: String,
}

impl WasmFunctionPlan {
    fn new(function: &MirFunction) -> Self {
        let mut names = CIdentifierAllocator::default();
        for param in &function.params {
            names.reserve(&param.name);
        }
        for local in &function.locals {
            names.reserve(&local.name);
        }
        for block in &function.blocks {
            names.reserve(&block.label);
        }

        let mut values = HashMap::new();
        for param in &function.params {
            values.insert(
                format!("param:{}", param.name),
                wasm_allocate_physical_value(&mut names, &param.name, &param.type_node),
            );
        }
        for local in &function.locals {
            values.insert(
                format!("local:{}", local.name),
                wasm_allocate_physical_value(&mut names, &local.name, &local.type_node),
            );
        }
        for (name, type_node) in collect_temps(function) {
            let physical = if matches!(type_node, MirType::Slice(_)) {
                wasm_allocate_physical_value(&mut names, &name, &type_node)
            } else {
                WasmPhysicalValue::Scalar(names.allocate(&name))
            };
            values.insert(format!("temp:{name}"), physical);
        }

        Self {
            values,
            block_local: names.allocate("ik_bb"),
            return_scalar: names.allocate("ik_ret"),
            return_data: names.allocate("ik_ret_data"),
            return_len: names.allocate("ik_ret_len"),
            address_local: names.allocate("ik_addr"),
        }
    }

    fn value(&self, value: &MirValue) -> &WasmPhysicalValue {
        self.values
            .get(&c_value_identity(value))
            .expect("every named MIR value must have physical WASM names")
    }

    fn place_value(&self, place: &MirPlace) -> &WasmPhysicalValue {
        let key = match place {
            MirPlace::Param { name, .. } => format!("param:{name}"),
            MirPlace::Local { name, .. } => format!("local:{name}"),
            _ => panic!("only param and local places have direct WASM local names"),
        };
        self.values
            .get(&key)
            .expect("every named MIR place must have physical WASM names")
    }

    fn scalar(&self, value: &MirValue) -> &str {
        match self.value(value) {
            WasmPhysicalValue::Scalar(name) => name,
            WasmPhysicalValue::Slice { .. } => panic!("slice is not a scalar WASM value"),
        }
    }

    fn slice(&self, value: &MirValue) -> (&str, &str) {
        match self.value(value) {
            WasmPhysicalValue::Slice { data, len } => (data, len),
            WasmPhysicalValue::Scalar(_) => panic!("scalar is not a paired WASM slice value"),
        }
    }
}

fn wasm_allocate_physical_value(
    names: &mut CIdentifierAllocator,
    logical_name: &str,
    type_node: &MirType,
) -> WasmPhysicalValue {
    if matches!(type_node, MirType::Slice(_)) {
        WasmPhysicalValue::Slice {
            data: names.allocate(&format!("{logical_name}_data")),
            len: names.allocate(&format!("{logical_name}_len")),
        }
    } else {
        WasmPhysicalValue::Scalar(logical_name.to_string())
    }
}

fn wasm_function_uses_slices(function: &MirFunction) -> bool {
    let mut slices = HashSet::new();
    for param in &function.params {
        collect_slice_types_from_type(&param.type_node, &mut slices);
    }
    collect_slice_types_from_type(&function.return_type, &mut slices);
    for local in &function.locals {
        collect_slice_types_from_type(&local.type_node, &mut slices);
    }
    for block in &function.blocks {
        for instruction in &block.instructions {
            collect_slice_types_from_instruction(instruction, &mut slices);
        }
    }
    !slices.is_empty()
}

fn emit_wat_slice_function(
    out: &mut String,
    function: &MirFunction,
    layout: &WasmStructLayout,
    options: EmitWasmOptions,
) {
    let plan = WasmFunctionPlan::new(function);
    let export = if function.exported {
        format!(" (export \"{}\")", function.name)
    } else {
        String::new()
    };
    out.push_str(&format!("  (func ${}{}\n", function.name, export));
    for param in &function.params {
        match plan
            .values
            .get(&format!("param:{}", param.name))
            .expect("param must have physical WASM names")
        {
            WasmPhysicalValue::Scalar(name) => out.push_str(&format!(
                "    (param ${name} {})\n",
                wasm_type(&param.type_node)
            )),
            WasmPhysicalValue::Slice { data, len } => {
                out.push_str(&format!("    (param ${data} i32)\n"));
                out.push_str(&format!("    (param ${len} i32)\n"));
            }
        }
    }
    match &function.return_type {
        MirType::Void => {}
        MirType::Slice(_) => out.push_str("    (result i32 i32)\n"),
        type_node => out.push_str(&format!("    (result {})\n", wasm_type(type_node))),
    }

    for local in &function.locals {
        emit_wat_physical_local(
            out,
            plan.values
                .get(&format!("local:{}", local.name))
                .expect("local must have physical WASM names"),
            &local.type_node,
        );
    }
    for (name, type_node) in collect_temps(function) {
        emit_wat_physical_local(
            out,
            plan.values
                .get(&format!("temp:{name}"))
                .expect("temp must have physical WASM names"),
            &type_node,
        );
    }
    out.push_str(&format!("    (local ${} i32)\n", plan.address_local));

    if function.blocks.len() == 1 {
        for instruction in &function.blocks[0].instructions {
            emit_wat_paired_instruction(out, instruction, layout, &plan, 4);
        }
        emit_wat_paired_terminator(out, &function.blocks[0].terminator, None, &plan, 4);
    } else if let Some(loop_context) = (options.opt_level >= 3)
        .then(|| detect_simple_wasm_while(function))
        .flatten()
    {
        emit_structured_wasm_while_paired(out, &loop_context, layout, &plan);
    } else {
        emit_dispatched_wasm_function_paired(out, function, layout, &plan);
    }
    out.push_str("  )\n");
}

fn emit_wat_physical_local(out: &mut String, value: &WasmPhysicalValue, type_node: &MirType) {
    match value {
        WasmPhysicalValue::Scalar(name) => {
            out.push_str(&format!("    (local ${name} {})\n", wasm_type(type_node)));
        }
        WasmPhysicalValue::Slice { data, len } => {
            out.push_str(&format!("    (local ${data} i32)\n"));
            out.push_str(&format!("    (local ${len} i32)\n"));
        }
    }
}

fn emit_structured_wasm_while_paired(
    out: &mut String,
    loop_context: &StructuredWasmWhile<'_>,
    layout: &WasmStructLayout,
    plan: &WasmFunctionPlan,
) {
    for instruction in &loop_context.entry.instructions {
        emit_wat_paired_instruction(out, instruction, layout, plan, 4);
    }
    out.push_str(&format!("    block ${}\n", loop_context.exit_label));
    out.push_str(&format!("      loop ${}\n", loop_context.loop_label));
    for instruction in &loop_context.header.instructions {
        emit_wat_paired_instruction(out, instruction, layout, plan, 8);
    }
    let MirTerminator::Branch { condition, .. } = &loop_context.header.terminator else {
        unreachable!("structured while header is always a branch")
    };
    emit_wat_paired_scalar_value(out, condition, plan, 8);
    out.push_str("        i32.eqz\n");
    out.push_str(&format!("        br_if ${}\n", loop_context.exit_label));
    for instruction in &loop_context.body.instructions {
        emit_wat_paired_instruction(out, instruction, layout, plan, 8);
    }
    out.push_str(&format!("        br ${}\n", loop_context.loop_label));
    out.push_str("      end\n    end\n");
    for instruction in &loop_context.exit.instructions {
        emit_wat_paired_instruction(out, instruction, layout, plan, 4);
    }
    emit_wat_paired_terminator(out, &loop_context.exit.terminator, None, plan, 4);
}

fn emit_dispatched_wasm_function_paired(
    out: &mut String,
    function: &MirFunction,
    layout: &WasmStructLayout,
    plan: &WasmFunctionPlan,
) {
    out.push_str(&format!("    (local ${} i32)\n", plan.block_local));
    match &function.return_type {
        MirType::Void => {}
        MirType::Slice(_) => {
            out.push_str(&format!("    (local ${} i32)\n", plan.return_data));
            out.push_str(&format!("    (local ${} i32)\n", plan.return_len));
        }
        type_node => out.push_str(&format!(
            "    (local ${} {})\n",
            plan.return_scalar,
            wasm_type(type_node)
        )),
    }
    out.push_str("    i32.const 0\n");
    out.push_str(&format!("    local.set ${}\n", plan.block_local));
    out.push_str("    block $ik_exit\n      loop $ik_dispatch\n");
    for index in 0..function.blocks.len() {
        out.push_str(&format!(
            "{}block $ik_case{index}\n",
            " ".repeat(8 + index * 2)
        ));
    }
    let case_labels = (0..function.blocks.len())
        .map(|index| format!("$ik_case{index}"))
        .collect::<Vec<_>>()
        .join(" ");
    let dispatch_indent = " ".repeat(8 + function.blocks.len() * 2);
    out.push_str(&format!(
        "{dispatch_indent}local.get ${}\n{dispatch_indent}br_table {case_labels} $ik_case0\n",
        plan.block_local
    ));
    for index in (0..function.blocks.len()).rev() {
        let block_indent = 8 + index * 2;
        out.push_str(&format!("{}end\n", " ".repeat(block_indent)));
        let block = &function.blocks[index];
        for instruction in &block.instructions {
            emit_wat_paired_instruction(out, instruction, layout, plan, block_indent);
        }
        emit_wat_paired_terminator(out, &block.terminator, Some(function), plan, block_indent);
    }
    out.push_str("      end\n    end\n");
    match &function.return_type {
        MirType::Void => {}
        MirType::Slice(_) => out.push_str(&format!(
            "    local.get ${}\n    local.get ${}\n",
            plan.return_data, plan.return_len
        )),
        _ => out.push_str(&format!("    local.get ${}\n", plan.return_scalar)),
    }
}

fn emit_wat_paired_instruction(
    out: &mut String,
    instruction: &MirInstruction,
    layout: &WasmStructLayout,
    plan: &WasmFunctionPlan,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    match instruction {
        MirInstruction::ConstInt { target, value } => out.push_str(&format!(
            "{pad}{}.const {value}\n{pad}local.set ${}\n",
            wasm_type(value_type(target)),
            plan.scalar(target)
        )),
        MirInstruction::ConstFloat { target, value } => out.push_str(&format!(
            "{pad}f64.const {value}\n{pad}local.set ${}\n",
            plan.scalar(target)
        )),
        MirInstruction::ConstBool { target, value } => out.push_str(&format!(
            "{pad}i32.const {}\n{pad}local.set ${}\n",
            if *value { 1 } else { 0 },
            plan.scalar(target)
        )),
        MirInstruction::Move { target, value }
            if matches!(value_type(target), MirType::Slice(_)) =>
        {
            let (target_data, target_len) = plan.slice(target);
            let (value_data, value_len) = plan.slice(value);
            out.push_str(&format!(
                "{pad}local.get ${value_data}\n{pad}local.set ${target_data}\n\
                 {pad}local.get ${value_len}\n{pad}local.set ${target_len}\n"
            ));
        }
        MirInstruction::Move { target, value } => {
            emit_wat_paired_scalar_value(out, value, plan, indent);
            out.push_str(&format!("{pad}local.set ${}\n", plan.scalar(target)));
        }
        MirInstruction::Binary {
            target,
            op,
            left,
            right,
        } => {
            emit_wat_paired_scalar_value(out, left, plan, indent);
            emit_wat_paired_scalar_value(out, right, plan, indent);
            out.push_str(&format!(
                "{pad}{}\n{pad}local.set ${}\n",
                wat_binary_instruction(*op, value_type(left)),
                plan.scalar(target)
            ));
        }
        MirInstruction::Unary {
            target,
            op,
            operand,
        } => emit_wat_paired_unary(out, *op, operand, target, plan, indent),
        MirInstruction::Compare {
            target,
            op,
            left,
            right,
        } => {
            emit_wat_paired_scalar_value(out, left, plan, indent);
            emit_wat_paired_scalar_value(out, right, plan, indent);
            out.push_str(&format!(
                "{pad}{}\n{pad}local.set ${}\n",
                wat_compare_instruction(*op, value_type(left)),
                plan.scalar(target)
            ));
        }
        MirInstruction::Cast { target, op, value } => {
            emit_wat_paired_scalar_value(out, value, plan, indent);
            let opcode = match op {
                MirCastOp::I32ToF64 => "f64.convert_i32_s",
                MirCastOp::U32ToF64 => "f64.convert_i32_u",
            };
            out.push_str(&format!(
                "{pad}{opcode}\n{pad}local.set ${}\n",
                plan.scalar(target)
            ));
        }
        MirInstruction::Address { target, place } => {
            emit_wat_paired_address(out, place, layout, plan, indent);
            out.push_str(&format!("{pad}local.set ${}\n", plan.scalar(target)));
        }
        MirInstruction::Load { target, place }
            if matches!(value_type(target), MirType::Slice(_)) =>
        {
            let (data, len) = plan.slice(target);
            emit_wat_paired_address(out, place, layout, plan, indent);
            out.push_str(&format!(
                "{pad}local.set ${}\n\
                 {pad}local.get ${}\n{pad}i32.load offset=0 align=4\n{pad}local.set ${data}\n\
                 {pad}local.get ${}\n{pad}i32.load offset=4 align=4\n{pad}local.set ${len}\n",
                plan.address_local, plan.address_local, plan.address_local
            ));
        }
        MirInstruction::Load { target, place } => {
            emit_wat_paired_address(out, place, layout, plan, indent);
            out.push_str(&format!(
                "{pad}{}.load offset=0 align={}\n{pad}local.set ${}\n",
                wasm_type(value_type(target)),
                layout.align_of(value_type(target)),
                plan.scalar(target)
            ));
        }
        MirInstruction::Store { place, value }
            if matches!(value_type(value), MirType::Slice(_)) =>
        {
            let (data, len) = plan.slice(value);
            emit_wat_paired_address(out, place, layout, plan, indent);
            out.push_str(&format!(
                "{pad}local.set ${}\n\
                 {pad}local.get ${}\n{pad}local.get ${data}\n{pad}i32.store offset=0 align=4\n\
                 {pad}local.get ${}\n{pad}local.get ${len}\n{pad}i32.store offset=4 align=4\n",
                plan.address_local, plan.address_local, plan.address_local
            ));
        }
        MirInstruction::Store { place, value } => {
            emit_wat_paired_address(out, place, layout, plan, indent);
            emit_wat_paired_scalar_value(out, value, plan, indent);
            out.push_str(&format!(
                "{pad}{}.store offset=0 align={}\n",
                wasm_type(value_type(value)),
                layout.align_of(value_type(value))
            ));
        }
        MirInstruction::MakeSlice { target, data, len } => {
            let (target_data, target_len) = plan.slice(target);
            emit_wat_paired_scalar_value(out, data, plan, indent);
            out.push_str(&format!("{pad}local.set ${target_data}\n"));
            emit_wat_paired_scalar_value(out, len, plan, indent);
            out.push_str(&format!("{pad}local.set ${target_len}\n"));
        }
        MirInstruction::SliceData { target, slice } => {
            let (data, _) = plan.slice(slice);
            out.push_str(&format!(
                "{pad}local.get ${data}\n{pad}local.set ${}\n",
                plan.scalar(target)
            ));
        }
        MirInstruction::SliceLen { target, slice } => {
            let (_, len) = plan.slice(slice);
            out.push_str(&format!(
                "{pad}local.get ${len}\n{pad}local.set ${}\n",
                plan.scalar(target)
            ));
        }
        MirInstruction::Subslice {
            target,
            slice,
            start,
            end,
        } => {
            let (target_data, target_len) = plan.slice(target);
            let (slice_data, _) = plan.slice(slice);
            let MirType::Slice(element_type) = value_type(slice) else {
                unreachable!("subslice source must be a slice")
            };
            out.push_str(&format!("{pad}local.get ${slice_data}\n"));
            emit_wat_paired_scalar_value(out, start, plan, indent);
            out.push_str(&format!(
                "{pad}i32.const {}\n{pad}i32.mul\n{pad}i32.add\n{pad}local.set ${target_data}\n",
                layout.size_of(element_type)
            ));
            emit_wat_paired_scalar_value(out, end, plan, indent);
            emit_wat_paired_scalar_value(out, start, plan, indent);
            out.push_str(&format!("{pad}i32.sub\n{pad}local.set ${target_len}\n"));
        }
        MirInstruction::Call {
            target,
            function_name,
            args,
        } => {
            for arg in args {
                if matches!(value_type(arg), MirType::Slice(_)) {
                    emit_wat_paired_slice_value(out, arg, plan, indent);
                } else {
                    emit_wat_paired_scalar_value(out, arg, plan, indent);
                }
            }
            out.push_str(&format!("{pad}call ${function_name}\n"));
            if let Some(target) = target {
                match plan.value(target) {
                    WasmPhysicalValue::Scalar(name) => {
                        out.push_str(&format!("{pad}local.set ${name}\n"));
                    }
                    WasmPhysicalValue::Slice { data, len } => {
                        out.push_str(&format!("{pad}local.set ${len}\n{pad}local.set ${data}\n"))
                    }
                }
            }
        }
    }
}

fn emit_wat_paired_scalar_value(
    out: &mut String,
    value: &MirValue,
    plan: &WasmFunctionPlan,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    match value {
        MirValue::Param { .. } | MirValue::Local { .. } | MirValue::Temp { .. } => {
            out.push_str(&format!("{pad}local.get ${}\n", plan.scalar(value)));
        }
        MirValue::ConstInt { text, type_node } => {
            out.push_str(&format!("{pad}{}.const {text}\n", wasm_type(type_node)));
        }
        MirValue::ConstFloat { text, .. } => out.push_str(&format!("{pad}f64.const {text}\n")),
        MirValue::ConstBool { value, .. } => {
            out.push_str(&format!("{pad}i32.const {}\n", if *value { 1 } else { 0 }))
        }
    }
}

fn emit_wat_paired_slice_value(
    out: &mut String,
    value: &MirValue,
    plan: &WasmFunctionPlan,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    let (data, len) = plan.slice(value);
    out.push_str(&format!("{pad}local.get ${data}\n{pad}local.get ${len}\n"));
}

fn emit_wat_paired_unary(
    out: &mut String,
    op: MirUnaryOp,
    operand: &MirValue,
    target: &MirValue,
    plan: &WasmFunctionPlan,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    match op {
        MirUnaryOp::Not => {
            emit_wat_paired_scalar_value(out, operand, plan, indent);
            out.push_str(&format!(
                "{pad}i32.eqz\n{pad}local.set ${}\n",
                plan.scalar(target)
            ));
        }
        MirUnaryOp::Neg if is_f64_type(value_type(operand)) => {
            emit_wat_paired_scalar_value(out, operand, plan, indent);
            out.push_str(&format!(
                "{pad}f64.neg\n{pad}local.set ${}\n",
                plan.scalar(target)
            ));
        }
        MirUnaryOp::Neg => {
            out.push_str(&format!(
                "{pad}{}.const 0\n",
                wasm_type(value_type(operand))
            ));
            emit_wat_paired_scalar_value(out, operand, plan, indent);
            out.push_str(&format!(
                "{pad}{}.sub\n{pad}local.set ${}\n",
                wasm_type(value_type(operand)),
                plan.scalar(target)
            ));
        }
    }
}

fn emit_wat_paired_address(
    out: &mut String,
    place: &MirPlace,
    layout: &WasmStructLayout,
    plan: &WasmFunctionPlan,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    match place {
        MirPlace::Param { .. } | MirPlace::Local { .. } => match plan.place_value(place) {
            WasmPhysicalValue::Scalar(name) => {
                out.push_str(&format!("{pad}local.get ${name}\n"));
            }
            WasmPhysicalValue::Slice { .. } => {
                panic!("a logical slice local is not directly addressable")
            }
        },
        MirPlace::Deref { pointer, .. } => emit_wat_paired_scalar_value(out, pointer, plan, indent),
        MirPlace::Index { base, index, .. } => {
            let MirType::Pointer(element_type) = place_type(base) else {
                panic!("WAT index base must be pointer");
            };
            emit_wat_paired_address(out, base, layout, plan, indent);
            emit_wat_paired_scalar_value(out, index, plan, indent);
            out.push_str(&format!(
                "{pad}i32.const {}\n{pad}i32.mul\n{pad}i32.add\n",
                layout.size_of(element_type)
            ));
        }
        MirPlace::SliceIndex { slice, index, .. } => {
            let (data, _) = plan.slice(slice);
            let MirType::Slice(element_type) = value_type(slice) else {
                unreachable!("slice index base must be a slice")
            };
            out.push_str(&format!("{pad}local.get ${data}\n"));
            emit_wat_paired_scalar_value(out, index, plan, indent);
            out.push_str(&format!(
                "{pad}i32.const {}\n{pad}i32.mul\n{pad}i32.add\n",
                layout.size_of(element_type)
            ));
        }
        MirPlace::Field {
            base, field_name, ..
        } => {
            let MirType::Struct(struct_name) = place_type(base) else {
                panic!("WAT field base must be struct");
            };
            emit_wat_paired_address(out, base, layout, plan, indent);
            let offset = layout.field_offset(struct_name, field_name);
            if offset != 0 {
                out.push_str(&format!("{pad}i32.const {offset}\n{pad}i32.add\n"));
            }
        }
    }
}

fn emit_wat_paired_terminator(
    out: &mut String,
    terminator: &MirTerminator,
    function: Option<&MirFunction>,
    plan: &WasmFunctionPlan,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    match terminator {
        MirTerminator::Return { value } => {
            if let Some(value) = value {
                if matches!(value_type(value), MirType::Slice(_)) {
                    emit_wat_paired_slice_value(out, value, plan, indent);
                    if function.is_some() {
                        out.push_str(&format!(
                            "{pad}local.set ${}\n{pad}local.set ${}\n{pad}br $ik_exit\n",
                            plan.return_len, plan.return_data
                        ));
                    } else {
                        out.push_str(&format!("{pad}return\n"));
                    }
                } else {
                    emit_wat_paired_scalar_value(out, value, plan, indent);
                    if function.is_some() {
                        out.push_str(&format!(
                            "{pad}local.set ${}\n{pad}br $ik_exit\n",
                            plan.return_scalar
                        ));
                    } else {
                        out.push_str(&format!("{pad}return\n"));
                    }
                }
            } else if function.is_some() {
                out.push_str(&format!("{pad}br $ik_exit\n"));
            } else {
                out.push_str(&format!("{pad}return\n"));
            }
        }
        MirTerminator::Jump { label } => {
            let index = block_index(function.expect("dispatcher function"), label);
            out.push_str(&format!(
                "{pad}i32.const {index}\n{pad}local.set ${}\n{pad}br $ik_dispatch\n",
                plan.block_local
            ));
        }
        MirTerminator::Branch {
            condition,
            then_label,
            else_label,
        } => {
            let function = function.expect("dispatcher function");
            emit_wat_paired_scalar_value(out, condition, plan, indent);
            out.push_str(&format!(
                "{pad}if\n{pad}  i32.const {}\n{pad}  local.set ${}\n{pad}else\n{pad}  i32.const {}\n{pad}  local.set ${}\n{pad}end\n{pad}br $ik_dispatch\n",
                block_index(function, then_label),
                plan.block_local,
                block_index(function, else_label),
                plan.block_local
            ));
        }
    }
}

struct StructuredWasmWhile<'a> {
    entry: &'a MirBlock,
    header: &'a MirBlock,
    body: &'a MirBlock,
    exit: &'a MirBlock,
    loop_label: String,
    exit_label: String,
}

fn detect_simple_wasm_while(function: &MirFunction) -> Option<StructuredWasmWhile<'_>> {
    if function.blocks.len() != 4 {
        return None;
    }

    let entry = function.blocks.first()?;
    let MirTerminator::Jump {
        label: header_label,
    } = &entry.terminator
    else {
        return None;
    };

    let header = wasm_block_by_label(function, header_label)?;
    let MirTerminator::Branch {
        then_label,
        else_label,
        ..
    } = &header.terminator
    else {
        return None;
    };

    let body = wasm_block_by_label(function, then_label)?;
    let exit = wasm_block_by_label(function, else_label)?;
    if !matches!(&body.terminator, MirTerminator::Jump { label } if label == &header.label)
        || !matches!(exit.terminator, MirTerminator::Return { .. })
    {
        return None;
    }

    let matched_labels = [
        entry.label.as_str(),
        header.label.as_str(),
        body.label.as_str(),
        exit.label.as_str(),
    ]
    .into_iter()
    .collect::<HashSet<_>>();
    if matched_labels.len() != function.blocks.len() {
        return None;
    }

    let mut used_names = collect_wasm_function_names(function);
    let loop_label = unique_wasm_internal_name("ik_loop", &mut used_names);
    let exit_label = unique_wasm_internal_name("ik_exit", &mut used_names);
    Some(StructuredWasmWhile {
        entry,
        header,
        body,
        exit,
        loop_label,
        exit_label,
    })
}

fn wasm_block_by_label<'a>(function: &'a MirFunction, label: &str) -> Option<&'a MirBlock> {
    function.blocks.iter().find(|block| block.label == label)
}

fn collect_wasm_function_names(function: &MirFunction) -> HashSet<String> {
    let mut names = HashSet::new();
    for param in &function.params {
        names.insert(param.name.clone());
    }
    for local in &function.locals {
        names.insert(local.name.clone());
    }
    for block in &function.blocks {
        names.insert(block.label.clone());
        for instruction in &block.instructions {
            if let Some(MirValue::Temp { name, .. }) = instruction_target(instruction) {
                names.insert(name.clone());
            }
        }
    }
    names
}

fn unique_wasm_internal_name(base_name: &str, used_names: &mut HashSet<String>) -> String {
    if used_names.insert(base_name.to_string()) {
        return base_name.to_string();
    }

    for index in 0.. {
        let candidate = format!("{base_name}{index}");
        if used_names.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!("unbounded internal name search should always find a name")
}

fn emit_structured_wasm_while(
    out: &mut String,
    loop_context: &StructuredWasmWhile<'_>,
    layout: &WasmStructLayout,
) {
    for instruction in &loop_context.entry.instructions {
        emit_wat_instruction(out, instruction, layout, 4);
    }
    out.push_str(&format!("    block ${}\n", loop_context.exit_label));
    out.push_str(&format!("      loop ${}\n", loop_context.loop_label));

    for instruction in &loop_context.header.instructions {
        emit_wat_instruction(out, instruction, layout, 8);
    }
    let MirTerminator::Branch { condition, .. } = &loop_context.header.terminator else {
        unreachable!("structured while header is always a branch")
    };
    emit_wat_value(out, condition, 8);
    out.push_str("        i32.eqz\n");
    out.push_str(&format!("        br_if ${}\n", loop_context.exit_label));

    for instruction in &loop_context.body.instructions {
        emit_wat_instruction(out, instruction, layout, 8);
    }
    out.push_str(&format!("        br ${}\n", loop_context.loop_label));
    out.push_str("      end\n");
    out.push_str("    end\n");

    for instruction in &loop_context.exit.instructions {
        emit_wat_instruction(out, instruction, layout, 4);
    }
    emit_wat_terminator(out, &loop_context.exit.terminator, None, 4);
}

fn emit_dispatched_wasm_function(
    out: &mut String,
    function: &MirFunction,
    layout: &WasmStructLayout,
) {
    out.push_str("    (local $ik_bb i32)\n");
    if !matches!(function.return_type, MirType::Void) {
        out.push_str(&format!(
            "    (local $ik_ret {})\n",
            wasm_type(&function.return_type)
        ));
    }
    out.push_str("    i32.const 0\n");
    out.push_str("    local.set $ik_bb\n");
    out.push_str("    block $ik_exit\n");
    out.push_str("      loop $ik_dispatch\n");
    for index in 0..function.blocks.len() {
        out.push_str(&format!(
            "{}block $ik_case{index}\n",
            " ".repeat(8 + index * 2)
        ));
    }
    let case_labels = (0..function.blocks.len())
        .map(|index| format!("$ik_case{index}"))
        .collect::<Vec<_>>()
        .join(" ");
    let dispatch_indent = " ".repeat(8 + function.blocks.len() * 2);
    out.push_str(&format!("{dispatch_indent}local.get $ik_bb\n"));
    out.push_str(&format!(
        "{dispatch_indent}br_table {case_labels} $ik_case0\n"
    ));
    for index in (0..function.blocks.len()).rev() {
        let block_indent = 8 + index * 2;
        out.push_str(&format!("{}end\n", " ".repeat(block_indent)));
        let block = &function.blocks[index];
        for instruction in &block.instructions {
            emit_wat_instruction(out, instruction, layout, block_indent);
        }
        emit_wat_terminator(out, &block.terminator, Some(function), block_indent);
    }
    out.push_str("      end\n");
    out.push_str("    end\n");
    if !matches!(function.return_type, MirType::Void) {
        out.push_str("    local.get $ik_ret\n");
    }
}

fn emit_wat_instruction(
    out: &mut String,
    instruction: &MirInstruction,
    layout: &WasmStructLayout,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    match instruction {
        MirInstruction::ConstInt { target, value } => {
            out.push_str(&format!(
                "{pad}{}.const {value}\n{pad}local.set ${}\n",
                wasm_type(value_type(target)),
                wat_local_name(target)
            ));
        }
        MirInstruction::ConstFloat { target, value } => {
            out.push_str(&format!(
                "{pad}f64.const {value}\n{pad}local.set ${}\n",
                wat_local_name(target)
            ));
        }
        MirInstruction::ConstBool { target, value } => {
            out.push_str(&format!(
                "{pad}i32.const {}\n{pad}local.set ${}\n",
                if *value { 1 } else { 0 },
                wat_local_name(target)
            ));
        }
        MirInstruction::Move { target, value } => {
            emit_wat_value(out, value, indent);
            out.push_str(&format!("{pad}local.set ${}\n", wat_local_name(target)));
        }
        MirInstruction::Binary {
            target,
            op,
            left,
            right,
        } => {
            emit_wat_value(out, left, indent);
            emit_wat_value(out, right, indent);
            out.push_str(&format!(
                "{pad}{}\n{pad}local.set ${}\n",
                wat_binary_instruction(*op, value_type(left)),
                wat_local_name(target)
            ));
        }
        MirInstruction::Unary {
            target,
            op,
            operand,
        } => {
            emit_wat_unary(out, *op, operand, target, indent);
        }
        MirInstruction::Compare {
            target,
            op,
            left,
            right,
        } => {
            emit_wat_value(out, left, indent);
            emit_wat_value(out, right, indent);
            out.push_str(&format!(
                "{pad}{}\n{pad}local.set ${}\n",
                wat_compare_instruction(*op, value_type(left)),
                wat_local_name(target)
            ));
        }
        MirInstruction::Cast { target, op, value } => {
            emit_wat_value(out, value, indent);
            let opcode = match op {
                MirCastOp::I32ToF64 => "f64.convert_i32_s",
                MirCastOp::U32ToF64 => "f64.convert_i32_u",
            };
            out.push_str(&format!(
                "{pad}{opcode}\n{pad}local.set ${}\n",
                wat_local_name(target)
            ));
        }
        MirInstruction::Address { target, place } => {
            emit_wat_address(out, place, layout, indent);
            out.push_str(&format!("{pad}local.set ${}\n", wat_local_name(target)));
        }
        MirInstruction::Load { target, place } => {
            emit_wat_address(out, place, layout, indent);
            out.push_str(&format!(
                "{pad}{}.load offset=0 align={}\n{pad}local.set ${}\n",
                wasm_type(value_type(target)),
                layout.align_of(value_type(target)),
                wat_local_name(target)
            ));
        }
        MirInstruction::Store { place, value } => {
            emit_wat_address(out, place, layout, indent);
            emit_wat_value(out, value, indent);
            out.push_str(&format!(
                "{pad}{}.store offset=0 align={}\n",
                wasm_type(value_type(value)),
                layout.align_of(value_type(value))
            ));
        }
        MirInstruction::Call {
            target,
            function_name,
            args,
        } => {
            for arg in args {
                emit_wat_value(out, arg, indent);
            }
            out.push_str(&format!("{pad}call ${function_name}\n"));
            if let Some(target) = target {
                out.push_str(&format!("{pad}local.set ${}\n", wat_local_name(target)));
            }
        }
        MirInstruction::MakeSlice { .. }
        | MirInstruction::SliceData { .. }
        | MirInstruction::SliceLen { .. }
        | MirInstruction::Subslice { .. } => {
            unreachable!("slice functions must use the paired WAT emitter")
        }
    }
}

fn emit_wat_terminator(
    out: &mut String,
    terminator: &MirTerminator,
    function: Option<&MirFunction>,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    match terminator {
        MirTerminator::Return { value } => {
            if let Some(value) = value {
                emit_wat_value(out, value, indent);
                if function.is_some() {
                    out.push_str(&format!("{pad}local.set $ik_ret\n{pad}br $ik_exit\n"));
                } else {
                    out.push_str(&format!("{pad}return\n"));
                }
            } else if function.is_some() {
                out.push_str(&format!("{pad}br $ik_exit\n"));
            } else {
                out.push_str(&format!("{pad}return\n"));
            }
        }
        MirTerminator::Jump { label } => {
            let index = block_index(function.expect("dispatcher function"), label);
            out.push_str(&format!(
                "{pad}i32.const {index}\n{pad}local.set $ik_bb\n{pad}br $ik_dispatch\n"
            ));
        }
        MirTerminator::Branch {
            condition,
            then_label,
            else_label,
        } => {
            let function = function.expect("dispatcher function");
            emit_wat_value(out, condition, indent);
            out.push_str(&format!(
                "{pad}if\n{pad}  i32.const {}\n{pad}  local.set $ik_bb\n{pad}else\n{pad}  i32.const {}\n{pad}  local.set $ik_bb\n{pad}end\n{pad}br $ik_dispatch\n",
                block_index(function, then_label),
                block_index(function, else_label)
            ));
        }
    }
}

fn emit_wat_value(out: &mut String, value: &MirValue, indent: usize) {
    let pad = " ".repeat(indent);
    match value {
        MirValue::Param { name, .. }
        | MirValue::Local { name, .. }
        | MirValue::Temp { name, .. } => {
            out.push_str(&format!("{pad}local.get ${name}\n"));
        }
        MirValue::ConstInt { text, type_node } => {
            out.push_str(&format!("{pad}{}.const {text}\n", wasm_type(type_node)));
        }
        MirValue::ConstFloat { text, .. } => {
            out.push_str(&format!("{pad}f64.const {text}\n"));
        }
        MirValue::ConstBool { value, .. } => {
            out.push_str(&format!("{pad}i32.const {}\n", if *value { 1 } else { 0 }));
        }
    }
}

fn emit_wat_unary(
    out: &mut String,
    op: MirUnaryOp,
    operand: &MirValue,
    target: &MirValue,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    match op {
        MirUnaryOp::Not => {
            emit_wat_value(out, operand, indent);
            out.push_str(&format!(
                "{pad}i32.eqz\n{pad}local.set ${}\n",
                wat_local_name(target)
            ));
        }
        MirUnaryOp::Neg if is_f64_type(value_type(operand)) => {
            emit_wat_value(out, operand, indent);
            out.push_str(&format!(
                "{pad}f64.neg\n{pad}local.set ${}\n",
                wat_local_name(target)
            ));
        }
        MirUnaryOp::Neg => {
            out.push_str(&format!(
                "{pad}{}.const 0\n",
                wasm_type(value_type(operand))
            ));
            emit_wat_value(out, operand, indent);
            out.push_str(&format!(
                "{pad}{}.sub\n{pad}local.set ${}\n",
                wasm_type(value_type(operand)),
                wat_local_name(target)
            ));
        }
    }
}

fn wat_local_name(value: &MirValue) -> &str {
    match value {
        MirValue::Param { name, .. }
        | MirValue::Local { name, .. }
        | MirValue::Temp { name, .. } => name,
        MirValue::ConstInt { .. } | MirValue::ConstFloat { .. } | MirValue::ConstBool { .. } => {
            panic!("WAT locals cannot be MIR constants")
        }
    }
}

fn emit_wat_address(out: &mut String, place: &MirPlace, layout: &WasmStructLayout, indent: usize) {
    let pad = " ".repeat(indent);
    match place {
        MirPlace::Param { name, .. } | MirPlace::Local { name, .. } => {
            out.push_str(&format!("{pad}local.get ${name}\n"));
        }
        MirPlace::Deref { pointer, .. } => emit_wat_value(out, pointer, indent),
        MirPlace::Index { base, index, .. } => {
            let MirType::Pointer(element_type) = place_type(base) else {
                panic!("WAT index base must be pointer");
            };
            emit_wat_address(out, base, layout, indent);
            emit_wat_value(out, index, indent);
            out.push_str(&format!(
                "{pad}i32.const {}\n{pad}i32.mul\n{pad}i32.add\n",
                layout.size_of(element_type)
            ));
        }
        MirPlace::SliceIndex { .. } => {
            unreachable!("slice functions must use the paired WAT emitter")
        }
        MirPlace::Field {
            base, field_name, ..
        } => {
            let MirType::Struct(struct_name) = place_type(base) else {
                panic!("WAT field base must be struct");
            };
            emit_wat_address(out, base, layout, indent);
            let offset = layout.field_offset(struct_name, field_name);
            if offset != 0 {
                out.push_str(&format!("{pad}i32.const {offset}\n{pad}i32.add\n"));
            }
        }
    }
}

fn wat_binary_instruction(op: MirBinaryOp, type_node: &MirType) -> String {
    if is_f64_type(type_node) {
        return match op {
            MirBinaryOp::Add => "f64.add".to_string(),
            MirBinaryOp::Sub => "f64.sub".to_string(),
            MirBinaryOp::Mul => "f64.mul".to_string(),
            MirBinaryOp::Div => "f64.div".to_string(),
            MirBinaryOp::Mod => panic!("WAT backend does not support f64 modulo"),
        };
    }
    let wasm = wasm_type(type_node);
    match op {
        MirBinaryOp::Add => format!("{wasm}.add"),
        MirBinaryOp::Sub => format!("{wasm}.sub"),
        MirBinaryOp::Mul => format!("{wasm}.mul"),
        MirBinaryOp::Div if is_unsigned_integer_type(type_node) => format!("{wasm}.div_u"),
        MirBinaryOp::Div => format!("{wasm}.div_s"),
        MirBinaryOp::Mod if is_unsigned_integer_type(type_node) => format!("{wasm}.rem_u"),
        MirBinaryOp::Mod => format!("{wasm}.rem_s"),
    }
}

fn wat_compare_instruction(op: MirCompareOp, type_node: &MirType) -> String {
    if is_f64_type(type_node) {
        return match op {
            MirCompareOp::Eq => "f64.eq",
            MirCompareOp::Ne => "f64.ne",
            MirCompareOp::Lt => "f64.lt",
            MirCompareOp::Le => "f64.le",
            MirCompareOp::Gt => "f64.gt",
            MirCompareOp::Ge => "f64.ge",
        }
        .to_string();
    }
    let wasm = wasm_type(type_node);
    match op {
        MirCompareOp::Eq => format!("{wasm}.eq"),
        MirCompareOp::Ne => format!("{wasm}.ne"),
        MirCompareOp::Lt if is_unsigned_integer_type(type_node) => format!("{wasm}.lt_u"),
        MirCompareOp::Lt => format!("{wasm}.lt_s"),
        MirCompareOp::Le if is_unsigned_integer_type(type_node) => format!("{wasm}.le_u"),
        MirCompareOp::Le => format!("{wasm}.le_s"),
        MirCompareOp::Gt if is_unsigned_integer_type(type_node) => format!("{wasm}.gt_u"),
        MirCompareOp::Gt => format!("{wasm}.gt_s"),
        MirCompareOp::Ge if is_unsigned_integer_type(type_node) => format!("{wasm}.ge_u"),
        MirCompareOp::Ge => format!("{wasm}.ge_s"),
    }
}

fn wasm_type(type_node: &MirType) -> &'static str {
    match type_node {
        MirType::Primitive(
            MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32 | MirPrimitiveTypeName::Bool,
        )
        | MirType::Pointer(_) => "i32",
        MirType::Slice(_) => unreachable!("logical slices must be lowered to paired WASM values"),
        MirType::Primitive(MirPrimitiveTypeName::I64 | MirPrimitiveTypeName::U64) => "i64",
        MirType::Primitive(MirPrimitiveTypeName::F64) => "f64",
        MirType::Struct(_) => panic!("struct values are not WASM scalar values"),
        MirType::Void => panic!("void is not a WASM scalar value"),
    }
}

fn wasm_size_of(
    type_node: &MirType,
    struct_sizes: &std::collections::HashMap<String, usize>,
) -> usize {
    match type_node {
        MirType::Primitive(
            MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32 | MirPrimitiveTypeName::Bool,
        )
        | MirType::Pointer(_) => 4,
        MirType::Slice(_) => 8,
        MirType::Primitive(
            MirPrimitiveTypeName::I64 | MirPrimitiveTypeName::U64 | MirPrimitiveTypeName::F64,
        ) => 8,
        MirType::Struct(name) => *struct_sizes.get(name).unwrap_or(&0),
        MirType::Void => panic!("void has no WASM storage size"),
    }
}

fn wasm_align_of(
    type_node: &MirType,
    struct_sizes: &std::collections::HashMap<String, usize>,
) -> usize {
    match type_node {
        MirType::Slice(_) => 4,
        MirType::Struct(name) => struct_sizes.get(name).copied().unwrap_or(1).clamp(1, 8),
        _ => wasm_size_of(type_node, struct_sizes).clamp(1, 8),
    }
}

fn align_to(value: usize, align: usize) -> usize {
    if align == 0 {
        return value;
    }
    value.div_ceil(align) * align
}

fn block_index(function: &MirFunction, label: &str) -> usize {
    function
        .blocks
        .iter()
        .position(|block| block.label == label)
        .unwrap_or_else(|| panic!("unknown WAT block label {label}"))
}

#[derive(Debug, Clone)]
struct LlvmStructLayout {
    fields: std::collections::HashMap<String, std::collections::HashMap<String, usize>>,
}

impl LlvmStructLayout {
    fn new(module: &MirModule) -> Self {
        let fields = module
            .structs
            .iter()
            .map(|struct_info| {
                (
                    struct_info.name.clone(),
                    struct_info
                        .fields
                        .iter()
                        .enumerate()
                        .map(|(index, field)| (field.name.clone(), index))
                        .collect(),
                )
            })
            .collect();
        Self { fields }
    }

    fn field_index(&self, struct_name: &str, field_name: &str) -> usize {
        self.fields
            .get(struct_name)
            .and_then(|fields| fields.get(field_name))
            .copied()
            .unwrap_or_else(|| panic!("unknown LLVM struct field {struct_name}.{field_name}"))
    }
}

#[derive(Debug)]
struct LlvmFunctionContext<'layout> {
    register_counter: usize,
    used_value_names: HashSet<String>,
    layout: &'layout LlvmStructLayout,
}

fn emit_llvm_function(out: &mut String, function: &MirFunction, layout: &LlvmStructLayout) {
    let linkage = if function.exported { "" } else { "internal " };
    let params = function
        .params
        .iter()
        .flat_map(|param| match &param.type_node {
            MirType::Slice(_) => vec![
                format!("ptr %{}.data", param.name),
                format!("i32 %{}.len", param.name),
            ],
            _ => vec![format!(
                "{} %{}",
                llvm_param_type(&param.type_node),
                param.name
            )],
        })
        .collect::<Vec<_>>()
        .join(", ");
    out.push_str(&format!(
        "define {linkage}{} @{}({}) {{\n",
        llvm_return_type(&function.return_type),
        function.name,
        params
    ));

    let used_value_names = function
        .params
        .iter()
        .flat_map(|param| match param.type_node {
            MirType::Slice(_) => vec![
                format!("{}.data", param.name),
                format!("{}.len", param.name),
            ],
            _ => vec![param.name.clone()],
        })
        .collect();
    let mut context = LlvmFunctionContext {
        register_counter: 0,
        used_value_names,
        layout,
    };

    if function.blocks.is_empty() {
        out.push_str("entry:\n");
        if matches!(function.return_type, MirType::Void) {
            out.push_str("  ret void\n");
        } else {
            out.push_str(&format!(
                "  ret {} {}\n",
                llvm_return_type(&function.return_type),
                llvm_zero_value(&function.return_type)
            ));
        }
        out.push_str("}\n");
        return;
    }

    for (index, block) in function.blocks.iter().enumerate() {
        out.push_str(&format!("{}:\n", llvm_block_label(function, &block.label)));
        if index == 0 {
            emit_llvm_allocas(out, function);
            emit_llvm_param_stores(out, &mut context, function);
        }
        for instruction in &block.instructions {
            emit_llvm_instruction(out, &mut context, instruction);
        }
        emit_llvm_terminator(out, &mut context, function, &block.terminator);
    }
    out.push_str("}\n");
}

fn emit_llvm_allocas(out: &mut String, function: &MirFunction) {
    let mut emitted = HashSet::new();
    for param in &function.params {
        out.push_str(&format!(
            "  {} = alloca {}\n",
            llvm_address_name(&param.name),
            llvm_storage_type(&param.type_node)
        ));
    }
    for local in &function.locals {
        if emitted.insert(local.name.clone()) {
            out.push_str(&format!(
                "  {} = alloca {}\n",
                llvm_address_name(&local.name),
                llvm_storage_type(&local.type_node)
            ));
        }
    }
    for (name, type_node) in collect_temps(function) {
        if emitted.insert(name.clone()) {
            out.push_str(&format!(
                "  {} = alloca {}\n",
                llvm_address_name(&llvm_storage_name_for_temp(&name)),
                llvm_storage_type(&type_node)
            ));
        }
    }
}

fn emit_llvm_param_stores(
    out: &mut String,
    context: &mut LlvmFunctionContext<'_>,
    function: &MirFunction,
) {
    for param in &function.params {
        if matches!(param.type_node, MirType::Slice(_)) {
            let with_data = llvm_next_register(context);
            out.push_str(&format!(
                "  {with_data} = insertvalue {{ ptr, i32 }} undef, ptr %{}.data, 0\n",
                param.name
            ));
            let with_len = llvm_next_register(context);
            out.push_str(&format!(
                "  {with_len} = insertvalue {{ ptr, i32 }} {with_data}, i32 %{}.len, 1\n",
                param.name
            ));
            out.push_str(&format!(
                "  store {{ ptr, i32 }} {with_len}, ptr {}\n",
                llvm_address_name(&param.name)
            ));
        } else {
            out.push_str(&format!(
                "  store {} %{}, ptr {}\n",
                llvm_param_type(&param.type_node),
                param.name,
                llvm_address_name(&param.name)
            ));
        }
    }
}

fn emit_llvm_instruction(
    out: &mut String,
    context: &mut LlvmFunctionContext<'_>,
    instruction: &MirInstruction,
) {
    match instruction {
        MirInstruction::ConstInt { target, value } => {
            emit_llvm_store(out, target, value);
        }
        MirInstruction::ConstFloat { target, value } => {
            emit_llvm_store(out, target, value);
        }
        MirInstruction::ConstBool { target, value } => {
            emit_llvm_store(out, target, if *value { "1" } else { "0" });
        }
        MirInstruction::Move { target, value } => {
            let loaded = llvm_load_value(out, context, value);
            emit_llvm_store(out, target, &loaded);
        }
        MirInstruction::Binary {
            target,
            op,
            left,
            right,
        } => {
            let left_value = llvm_load_value(out, context, left);
            let right_value = llvm_load_value(out, context, right);
            let result = llvm_next_register(context);
            out.push_str(&format!(
                "  {result} = {} {} {}, {}\n",
                llvm_binary_opcode(*op, value_type(target)),
                llvm_value_type(value_type(target)),
                left_value,
                right_value
            ));
            emit_llvm_store(out, target, &result);
        }
        MirInstruction::Unary {
            target,
            op,
            operand,
        } => {
            let operand_value = llvm_load_value(out, context, operand);
            let result = llvm_next_register(context);
            match op {
                MirUnaryOp::Not => {
                    out.push_str(&format!("  {result} = xor i1 {operand_value}, true\n"))
                }
                MirUnaryOp::Neg if is_f64_type(value_type(target)) => out.push_str(&format!(
                    "  {result} = fneg {} {operand_value}\n",
                    llvm_value_type(value_type(target))
                )),
                MirUnaryOp::Neg => out.push_str(&format!(
                    "  {result} = sub {} 0, {operand_value}\n",
                    llvm_value_type(value_type(target))
                )),
            }
            emit_llvm_store(out, target, &result);
        }
        MirInstruction::Compare {
            target,
            op,
            left,
            right,
        } => {
            let left_value = llvm_load_value(out, context, left);
            let right_value = llvm_load_value(out, context, right);
            let result = llvm_next_register(context);
            out.push_str(&format!(
                "  {result} = {} {} {} {}, {}\n",
                if is_f64_type(value_type(left)) {
                    "fcmp"
                } else {
                    "icmp"
                },
                llvm_compare_predicate(*op, value_type(left)),
                llvm_value_type(value_type(left)),
                left_value,
                right_value
            ));
            emit_llvm_store(out, target, &result);
        }
        MirInstruction::Cast { target, op, value } => {
            let value_text = llvm_load_value(out, context, value);
            let result = llvm_next_register(context);
            let opcode = match op {
                MirCastOp::I32ToF64 => "sitofp",
                MirCastOp::U32ToF64 => "uitofp",
            };
            out.push_str(&format!(
                "  {result} = {opcode} {} {value_text} to {}\n",
                llvm_value_type(value_type(value)),
                llvm_value_type(value_type(target))
            ));
            emit_llvm_store(out, target, &result);
        }
        MirInstruction::Address { target, place } => {
            let pointer = llvm_place_pointer(out, context, place);
            emit_llvm_store(out, target, &pointer);
        }
        MirInstruction::Load { target, place } => {
            let pointer = llvm_place_pointer(out, context, place);
            let result = llvm_next_register(context);
            out.push_str(&format!(
                "  {result} = load {}, ptr {pointer}\n",
                llvm_value_type(value_type(target))
            ));
            emit_llvm_store(out, target, &result);
        }
        MirInstruction::Store { place, value } => {
            let pointer = llvm_place_pointer(out, context, place);
            let value_text = llvm_load_value(out, context, value);
            out.push_str(&format!(
                "  store {} {}, ptr {pointer}\n",
                llvm_value_type(value_type(value)),
                value_text
            ));
        }
        MirInstruction::Call {
            target,
            function_name,
            args,
        } => {
            let mut physical_args = Vec::new();
            for arg in args {
                if matches!(value_type(arg), MirType::Slice(_)) {
                    let aggregate = llvm_load_value(out, context, arg);
                    let data = llvm_next_register(context);
                    out.push_str(&format!(
                        "  {data} = extractvalue {{ ptr, i32 }} {aggregate}, 0\n"
                    ));
                    let len = llvm_next_register(context);
                    out.push_str(&format!(
                        "  {len} = extractvalue {{ ptr, i32 }} {aggregate}, 1\n"
                    ));
                    physical_args.push(format!("ptr {data}"));
                    physical_args.push(format!("i32 {len}"));
                } else {
                    let value = llvm_load_value(out, context, arg);
                    physical_args.push(format!("{} {}", llvm_value_type(value_type(arg)), value));
                }
            }
            let args = physical_args.join(", ");
            if let Some(target) = target {
                let result = llvm_next_register(context);
                out.push_str(&format!(
                    "  {result} = call {} @{}({args})\n",
                    llvm_return_type(value_type(target)),
                    function_name
                ));
                emit_llvm_store(out, target, &result);
            } else {
                out.push_str(&format!("  call void @{function_name}({args})\n"));
            }
        }
        MirInstruction::MakeSlice { target, data, len } => {
            let data = llvm_load_value(out, context, data);
            let len = llvm_load_value(out, context, len);
            let with_data = llvm_next_register(context);
            out.push_str(&format!(
                "  {with_data} = insertvalue {{ ptr, i32 }} undef, ptr {data}, 0\n"
            ));
            let descriptor = llvm_next_register(context);
            out.push_str(&format!(
                "  {descriptor} = insertvalue {{ ptr, i32 }} {with_data}, i32 {len}, 1\n"
            ));
            emit_llvm_store(out, target, &descriptor);
        }
        MirInstruction::SliceData { target, slice } => {
            let descriptor = llvm_load_value(out, context, slice);
            let data = llvm_next_register(context);
            out.push_str(&format!(
                "  {data} = extractvalue {{ ptr, i32 }} {descriptor}, 0\n"
            ));
            emit_llvm_store(out, target, &data);
        }
        MirInstruction::SliceLen { target, slice } => {
            let descriptor = llvm_load_value(out, context, slice);
            let len = llvm_next_register(context);
            out.push_str(&format!(
                "  {len} = extractvalue {{ ptr, i32 }} {descriptor}, 1\n"
            ));
            emit_llvm_store(out, target, &len);
        }
        MirInstruction::Subslice {
            target,
            slice,
            start,
            end,
        } => {
            let MirType::Slice(element_type) = value_type(slice) else {
                unreachable!("subslice source must be a slice")
            };
            let descriptor = llvm_load_value(out, context, slice);
            let data = llvm_next_register(context);
            out.push_str(&format!(
                "  {data} = extractvalue {{ ptr, i32 }} {descriptor}, 0\n"
            ));
            let start = llvm_load_value(out, context, start);
            let end = llvm_load_value(out, context, end);
            let start64 = llvm_index_to_i64(
                out,
                context,
                &MirType::Primitive(MirPrimitiveTypeName::U32),
                &start,
            );
            let advanced = llvm_next_register(context);
            out.push_str(&format!(
                "  {advanced} = getelementptr {}, ptr {data}, i64 {start64}\n",
                llvm_storage_type(element_type)
            ));
            let is_zero = llvm_next_register(context);
            out.push_str(&format!("  {is_zero} = icmp eq i32 {start}, 0\n"));
            let selected = llvm_next_register(context);
            out.push_str(&format!(
                "  {selected} = select i1 {is_zero}, ptr {data}, ptr {advanced}\n"
            ));
            let len = llvm_next_register(context);
            out.push_str(&format!("  {len} = sub i32 {end}, {start}\n"));
            let with_data = llvm_next_register(context);
            out.push_str(&format!(
                "  {with_data} = insertvalue {{ ptr, i32 }} undef, ptr {selected}, 0\n"
            ));
            let result = llvm_next_register(context);
            out.push_str(&format!(
                "  {result} = insertvalue {{ ptr, i32 }} {with_data}, i32 {len}, 1\n"
            ));
            emit_llvm_store(out, target, &result);
        }
    }
}

fn emit_llvm_terminator(
    out: &mut String,
    context: &mut LlvmFunctionContext<'_>,
    function: &MirFunction,
    terminator: &MirTerminator,
) {
    match terminator {
        MirTerminator::Return { value } => {
            if let Some(value) = value {
                let value_text = llvm_load_value(out, context, value);
                out.push_str(&format!(
                    "  ret {} {}\n",
                    llvm_return_type(value_type(value)),
                    value_text
                ));
            } else {
                out.push_str("  ret void\n");
            }
        }
        MirTerminator::Jump { label } => {
            out.push_str(&format!(
                "  br label %{}\n",
                llvm_block_label(function, label)
            ));
        }
        MirTerminator::Branch {
            condition,
            then_label,
            else_label,
        } => {
            let condition = llvm_load_value(out, context, condition);
            out.push_str(&format!(
                "  br i1 {condition}, label %{}, label %{}\n",
                llvm_block_label(function, then_label),
                llvm_block_label(function, else_label)
            ));
        }
    }
}

fn emit_llvm_store(out: &mut String, target: &MirValue, value: &str) {
    out.push_str(&format!(
        "  store {} {}, ptr {}\n",
        llvm_storage_type(value_type(target)),
        value,
        llvm_address_for_value(target)
    ));
}

fn llvm_load_value(
    out: &mut String,
    context: &mut LlvmFunctionContext<'_>,
    value: &MirValue,
) -> String {
    match value {
        MirValue::ConstInt { text, .. } | MirValue::ConstFloat { text, .. } => text.clone(),
        MirValue::ConstBool { value, .. } => {
            if *value {
                "1".to_string()
            } else {
                "0".to_string()
            }
        }
        MirValue::Param { .. } | MirValue::Local { .. } | MirValue::Temp { .. } => {
            let result = llvm_next_register(context);
            out.push_str(&format!(
                "  {result} = load {}, ptr {}\n",
                llvm_value_type(value_type(value)),
                llvm_address_for_value(value)
            ));
            result
        }
    }
}

fn llvm_place_pointer(
    out: &mut String,
    context: &mut LlvmFunctionContext<'_>,
    place: &MirPlace,
) -> String {
    match place {
        MirPlace::Param { name, type_node } | MirPlace::Local { name, type_node } => {
            if matches!(type_node, MirType::Pointer(_)) {
                let result = llvm_next_register(context);
                out.push_str(&format!(
                    "  {result} = load ptr, ptr {}\n",
                    llvm_address_name(name)
                ));
                result
            } else {
                llvm_address_name(name)
            }
        }
        MirPlace::Deref { pointer, .. } => llvm_load_value(out, context, pointer),
        MirPlace::Index { base, index, .. } => {
            let MirType::Pointer(element_type) = place_type(base) else {
                panic!("LLVM index base must be pointer");
            };
            let base_pointer = llvm_place_pointer(out, context, base);
            let index_value = llvm_load_value(out, context, index);
            let index64 = llvm_index_to_i64(out, context, value_type(index), &index_value);
            let result = llvm_next_register(context);
            out.push_str(&format!(
                "  {result} = getelementptr {}, ptr {}, i64 {}\n",
                llvm_storage_type(element_type),
                base_pointer,
                index64
            ));
            result
        }
        MirPlace::SliceIndex { slice, index, .. } => {
            let MirType::Slice(element_type) = value_type(slice) else {
                unreachable!("slice index base must be a slice")
            };
            let descriptor = llvm_load_value(out, context, slice);
            let data = llvm_next_register(context);
            out.push_str(&format!(
                "  {data} = extractvalue {{ ptr, i32 }} {descriptor}, 0\n"
            ));
            let index = llvm_load_value(out, context, index);
            let index64 = llvm_index_to_i64(
                out,
                context,
                &MirType::Primitive(MirPrimitiveTypeName::U32),
                &index,
            );
            let result = llvm_next_register(context);
            out.push_str(&format!(
                "  {result} = getelementptr {}, ptr {data}, i64 {index64}\n",
                llvm_storage_type(element_type)
            ));
            result
        }
        MirPlace::Field {
            base, field_name, ..
        } => {
            let MirType::Struct(struct_name) = place_type(base) else {
                panic!("LLVM field base must be struct");
            };
            let base_pointer = llvm_place_pointer(out, context, base);
            let field_index = context.layout.field_index(struct_name, field_name);
            let result = llvm_next_register(context);
            out.push_str(&format!(
                "  {result} = getelementptr %struct.{}, ptr {}, i32 0, i32 {}\n",
                struct_name, base_pointer, field_index
            ));
            result
        }
    }
}

fn llvm_index_to_i64(
    out: &mut String,
    context: &mut LlvmFunctionContext<'_>,
    index_type: &MirType,
    index_value: &str,
) -> String {
    match index_type {
        MirType::Primitive(MirPrimitiveTypeName::I32) => {
            let result = llvm_next_register(context);
            out.push_str(&format!("  {result} = sext i32 {index_value} to i64\n"));
            result
        }
        MirType::Primitive(MirPrimitiveTypeName::U32) => {
            let result = llvm_next_register(context);
            out.push_str(&format!("  {result} = zext i32 {index_value} to i64\n"));
            result
        }
        MirType::Primitive(MirPrimitiveTypeName::I64 | MirPrimitiveTypeName::U64) => {
            index_value.to_string()
        }
        _ => panic!("LLVM index type must be i32, u32, i64, or u64"),
    }
}

fn llvm_next_register(context: &mut LlvmFunctionContext<'_>) -> String {
    loop {
        let name = format!("v{}", context.register_counter);
        context.register_counter += 1;
        if context.used_value_names.insert(name.clone()) {
            return format!("%{name}");
        }
    }
}

fn llvm_address_for_value(value: &MirValue) -> String {
    match value {
        MirValue::Param { name, .. } | MirValue::Local { name, .. } => llvm_address_name(name),
        MirValue::Temp { name, .. } => llvm_address_name(&llvm_storage_name_for_temp(name)),
        MirValue::ConstInt { .. } | MirValue::ConstFloat { .. } | MirValue::ConstBool { .. } => {
            panic!("LLVM constants do not have storage")
        }
    }
}

fn llvm_address_name(name: &str) -> String {
    format!("%{name}.addr")
}

fn llvm_source_file_name(source_file_name: Option<&str>) -> String {
    source_file_name
        .and_then(|source_file_name| Path::new(source_file_name).file_name())
        .and_then(|file_name| file_name.to_str())
        .unwrap_or("input.ck")
        .to_string()
}

fn llvm_escape_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn llvm_block_label(function: &MirFunction, label: &str) -> String {
    if function
        .blocks
        .first()
        .is_some_and(|block| block.label == label)
    {
        "entry".to_string()
    } else {
        label.to_string()
    }
}

fn llvm_storage_name_for_temp(name: &str) -> String {
    c_temp_name(name)
}

fn llvm_storage_type(type_node: &MirType) -> String {
    match type_node {
        MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32) => {
            "i32".to_string()
        }
        MirType::Primitive(MirPrimitiveTypeName::I64 | MirPrimitiveTypeName::U64) => {
            "i64".to_string()
        }
        MirType::Primitive(MirPrimitiveTypeName::F64) => "double".to_string(),
        MirType::Primitive(MirPrimitiveTypeName::Bool) => "i1".to_string(),
        MirType::Pointer(_) => "ptr".to_string(),
        MirType::Slice(_) => "{ ptr, i32 }".to_string(),
        MirType::Struct(name) => format!("%struct.{name}"),
        MirType::Void => panic!("void has no LLVM storage type"),
    }
}

fn llvm_value_type(type_node: &MirType) -> String {
    llvm_storage_type(type_node)
}

fn llvm_param_type(type_node: &MirType) -> String {
    llvm_storage_type(type_node)
}

fn llvm_return_type(type_node: &MirType) -> String {
    if matches!(type_node, MirType::Void) {
        "void".to_string()
    } else {
        llvm_storage_type(type_node)
    }
}

fn llvm_zero_value(type_node: &MirType) -> &'static str {
    match type_node {
        MirType::Primitive(MirPrimitiveTypeName::F64) => "0.0",
        MirType::Primitive(MirPrimitiveTypeName::Bool) => "0",
        MirType::Primitive(_) | MirType::Pointer(_) => "0",
        MirType::Struct(_) | MirType::Slice(_) => "zeroinitializer",
        MirType::Void => panic!("void has no LLVM zero value"),
    }
}

fn llvm_binary_opcode(op: MirBinaryOp, type_node: &MirType) -> &'static str {
    if is_f64_type(type_node) {
        return match op {
            MirBinaryOp::Add => "fadd",
            MirBinaryOp::Sub => "fsub",
            MirBinaryOp::Mul => "fmul",
            MirBinaryOp::Div => "fdiv",
            MirBinaryOp::Mod => panic!("LLVM backend does not support f64 modulo"),
        };
    }

    match op {
        MirBinaryOp::Add => "add",
        MirBinaryOp::Sub => "sub",
        MirBinaryOp::Mul => "mul",
        MirBinaryOp::Div if is_unsigned_integer_type(type_node) => "udiv",
        MirBinaryOp::Div => "sdiv",
        MirBinaryOp::Mod if is_unsigned_integer_type(type_node) => "urem",
        MirBinaryOp::Mod => "srem",
    }
}

fn llvm_compare_predicate(op: MirCompareOp, type_node: &MirType) -> &'static str {
    if is_f64_type(type_node) {
        return match op {
            MirCompareOp::Eq => "oeq",
            MirCompareOp::Ne => "une",
            MirCompareOp::Lt => "olt",
            MirCompareOp::Le => "ole",
            MirCompareOp::Gt => "ogt",
            MirCompareOp::Ge => "oge",
        };
    }

    let prefix = if is_unsigned_integer_type(type_node) {
        "u"
    } else {
        "s"
    };
    match op {
        MirCompareOp::Eq => "eq",
        MirCompareOp::Ne => "ne",
        MirCompareOp::Lt if prefix == "u" => "ult",
        MirCompareOp::Lt => "slt",
        MirCompareOp::Le if prefix == "u" => "ule",
        MirCompareOp::Le => "sle",
        MirCompareOp::Gt if prefix == "u" => "ugt",
        MirCompareOp::Gt => "sgt",
        MirCompareOp::Ge if prefix == "u" => "uge",
        MirCompareOp::Ge => "sge",
    }
}

fn is_f64_type(type_node: &MirType) -> bool {
    matches!(type_node, MirType::Primitive(MirPrimitiveTypeName::F64))
}

fn is_unsigned_integer_type(type_node: &MirType) -> bool {
    matches!(
        type_node,
        MirType::Primitive(MirPrimitiveTypeName::U32 | MirPrimitiveTypeName::U64)
    )
}

fn is_signed_integer_type(type_node: &MirType) -> bool {
    matches!(
        type_node,
        MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::I64)
    )
}

fn signed_min_constant(type_node: &MirType) -> &'static str {
    match type_node {
        MirType::Primitive(MirPrimitiveTypeName::I32) => "INT32_MIN",
        MirType::Primitive(MirPrimitiveTypeName::I64) => "INT64_MIN",
        _ => panic!("checked C emission requires a signed integer type"),
    }
}

fn value_type(value: &MirValue) -> &MirType {
    match value {
        MirValue::Param { type_node, .. }
        | MirValue::Local { type_node, .. }
        | MirValue::Temp { type_node, .. }
        | MirValue::ConstInt { type_node, .. }
        | MirValue::ConstFloat { type_node, .. }
        | MirValue::ConstBool { type_node, .. } => type_node,
    }
}

fn place_type(place: &MirPlace) -> &MirType {
    match place {
        MirPlace::Param { type_node, .. }
        | MirPlace::Local { type_node, .. }
        | MirPlace::Deref { type_node, .. }
        | MirPlace::Index { type_node, .. }
        | MirPlace::SliceIndex { type_node, .. }
        | MirPlace::Field { type_node, .. } => type_node,
    }
}

#[derive(Debug)]
struct CModulePlan {
    slice_types: Vec<MirType>,
    slice_names: HashMap<MirType, String>,
    functions: HashMap<String, CFunctionPlan>,
    status_abi: bool,
}

#[derive(Debug)]
struct CFunctionPlan {
    slice_params: HashMap<String, (String, String)>,
    temp_names: HashMap<String, String>,
    return_pointer: String,
    status_local: String,
}

#[derive(Debug, Default)]
struct CIdentifierAllocator {
    used: HashSet<String>,
}

impl CIdentifierAllocator {
    fn reserve(&mut self, name: &str) {
        self.used.insert(name.to_string());
    }

    fn allocate(&mut self, preferred: &str) -> String {
        if self.used.insert(preferred.to_string()) {
            return preferred.to_string();
        }
        let mut suffix = 1_u32;
        loop {
            let candidate = format!("{preferred}_{suffix}");
            if self.used.insert(candidate.clone()) {
                return candidate;
            }
            suffix += 1;
        }
    }
}

impl CModulePlan {
    fn new(module: &MirModule, options: EmitCOptions) -> Self {
        let slice_types = collect_module_slice_types(module);
        let mut global_names = CIdentifierAllocator::default();
        for name in [
            "CK_Status",
            "CK_OK",
            "CK_ERR_OVERFLOW",
            "CK_ERR_DIV_BY_ZERO",
            "CK_ERR_NULL_POINTER",
            "CK_ERR_OUT_OF_BOUNDS",
        ] {
            global_names.reserve(name);
        }
        for struct_info in &module.structs {
            global_names.reserve(&struct_info.name);
        }
        for function in &module.functions {
            global_names.reserve(&function.name);
        }

        let mut slice_names = HashMap::new();
        for slice_type in &slice_types {
            let MirType::Slice(element_type) = slice_type else {
                unreachable!("collected slice types must be slices");
            };
            let preferred = format!("CK_Slice_{}", c_generated_type_name(element_type));
            slice_names.insert(slice_type.clone(), global_names.allocate(&preferred));
        }

        let functions = module
            .functions
            .iter()
            .map(|function| {
                let mut names = CIdentifierAllocator::default();
                for param in &function.params {
                    names.reserve(&param.name);
                }
                for local in &function.locals {
                    names.reserve(&local.name);
                }

                let mut slice_params = HashMap::new();
                for param in &function.params {
                    if matches!(param.type_node, MirType::Slice(_)) {
                        let data = names.allocate(&format!("{}_data", param.name));
                        let len = names.allocate(&format!("{}_len", param.name));
                        slice_params.insert(param.name.clone(), (data, len));
                    }
                }

                let mut temp_names = HashMap::new();
                for (name, _) in collect_temps(function) {
                    let generated = names.allocate(&c_temp_name(&name));
                    temp_names.insert(name, generated);
                }

                let return_pointer = names.allocate("ck_return");
                let status_local = names.allocate("ik_status");
                (
                    function.name.clone(),
                    CFunctionPlan {
                        slice_params,
                        temp_names,
                        return_pointer,
                        status_local,
                    },
                )
            })
            .collect();

        Self {
            slice_types,
            slice_names,
            functions,
            status_abi: options.overflow_mode == OverflowMode::Checked
                || options.bounds_mode == BoundsMode::Checked,
        }
    }

    fn function(&self, name: &str) -> &CFunctionPlan {
        self.functions
            .get(name)
            .expect("every MIR function must have a C function plan")
    }

    fn type_name(&self, type_node: &MirType) -> String {
        match type_node {
            MirType::Primitive(MirPrimitiveTypeName::I32) => "int32_t".to_string(),
            MirType::Primitive(MirPrimitiveTypeName::I64) => "int64_t".to_string(),
            MirType::Primitive(MirPrimitiveTypeName::U32) => "uint32_t".to_string(),
            MirType::Primitive(MirPrimitiveTypeName::U64) => "uint64_t".to_string(),
            MirType::Primitive(MirPrimitiveTypeName::F64) => "double".to_string(),
            MirType::Primitive(MirPrimitiveTypeName::Bool) => "bool".to_string(),
            MirType::Pointer(element_type) => format!("{}*", self.type_name(element_type)),
            MirType::Slice(_) => self
                .slice_names
                .get(type_node)
                .cloned()
                .expect("every reachable slice type must have a descriptor name"),
            MirType::Struct(name) => name.clone(),
            MirType::Void => "void".to_string(),
        }
    }
}

fn use_planned_c_emitter(module: &MirModule, options: EmitCOptions) -> bool {
    options.bounds_mode == BoundsMode::Checked || !collect_module_slice_types(module).is_empty()
}

fn c_generated_type_name(type_node: &MirType) -> String {
    match type_node {
        MirType::Primitive(name) => c_primitive_type_identity(*name).to_string(),
        MirType::Pointer(element_type) => {
            format!("ptr_{}", c_generated_type_name(element_type))
        }
        MirType::Slice(element_type) => {
            format!("slice_{}", c_generated_type_name(element_type))
        }
        MirType::Struct(name) => sanitize_c_identifier(name),
        MirType::Void => "void".to_string(),
    }
}

fn sanitize_c_identifier(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn collect_module_slice_types(module: &MirModule) -> Vec<MirType> {
    let mut slices = HashSet::new();
    for struct_info in &module.structs {
        for field in &struct_info.fields {
            collect_slice_types_from_type(&field.type_node, &mut slices);
        }
    }
    for function in &module.functions {
        for param in &function.params {
            collect_slice_types_from_type(&param.type_node, &mut slices);
        }
        collect_slice_types_from_type(&function.return_type, &mut slices);
        for local in &function.locals {
            collect_slice_types_from_type(&local.type_node, &mut slices);
        }
        for block in &function.blocks {
            for instruction in &block.instructions {
                collect_slice_types_from_instruction(instruction, &mut slices);
            }
            match &block.terminator {
                MirTerminator::Return { value } => {
                    if let Some(value) = value {
                        collect_slice_types_from_value(value, &mut slices);
                    }
                }
                MirTerminator::Branch { condition, .. } => {
                    collect_slice_types_from_value(condition, &mut slices);
                }
                MirTerminator::Jump { .. } => {}
            }
        }
    }
    let mut slices = slices.into_iter().collect::<Vec<_>>();
    slices.sort_by_key(c_type_identity);
    slices
}

fn collect_slice_types_from_type(type_node: &MirType, slices: &mut HashSet<MirType>) {
    match type_node {
        MirType::Pointer(element_type) => collect_slice_types_from_type(element_type, slices),
        MirType::Slice(element_type) => {
            slices.insert(type_node.clone());
            collect_slice_types_from_type(element_type, slices);
        }
        MirType::Primitive(_) | MirType::Struct(_) | MirType::Void => {}
    }
}

fn collect_slice_types_from_value(value: &MirValue, slices: &mut HashSet<MirType>) {
    collect_slice_types_from_type(value_type(value), slices);
}

fn collect_slice_types_from_place(place: &MirPlace, slices: &mut HashSet<MirType>) {
    collect_slice_types_from_type(place_type(place), slices);
    match place {
        MirPlace::Deref { pointer, .. } => collect_slice_types_from_value(pointer, slices),
        MirPlace::Index { base, index, .. } => {
            collect_slice_types_from_place(base, slices);
            collect_slice_types_from_value(index, slices);
        }
        MirPlace::SliceIndex { slice, index, .. } => {
            collect_slice_types_from_value(slice, slices);
            collect_slice_types_from_value(index, slices);
        }
        MirPlace::Field { base, .. } => collect_slice_types_from_place(base, slices),
        MirPlace::Param { .. } | MirPlace::Local { .. } => {}
    }
}

fn collect_slice_types_from_instruction(
    instruction: &MirInstruction,
    slices: &mut HashSet<MirType>,
) {
    if let Some(target) = instruction_target(instruction) {
        collect_slice_types_from_value(target, slices);
    }
    match instruction {
        MirInstruction::Move { value, .. }
        | MirInstruction::Unary { operand: value, .. }
        | MirInstruction::Cast { value, .. }
        | MirInstruction::SliceData { slice: value, .. }
        | MirInstruction::SliceLen { slice: value, .. } => {
            collect_slice_types_from_value(value, slices);
        }
        MirInstruction::Binary { left, right, .. }
        | MirInstruction::Compare { left, right, .. } => {
            collect_slice_types_from_value(left, slices);
            collect_slice_types_from_value(right, slices);
        }
        MirInstruction::Address { place, .. } | MirInstruction::Load { place, .. } => {
            collect_slice_types_from_place(place, slices);
        }
        MirInstruction::Store { place, value } => {
            collect_slice_types_from_place(place, slices);
            collect_slice_types_from_value(value, slices);
        }
        MirInstruction::MakeSlice { data, len, .. } => {
            collect_slice_types_from_value(data, slices);
            collect_slice_types_from_value(len, slices);
        }
        MirInstruction::Subslice {
            slice, start, end, ..
        } => {
            collect_slice_types_from_value(slice, slices);
            collect_slice_types_from_value(start, slices);
            collect_slice_types_from_value(end, slices);
        }
        MirInstruction::Call { args, .. } => {
            for arg in args {
                collect_slice_types_from_value(arg, slices);
            }
        }
        MirInstruction::ConstInt { .. }
        | MirInstruction::ConstFloat { .. }
        | MirInstruction::ConstBool { .. } => {}
    }
}

fn emit_planned_c_module(
    module: &MirModule,
    options: EmitCOptions,
    header_file_name: Option<&str>,
) -> String {
    let plan = CModulePlan::new(module, options);
    let mut out = String::new();
    if let Some(header_file_name) = header_file_name {
        out.push_str(&format!(
            "#include \"{}\"\n\n",
            escape_c_include_path(header_file_name)
        ));
        let internal = module
            .functions
            .iter()
            .filter(|function| !function.exported)
            .collect::<Vec<_>>();
        for function in &internal {
            out.push_str(&format!(
                "{};\n",
                planned_c_signature(function, &plan, false)
            ));
        }
        if !internal.is_empty() {
            out.push('\n');
        }
    } else {
        out.push_str("#include <stdbool.h>\n#include <stdint.h>\n");
        if plan.status_abi {
            out.push_str("#include <stddef.h>\n");
        }
        out.push('\n');
        emit_planned_status_declarations(&mut out, plan.status_abi);
        emit_planned_type_declarations(&mut out, module, &plan);
        for function in &module.functions {
            out.push_str(&format!(
                "{};\n",
                planned_c_signature(function, &plan, false)
            ));
        }
        if !module.functions.is_empty() {
            out.push('\n');
        }
    }

    for (index, function) in module.functions.iter().enumerate() {
        emit_planned_c_function(&mut out, function, &plan, options);
        if index + 1 < module.functions.len() {
            out.push('\n');
        }
    }
    out
}

fn emit_planned_c_header(module: &MirModule, options: EmitCOptions) -> String {
    let plan = CModulePlan::new(module, options);
    let mut out = String::new();
    out.push_str("#pragma once\n\n#include <stdint.h>\n#include <stdbool.h>\n");
    if plan.status_abi {
        out.push_str("#include <stddef.h>\n");
    }
    out.push_str(
        "\n#if defined(_WIN32) || defined(__CYGWIN__)\n  #ifdef CK_BUILD_DLL\n    #define CK_API __declspec(dllexport)\n  #else\n    #define CK_API __declspec(dllimport)\n  #endif\n#else\n  #define CK_API __attribute__((visibility(\"default\")))\n#endif\n\n",
    );
    emit_planned_status_declarations(&mut out, plan.status_abi);
    emit_planned_type_declarations(&mut out, module, &plan);
    out.push_str("#ifdef __cplusplus\nextern \"C\" {\n#endif\n");
    for function in module.functions.iter().filter(|function| function.exported) {
        out.push_str(&format!(
            "\nCK_API {};\n",
            planned_c_signature(function, &plan, true)
        ));
    }
    out.push_str("\n#ifdef __cplusplus\n}\n#endif\n");
    out
}

fn emit_planned_status_declarations(out: &mut String, enabled: bool) {
    if !enabled {
        return;
    }
    out.push_str(
        "typedef int32_t CK_Status;\n\n\
         #define CK_OK ((CK_Status)0)\n\
         #define CK_ERR_OVERFLOW ((CK_Status)1)\n\
         #define CK_ERR_DIV_BY_ZERO ((CK_Status)2)\n\
         #define CK_ERR_NULL_POINTER ((CK_Status)3)\n\
         #define CK_ERR_OUT_OF_BOUNDS ((CK_Status)4)\n\n",
    );
}

fn emit_planned_type_declarations(out: &mut String, module: &MirModule, plan: &CModulePlan) {
    for struct_info in &module.structs {
        out.push_str(&format!("typedef struct {0} {0};\n", struct_info.name));
    }
    if !module.structs.is_empty() {
        out.push('\n');
    }

    for slice_type in &plan.slice_types {
        let MirType::Slice(element_type) = slice_type else {
            unreachable!();
        };
        let name = plan.type_name(slice_type);
        out.push_str(&format!(
            "typedef struct {name} {{\n  {}* data;\n  uint32_t len;\n}} {name};\n\n",
            plan.type_name(element_type)
        ));
    }

    for struct_info in dependency_ordered_c_structs(module) {
        out.push_str(&format!("struct {} {{\n", struct_info.name));
        for field in &struct_info.fields {
            out.push_str(&format!(
                "  {} {};\n",
                plan.type_name(&field.type_node),
                field.name
            ));
        }
        out.push_str("};\n\n");
    }
}

fn dependency_ordered_c_structs(module: &MirModule) -> Vec<&crate::MirStruct> {
    let names = module
        .structs
        .iter()
        .map(|struct_info| struct_info.name.as_str())
        .collect::<HashSet<_>>();
    let mut emitted = HashSet::new();
    let mut ordered = Vec::new();
    while ordered.len() < module.structs.len() {
        let before = ordered.len();
        for struct_info in &module.structs {
            if emitted.contains(&struct_info.name) {
                continue;
            }
            let dependencies_ready =
                struct_info
                    .fields
                    .iter()
                    .all(|field| match &field.type_node {
                        MirType::Struct(name) if names.contains(name.as_str()) => {
                            emitted.contains(name)
                        }
                        _ => true,
                    });
            if dependencies_ready {
                emitted.insert(struct_info.name.clone());
                ordered.push(struct_info);
            }
        }
        if ordered.len() == before {
            for struct_info in &module.structs {
                if emitted.insert(struct_info.name.clone()) {
                    ordered.push(struct_info);
                }
            }
        }
    }
    ordered
}

fn planned_c_signature(
    function: &MirFunction,
    plan: &CModulePlan,
    exported_header: bool,
) -> String {
    let function_plan = plan.function(&function.name);
    let prefix = if !exported_header && !function.exported {
        "static "
    } else {
        ""
    };
    let mut params = Vec::new();
    for param in &function.params {
        if let MirType::Slice(element_type) = &param.type_node {
            let (data_name, len_name) = function_plan
                .slice_params
                .get(&param.name)
                .expect("slice parameter must have physical names");
            params.push(format!("{}* {data_name}", plan.type_name(element_type)));
            params.push(format!("uint32_t {len_name}"));
        } else {
            params.push(format!(
                "{} {}",
                plan.type_name(&param.type_node),
                param.name
            ));
        }
    }
    if plan.status_abi && !matches!(function.return_type, MirType::Void) {
        params.push(format!(
            "{}* {}",
            plan.type_name(&function.return_type),
            function_plan.return_pointer
        ));
    }
    let params = if params.is_empty() {
        "void".to_string()
    } else {
        params.join(", ")
    };
    if plan.status_abi {
        format!("{prefix}CK_Status {}({params})", function.name)
    } else {
        format!(
            "{prefix}{} {}({params})",
            plan.type_name(&function.return_type),
            function.name
        )
    }
}

struct PlannedCFunctionContext<'a> {
    plan: &'a CModulePlan,
    function_plan: &'a CFunctionPlan,
    options: EmitCOptions,
}

impl PlannedCFunctionContext<'_> {
    fn value(&self, value: &MirValue) -> String {
        match value {
            MirValue::Param { name, .. } | MirValue::Local { name, .. } => name.clone(),
            MirValue::Temp { name, .. } => self
                .function_plan
                .temp_names
                .get(name)
                .cloned()
                .expect("every MIR temp must have a planned C name"),
            MirValue::ConstInt { text, .. } | MirValue::ConstFloat { text, .. } => text.clone(),
            MirValue::ConstBool { value, .. } => if *value { "true" } else { "false" }.to_string(),
        }
    }

    fn lvalue(&self, value: &MirValue) -> String {
        match value {
            MirValue::Param { name, .. } | MirValue::Local { name, .. } => name.clone(),
            MirValue::Temp { name, .. } => self
                .function_plan
                .temp_names
                .get(name)
                .cloned()
                .expect("every MIR temp must have a planned C name"),
            MirValue::ConstInt { .. }
            | MirValue::ConstFloat { .. }
            | MirValue::ConstBool { .. } => panic!("cannot assign to MIR constant"),
        }
    }

    fn call_args(&self, args: &[MirValue]) -> Vec<String> {
        let mut physical = Vec::new();
        for arg in args {
            let value = self.value(arg);
            if matches!(value_type(arg), MirType::Slice(_)) {
                physical.push(format!("{value}.data"));
                physical.push(format!("{value}.len"));
            } else {
                physical.push(value);
            }
        }
        physical
    }

    fn place(&self, place: &MirPlace) -> CPlaceEmission {
        match place {
            MirPlace::Param { name, .. } | MirPlace::Local { name, .. } => CPlaceEmission {
                preludes: Vec::new(),
                expression: name.clone(),
            },
            MirPlace::Deref { pointer, .. } => CPlaceEmission {
                preludes: Vec::new(),
                expression: format!("(*{})", self.value(pointer)),
            },
            MirPlace::Index { base, index, .. } => {
                let mut emitted = self.place(base);
                emitted.expression = format!("{}[{}]", emitted.expression, self.value(index));
                emitted
            }
            MirPlace::SliceIndex { slice, index, .. } => {
                let slice = self.value(slice);
                let index = self.value(index);
                let preludes = if self.options.bounds_mode == BoundsMode::Checked {
                    vec![
                        format!("if ({index} >= {slice}.len) {{"),
                        "  return CK_ERR_OUT_OF_BOUNDS;".to_string(),
                        "}".to_string(),
                    ]
                } else {
                    Vec::new()
                };
                CPlaceEmission {
                    preludes,
                    expression: format!("{slice}.data[{index}]"),
                }
            }
            MirPlace::Field {
                base, field_name, ..
            } => {
                let mut emitted = self.place(base);
                emitted.expression = if let MirPlace::Deref { pointer, .. } = &**base {
                    format!("{}->{field_name}", self.value(pointer))
                } else {
                    format!("{}.{field_name}", emitted.expression)
                };
                emitted
            }
        }
    }
}

struct CPlaceEmission {
    preludes: Vec<String>,
    expression: String,
}

fn emit_planned_c_function(
    out: &mut String,
    function: &MirFunction,
    plan: &CModulePlan,
    options: EmitCOptions,
) {
    let function_plan = plan.function(&function.name);
    let context = PlannedCFunctionContext {
        plan,
        function_plan,
        options,
    };
    out.push_str(&format!(
        "{} {{\n",
        planned_c_signature(function, plan, false)
    ));
    let referenced_labels = collect_c_referenced_labels(function);
    let safe_unchecked_binary_targets =
        if options.overflow_mode == OverflowMode::Checked && options.opt_level >= 3 {
            collect_safe_checked_induction_binary_targets(function)
        } else {
            HashSet::new()
        };

    for param in &function.params {
        if matches!(param.type_node, MirType::Slice(_)) {
            out.push_str(&format!(
                "  {} {};\n",
                plan.type_name(&param.type_node),
                param.name
            ));
        }
    }
    for local in &function.locals {
        out.push_str(&format!(
            "  {} {};\n",
            plan.type_name(&local.type_node),
            local.name
        ));
    }
    for (name, type_node) in collect_temps(function) {
        out.push_str(&format!(
            "  {} {};\n",
            plan.type_name(&type_node),
            function_plan
                .temp_names
                .get(&name)
                .expect("temp must have planned name")
        ));
    }
    if plan.status_abi && function_has_call(function) {
        out.push_str(&format!("  CK_Status {};\n", function_plan.status_local));
    }
    if !function.params.is_empty()
        || !function.locals.is_empty()
        || !collect_temps(function).is_empty()
        || (plan.status_abi && function_has_call(function))
    {
        out.push('\n');
    }

    if plan.status_abi && !matches!(function.return_type, MirType::Void) {
        out.push_str(&format!(
            "  if ({} == NULL) {{\n    return CK_ERR_NULL_POINTER;\n  }}\n",
            function_plan.return_pointer
        ));
        if !function.blocks.is_empty()
            || function
                .params
                .iter()
                .any(|param| matches!(param.type_node, MirType::Slice(_)))
        {
            out.push('\n');
        }
    }

    let slice_params = function
        .params
        .iter()
        .filter(|param| matches!(param.type_node, MirType::Slice(_)))
        .collect::<Vec<_>>();
    for param in &slice_params {
        let (data_name, len_name) = function_plan
            .slice_params
            .get(&param.name)
            .expect("slice parameter must have physical names");
        out.push_str(&format!("  {}.data = {data_name};\n", param.name));
        out.push_str(&format!("  {}.len = {len_name};\n", param.name));
    }
    if !slice_params.is_empty() && !function.blocks.is_empty() {
        out.push('\n');
    }

    for (index, block) in function.blocks.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        if referenced_labels.contains(&block.label) {
            out.push_str(&format!("{}:\n", block.label));
        }
        for instruction in &block.instructions {
            for line in
                planned_c_instruction_lines(instruction, &context, &safe_unchecked_binary_targets)
            {
                out.push_str("  ");
                out.push_str(&line);
                out.push('\n');
            }
        }
        for line in planned_c_terminator_lines(&block.terminator, &context) {
            out.push_str("  ");
            out.push_str(&line);
            out.push('\n');
        }
    }
    out.push_str("}\n");
}

fn planned_c_instruction_lines(
    instruction: &MirInstruction,
    context: &PlannedCFunctionContext<'_>,
    safe_unchecked_binary_targets: &HashSet<String>,
) -> Vec<String> {
    match instruction {
        MirInstruction::ConstInt { target, value } => {
            vec![format!("{} = {value};", context.lvalue(target))]
        }
        MirInstruction::ConstFloat { target, value } => {
            vec![format!("{} = {value};", context.lvalue(target))]
        }
        MirInstruction::ConstBool { target, value } => vec![format!(
            "{} = {};",
            context.lvalue(target),
            if *value { "true" } else { "false" }
        )],
        MirInstruction::Move { target, value } => vec![format!(
            "{} = {};",
            context.lvalue(target),
            context.value(value)
        )],
        MirInstruction::Binary {
            target,
            op,
            left,
            right,
        } => {
            if context.options.overflow_mode == OverflowMode::Checked
                && !safe_unchecked_binary_targets.contains(&c_value_identity(target))
            {
                planned_checked_c_binary_lines(target, *op, left, right, context)
            } else {
                vec![format!(
                    "{} = {} {} {};",
                    context.lvalue(target),
                    context.value(left),
                    c_binary_op(*op),
                    context.value(right)
                )]
            }
        }
        MirInstruction::Unary {
            target,
            op,
            operand,
        } => {
            if context.options.overflow_mode == OverflowMode::Checked {
                planned_checked_c_unary_lines(target, *op, operand, context)
            } else {
                vec![format!(
                    "{} = {}{};",
                    context.lvalue(target),
                    c_unary_op(*op),
                    context.value(operand)
                )]
            }
        }
        MirInstruction::Compare {
            target,
            op,
            left,
            right,
        } => vec![format!(
            "{} = {} {} {};",
            context.lvalue(target),
            context.value(left),
            c_compare_op(*op),
            context.value(right)
        )],
        MirInstruction::Cast { target, op, value } => {
            let cast = match op {
                MirCastOp::I32ToF64 | MirCastOp::U32ToF64 => "double",
            };
            vec![format!(
                "{} = ({cast}){};",
                context.lvalue(target),
                context.value(value)
            )]
        }
        MirInstruction::Address { target, place } => {
            let emitted = context.place(place);
            let mut lines = emitted.preludes;
            lines.push(format!(
                "{} = &{};",
                context.lvalue(target),
                emitted.expression
            ));
            lines
        }
        MirInstruction::Load { target, place } => {
            let emitted = context.place(place);
            let mut lines = emitted.preludes;
            lines.push(format!(
                "{} = {};",
                context.lvalue(target),
                emitted.expression
            ));
            lines
        }
        MirInstruction::Store { place, value } => {
            let emitted = context.place(place);
            let mut lines = emitted.preludes;
            lines.push(format!(
                "{} = {};",
                emitted.expression,
                context.value(value)
            ));
            lines
        }
        MirInstruction::MakeSlice { target, data, len } => vec![
            format!("{}.data = {};", context.lvalue(target), context.value(data)),
            format!("{}.len = {};", context.lvalue(target), context.value(len)),
        ],
        MirInstruction::SliceData { target, slice } => vec![format!(
            "{} = {}.data;",
            context.lvalue(target),
            context.value(slice)
        )],
        MirInstruction::SliceLen { target, slice } => vec![format!(
            "{} = {}.len;",
            context.lvalue(target),
            context.value(slice)
        )],
        MirInstruction::Subslice {
            target,
            slice,
            start,
            end,
        } => {
            let target = context.lvalue(target);
            let slice = context.value(slice);
            let start = context.value(start);
            let end = context.value(end);
            let mut lines = Vec::new();
            if context.options.bounds_mode == BoundsMode::Checked {
                lines.extend([
                    format!("if ({start} > {end} || {end} > {slice}.len) {{"),
                    "  return CK_ERR_OUT_OF_BOUNDS;".to_string(),
                    "}".to_string(),
                ]);
            }
            lines.push(format!(
                "{target}.data = ({start} == 0 ? {slice}.data : {slice}.data + {start});"
            ));
            lines.push(format!("{target}.len = {end} - {start};"));
            lines
        }
        MirInstruction::Call {
            target,
            function_name,
            args,
        } => {
            let mut args = context.call_args(args);
            if context.plan.status_abi {
                if let Some(target) = target {
                    args.push(format!("&{}", context.lvalue(target)));
                }
                vec![
                    format!(
                        "{} = {function_name}({});",
                        context.function_plan.status_local,
                        args.join(", ")
                    ),
                    format!("if ({} != CK_OK) {{", context.function_plan.status_local),
                    format!("  return {};", context.function_plan.status_local),
                    "}".to_string(),
                ]
            } else {
                let call = format!("{function_name}({})", args.join(", "));
                target.as_ref().map_or_else(
                    || vec![format!("{call};")],
                    |target| vec![format!("{} = {call};", context.lvalue(target))],
                )
            }
        }
    }
}

fn planned_checked_c_unary_lines(
    target: &MirValue,
    op: MirUnaryOp,
    operand: &MirValue,
    context: &PlannedCFunctionContext<'_>,
) -> Vec<String> {
    let target_text = context.lvalue(target);
    let operand_text = context.value(operand);
    match op {
        MirUnaryOp::Not => vec![format!("{target_text} = !{operand_text};")],
        MirUnaryOp::Neg if is_f64_type(value_type(target)) => {
            vec![format!("{target_text} = -{operand_text};")]
        }
        MirUnaryOp::Neg if is_unsigned_integer_type(value_type(target)) => vec![
            format!(
                "if (__builtin_sub_overflow(({})0, {operand_text}, &{target_text})) {{",
                context.plan.type_name(value_type(target))
            ),
            "  return CK_ERR_OVERFLOW;".to_string(),
            "}".to_string(),
        ],
        MirUnaryOp::Neg => vec![
            format!(
                "if ({operand_text} == {}) {{",
                signed_min_constant(value_type(target))
            ),
            "  return CK_ERR_OVERFLOW;".to_string(),
            "}".to_string(),
            format!("{target_text} = -{operand_text};"),
        ],
    }
}

fn planned_checked_c_binary_lines(
    target: &MirValue,
    op: MirBinaryOp,
    left: &MirValue,
    right: &MirValue,
    context: &PlannedCFunctionContext<'_>,
) -> Vec<String> {
    let target_text = context.lvalue(target);
    let left_text = context.value(left);
    let right_text = context.value(right);
    if is_f64_type(value_type(target)) {
        return vec![format!(
            "{target_text} = {left_text} {} {right_text};",
            c_binary_op(op)
        )];
    }
    match op {
        MirBinaryOp::Add => checked_c_overflow_builtin(
            "__builtin_add_overflow",
            &left_text,
            &right_text,
            &target_text,
        ),
        MirBinaryOp::Sub => checked_c_overflow_builtin(
            "__builtin_sub_overflow",
            &left_text,
            &right_text,
            &target_text,
        ),
        MirBinaryOp::Mul => checked_c_overflow_builtin(
            "__builtin_mul_overflow",
            &left_text,
            &right_text,
            &target_text,
        ),
        MirBinaryOp::Div | MirBinaryOp::Mod => {
            let mut lines = vec![
                format!("if ({right_text} == 0) {{"),
                "  return CK_ERR_DIV_BY_ZERO;".to_string(),
                "}".to_string(),
            ];
            if is_signed_integer_type(value_type(target)) {
                lines.extend([
                    format!(
                        "if ({left_text} == {} && {right_text} == -1) {{",
                        signed_min_constant(value_type(target))
                    ),
                    "  return CK_ERR_OVERFLOW;".to_string(),
                    "}".to_string(),
                ]);
            }
            lines.push(format!(
                "{target_text} = {left_text} {} {right_text};",
                c_binary_op(op)
            ));
            lines
        }
    }
}

fn planned_c_terminator_lines(
    terminator: &MirTerminator,
    context: &PlannedCFunctionContext<'_>,
) -> Vec<String> {
    match terminator {
        MirTerminator::Return { value } if context.plan.status_abi => value.as_ref().map_or_else(
            || vec!["return CK_OK;".to_string()],
            |value| {
                vec![
                    format!(
                        "*{} = {};",
                        context.function_plan.return_pointer,
                        context.value(value)
                    ),
                    "return CK_OK;".to_string(),
                ]
            },
        ),
        MirTerminator::Return { value } => value.as_ref().map_or_else(
            || vec!["return;".to_string()],
            |value| vec![format!("return {};", context.value(value))],
        ),
        MirTerminator::Jump { label } => vec![format!("goto {label};")],
        MirTerminator::Branch {
            condition,
            then_label,
            else_label,
        } => vec![
            format!("if ({}) {{", context.value(condition)),
            format!("  goto {then_label};"),
            "} else {".to_string(),
            format!("  goto {else_label};"),
            "}".to_string(),
        ],
    }
}

fn emit_c_function(out: &mut String, function: &MirFunction) {
    out.push_str(&format!("{} {{\n", c_signature(function)));
    let referenced_labels = collect_c_referenced_labels(function);
    for local in &function.locals {
        out.push_str(&format!("  {} {};\n", c_type(&local.type_node), local.name));
    }
    let mut seen_temps = HashSet::new();
    for temp in collect_temps(function) {
        if seen_temps.insert(temp.0.clone()) {
            out.push_str(&format!(
                "  {} {};\n",
                c_type(&temp.1),
                c_temp_name(&temp.0)
            ));
        }
    }
    if (!function.locals.is_empty() || !seen_temps.is_empty()) && !function.blocks.is_empty() {
        out.push('\n');
    }

    for (index, block) in function.blocks.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        if referenced_labels.contains(&block.label) {
            out.push_str(&format!("{}:\n", block.label));
        }
        for instruction in &block.instructions {
            out.push_str("  ");
            out.push_str(&emit_c_instruction(instruction));
            out.push('\n');
        }
        for line in emit_c_terminator(&block.terminator) {
            out.push_str("  ");
            out.push_str(&line);
            out.push('\n');
        }
    }
    out.push_str("}\n");
}

fn emit_checked_c_function(out: &mut String, function: &MirFunction, opt_level: u8) {
    out.push_str(&format!("{} {{\n", checked_c_signature(function)));
    let referenced_labels = collect_c_referenced_labels(function);
    let safe_unchecked_binary_targets = if opt_level >= 3 {
        collect_safe_checked_induction_binary_targets(function)
    } else {
        HashSet::new()
    };
    for local in &function.locals {
        out.push_str(&format!("  {} {};\n", c_type(&local.type_node), local.name));
    }
    let mut seen_temps = HashSet::new();
    for temp in collect_temps(function) {
        if seen_temps.insert(temp.0.clone()) {
            out.push_str(&format!(
                "  {} {};\n",
                c_type(&temp.1),
                c_temp_name(&temp.0)
            ));
        }
    }
    if function_has_call(function) {
        out.push_str("  CK_Status ik_status;\n");
    }
    if !function.locals.is_empty() || !seen_temps.is_empty() || function_has_call(function) {
        out.push('\n');
    }

    if !matches!(function.return_type, MirType::Void) {
        out.push_str("  if (ck_return == NULL) {\n");
        out.push_str("    return CK_ERR_NULL_POINTER;\n");
        out.push_str("  }\n");
        if !function.blocks.is_empty() {
            out.push('\n');
        }
    }

    for (index, block) in function.blocks.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        if referenced_labels.contains(&block.label) {
            out.push_str(&format!("{}:\n", block.label));
        }
        for instruction in &block.instructions {
            for line in emit_checked_c_instruction(instruction, &safe_unchecked_binary_targets) {
                out.push_str("  ");
                out.push_str(&line);
                out.push('\n');
            }
        }
        for line in emit_checked_c_terminator(&block.terminator) {
            out.push_str("  ");
            out.push_str(&line);
            out.push('\n');
        }
    }
    out.push_str("}\n");
}

fn c_signature(function: &MirFunction) -> String {
    let prefix = if function.exported { "" } else { "static " };
    let params = function
        .params
        .iter()
        .map(|param| format!("{} {}", c_type(&param.type_node), param.name))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{prefix}{} {}({})",
        c_type(&function.return_type),
        function.name,
        params
    )
}

fn checked_c_signature(function: &MirFunction) -> String {
    let prefix = if function.exported { "" } else { "static " };
    let mut params = function
        .params
        .iter()
        .map(|param| format!("{} {}", c_type(&param.type_node), param.name))
        .collect::<Vec<_>>();
    if !matches!(function.return_type, MirType::Void) {
        params.push(format!("{}* ck_return", c_type(&function.return_type)));
    }
    format!("{prefix}CK_Status {}({})", function.name, params.join(", "))
}

fn c_export_signature(function: &MirFunction) -> String {
    let params = function
        .params
        .iter()
        .map(|param| format!("{} {}", c_type(&param.type_node), param.name))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{} {}({})",
        c_type(&function.return_type),
        function.name,
        params
    )
}

fn c_export_signature_checked(function: &MirFunction) -> String {
    let mut params = function
        .params
        .iter()
        .map(|param| format!("{} {}", c_type(&param.type_node), param.name))
        .collect::<Vec<_>>();
    if !matches!(function.return_type, MirType::Void) {
        params.push(format!("{}* ck_return", c_type(&function.return_type)));
    }
    format!("CK_Status {}({})", function.name, params.join(", "))
}

fn escape_c_include_path(path: &str) -> String {
    path.replace('\\', "\\\\").replace('"', "\\\"")
}

fn emit_c_instruction(instruction: &MirInstruction) -> String {
    match instruction {
        MirInstruction::ConstInt { target, value } => {
            format!("{} = {};", c_value_lvalue(target), value)
        }
        MirInstruction::ConstFloat { target, value } => {
            format!("{} = {};", c_value_lvalue(target), value)
        }
        MirInstruction::ConstBool { target, value } => {
            format!(
                "{} = {};",
                c_value_lvalue(target),
                if *value { "true" } else { "false" }
            )
        }
        MirInstruction::Move { target, value } => {
            format!("{} = {};", c_value_lvalue(target), c_value(value))
        }
        MirInstruction::Binary {
            target,
            op,
            left,
            right,
        } => format!(
            "{} = {} {} {};",
            c_value_lvalue(target),
            c_value(left),
            c_binary_op(*op),
            c_value(right)
        ),
        MirInstruction::Unary {
            target,
            op,
            operand,
        } => format!(
            "{} = {}{};",
            c_value_lvalue(target),
            c_unary_op(*op),
            c_value(operand)
        ),
        MirInstruction::Compare {
            target,
            op,
            left,
            right,
        } => format!(
            "{} = {} {} {};",
            c_value_lvalue(target),
            c_value(left),
            c_compare_op(*op),
            c_value(right)
        ),
        MirInstruction::Cast { target, op, value } => {
            let cast = match op {
                MirCastOp::I32ToF64 | MirCastOp::U32ToF64 => "double",
            };
            format!("{} = ({cast}){};", c_value_lvalue(target), c_value(value))
        }
        MirInstruction::Address { target, place } => {
            format!("{} = &{};", c_value_lvalue(target), c_place(place))
        }
        MirInstruction::Load { target, place } => {
            format!("{} = {};", c_value_lvalue(target), c_place(place))
        }
        MirInstruction::Store { place, value } => {
            format!("{} = {};", c_place(place), c_value(value))
        }
        MirInstruction::Call {
            target,
            function_name,
            args,
        } => {
            let call = format!(
                "{}({})",
                function_name,
                args.iter().map(c_value).collect::<Vec<_>>().join(", ")
            );
            target.as_ref().map_or_else(
                || format!("{call};"),
                |target| format!("{} = {call};", c_value_lvalue(target)),
            )
        }
        MirInstruction::MakeSlice { .. }
        | MirInstruction::SliceData { .. }
        | MirInstruction::SliceLen { .. }
        | MirInstruction::Subslice { .. } => {
            unreachable!("slice modules must use the planned C emitter")
        }
    }
}

fn emit_checked_c_instruction(
    instruction: &MirInstruction,
    safe_unchecked_binary_targets: &HashSet<String>,
) -> Vec<String> {
    match instruction {
        MirInstruction::ConstInt { target, value } => {
            vec![format!("{} = {};", c_value_lvalue(target), value)]
        }
        MirInstruction::ConstFloat { target, value } => {
            vec![format!("{} = {};", c_value_lvalue(target), value)]
        }
        MirInstruction::ConstBool { target, value } => {
            vec![format!(
                "{} = {};",
                c_value_lvalue(target),
                if *value { "true" } else { "false" }
            )]
        }
        MirInstruction::Move { target, value } => {
            vec![format!("{} = {};", c_value_lvalue(target), c_value(value))]
        }
        MirInstruction::Compare {
            target,
            op,
            left,
            right,
        } => vec![format!(
            "{} = {} {} {};",
            c_value_lvalue(target),
            c_value(left),
            c_compare_op(*op),
            c_value(right)
        )],
        MirInstruction::Cast { target, op, value } => {
            let cast = match op {
                MirCastOp::I32ToF64 | MirCastOp::U32ToF64 => "double",
            };
            vec![format!(
                "{} = ({cast}){};",
                c_value_lvalue(target),
                c_value(value)
            )]
        }
        MirInstruction::Unary {
            target,
            op,
            operand,
        } => checked_c_unary_lines(target, *op, operand),
        MirInstruction::Binary {
            target,
            op,
            left,
            right,
        } => {
            if safe_unchecked_binary_targets.contains(&c_value_identity(target)) {
                return vec![format!(
                    "{} = {} {} {};",
                    c_value_lvalue(target),
                    c_value(left),
                    c_binary_op(*op),
                    c_value(right)
                )];
            }
            checked_c_binary_lines(target, *op, left, right)
        }
        MirInstruction::Address { target, place } => {
            vec![format!("{} = &{};", c_value_lvalue(target), c_place(place))]
        }
        MirInstruction::Load { target, place } => {
            vec![format!("{} = {};", c_value_lvalue(target), c_place(place))]
        }
        MirInstruction::Store { place, value } => {
            vec![format!("{} = {};", c_place(place), c_value(value))]
        }
        MirInstruction::Call {
            target,
            function_name,
            args,
        } => {
            let mut call_args = args.iter().map(c_value).collect::<Vec<_>>();
            if let Some(target) = target {
                call_args.push(format!("&{}", c_value_lvalue(target)));
            }
            vec![
                format!("ik_status = {function_name}({});", call_args.join(", ")),
                "if (ik_status != CK_OK) {".to_string(),
                "  return ik_status;".to_string(),
                "}".to_string(),
            ]
        }
        MirInstruction::MakeSlice { .. }
        | MirInstruction::SliceData { .. }
        | MirInstruction::SliceLen { .. }
        | MirInstruction::Subslice { .. } => {
            unreachable!("slice modules must use the planned C emitter")
        }
    }
}

fn checked_c_unary_lines(target: &MirValue, op: MirUnaryOp, operand: &MirValue) -> Vec<String> {
    let target_text = c_value_lvalue(target);
    let operand_text = c_value(operand);
    match op {
        MirUnaryOp::Not => vec![format!("{target_text} = !{operand_text};")],
        MirUnaryOp::Neg if is_f64_type(value_type(target)) => {
            vec![format!("{target_text} = -{operand_text};")]
        }
        MirUnaryOp::Neg if is_unsigned_integer_type(value_type(target)) => vec![
            format!(
                "if (__builtin_sub_overflow(({})0, {operand_text}, &{target_text})) {{",
                c_type(value_type(target))
            ),
            "  return CK_ERR_OVERFLOW;".to_string(),
            "}".to_string(),
        ],
        MirUnaryOp::Neg => vec![
            format!(
                "if ({operand_text} == {}) {{",
                signed_min_constant(value_type(target))
            ),
            "  return CK_ERR_OVERFLOW;".to_string(),
            "}".to_string(),
            format!("{target_text} = -{operand_text};"),
        ],
    }
}

fn checked_c_binary_lines(
    target: &MirValue,
    op: MirBinaryOp,
    left: &MirValue,
    right: &MirValue,
) -> Vec<String> {
    let target_text = c_value_lvalue(target);
    let left_text = c_value(left);
    let right_text = c_value(right);
    if is_f64_type(value_type(target)) {
        return vec![format!(
            "{target_text} = {left_text} {} {right_text};",
            c_binary_op(op)
        )];
    }

    match op {
        MirBinaryOp::Add => checked_c_overflow_builtin(
            "__builtin_add_overflow",
            &left_text,
            &right_text,
            &target_text,
        ),
        MirBinaryOp::Sub => checked_c_overflow_builtin(
            "__builtin_sub_overflow",
            &left_text,
            &right_text,
            &target_text,
        ),
        MirBinaryOp::Mul => checked_c_overflow_builtin(
            "__builtin_mul_overflow",
            &left_text,
            &right_text,
            &target_text,
        ),
        MirBinaryOp::Div | MirBinaryOp::Mod => {
            let mut lines = vec![
                format!("if ({right_text} == 0) {{"),
                "  return CK_ERR_DIV_BY_ZERO;".to_string(),
                "}".to_string(),
            ];
            if is_signed_integer_type(value_type(target)) {
                lines.push(format!(
                    "if ({left_text} == {} && {right_text} == -1) {{",
                    signed_min_constant(value_type(target))
                ));
                lines.push("  return CK_ERR_OVERFLOW;".to_string());
                lines.push("}".to_string());
            }
            lines.push(format!(
                "{target_text} = {left_text} {} {right_text};",
                c_binary_op(op)
            ));
            lines
        }
    }
}

fn checked_c_overflow_builtin(builtin: &str, left: &str, right: &str, target: &str) -> Vec<String> {
    vec![
        format!("if ({builtin}({left}, {right}, &{target})) {{"),
        "  return CK_ERR_OVERFLOW;".to_string(),
        "}".to_string(),
    ]
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BodyIncrementCandidate {
    binary: MirInstruction,
    move_instruction: MirInstruction,
}

fn collect_safe_checked_induction_binary_targets(function: &MirFunction) -> HashSet<String> {
    let mut safe_targets = HashSet::new();

    for header in &function.blocks {
        let MirTerminator::Branch {
            condition,
            then_label,
            ..
        } = &header.terminator
        else {
            continue;
        };
        let Some(MirInstruction::Compare {
            op: MirCompareOp::Lt,
            left:
                induction @ MirValue::Local {
                    type_node: induction_type,
                    ..
                },
            right: limit,
            ..
        }) = find_c_value_def(header, condition)
        else {
            continue;
        };
        if !is_i32_or_u32_type(induction_type)
            || value_type(limit) != induction_type
            || !is_stable_limit_value(limit)
        {
            continue;
        }

        let Some(body) = function
            .blocks
            .iter()
            .find(|block| block.label == *then_label)
        else {
            continue;
        };
        if !matches!(&body.terminator, MirTerminator::Jump { label } if label == &header.label) {
            continue;
        }

        let Some(candidate) = find_body_increment_candidate(body, induction, limit) else {
            continue;
        };
        let Some(init) = find_zero_initialization_before(function, header, induction) else {
            continue;
        };
        if has_unexpected_assignments(
            function,
            induction,
            &[init, candidate.move_instruction.clone()],
        ) {
            continue;
        }

        if let MirInstruction::Binary { target, .. } = candidate.binary {
            safe_targets.insert(c_value_identity(&target));
        }
    }

    safe_targets
}

fn find_c_value_def<'block>(
    block: &'block MirBlock,
    value: &MirValue,
) -> Option<&'block MirInstruction> {
    if !matches!(value, MirValue::Temp { .. }) {
        return None;
    }
    let identity = c_value_identity(value);
    block.instructions.iter().find(|instruction| {
        instruction_target(instruction).is_some_and(|target| c_value_identity(target) == identity)
    })
}

fn find_body_increment_candidate(
    body: &MirBlock,
    induction: &MirValue,
    limit: &MirValue,
) -> Option<BodyIncrementCandidate> {
    let mut int_constants = std::collections::HashMap::new();
    let mut candidate_binary: Option<MirInstruction> = None;
    let mut candidate_move: Option<MirInstruction> = None;

    for instruction in &body.instructions {
        if let MirInstruction::ConstInt { target, value } = instruction {
            int_constants.insert(c_value_identity(target), value.clone());
            continue;
        }

        if assigns_c_value(instruction, limit) {
            return None;
        }

        if let MirInstruction::Binary {
            target: _,
            op: MirBinaryOp::Add,
            left,
            right,
        } = instruction
            && same_c_value(left, induction)
            && int_constants
                .get(&c_value_identity(right))
                .is_some_and(|value| value == "1")
        {
            if candidate_binary.is_some() {
                return None;
            }
            candidate_binary = Some((*instruction).clone());
            continue;
        }

        if let MirInstruction::Move { target, value } = instruction
            && same_c_value(target, induction)
        {
            let Some(MirInstruction::Binary {
                target: binary_target,
                ..
            }) = &candidate_binary
            else {
                return None;
            };
            if !same_c_value(value, binary_target) || candidate_move.is_some() {
                return None;
            }
            candidate_move = Some((*instruction).clone());
            continue;
        }

        if assigns_c_value(instruction, induction) {
            return None;
        }
    }

    Some(BodyIncrementCandidate {
        binary: candidate_binary?,
        move_instruction: candidate_move?,
    })
}

fn find_zero_initialization_before(
    function: &MirFunction,
    header: &MirBlock,
    induction: &MirValue,
) -> Option<MirInstruction> {
    let mut int_constants = std::collections::HashMap::new();

    for block in &function.blocks {
        if block.label == header.label {
            return None;
        }

        for instruction in &block.instructions {
            if let MirInstruction::ConstInt { target, value } = instruction {
                int_constants.insert(c_value_identity(target), value.clone());
                continue;
            }

            if let MirInstruction::Move { target, value } = instruction
                && same_c_value(target, induction)
            {
                return int_constants
                    .get(&c_value_identity(value))
                    .is_some_and(|value| value == "0")
                    .then(|| instruction.clone());
            }

            if assigns_c_value(instruction, induction) {
                return None;
            }
        }

        if matches!(&block.terminator, MirTerminator::Jump { label } if label == &header.label) {
            return None;
        }
    }

    None
}

fn has_unexpected_assignments(
    function: &MirFunction,
    value: &MirValue,
    allowed: &[MirInstruction],
) -> bool {
    function.blocks.iter().any(|block| {
        block.instructions.iter().any(|instruction| {
            assigns_c_value(instruction, value)
                && !allowed.iter().any(|allowed| allowed == instruction)
        })
    })
}

fn assigns_c_value(instruction: &MirInstruction, value: &MirValue) -> bool {
    instruction_target(instruction).is_some_and(|target| same_c_value(target, value))
}

fn same_c_value(left: &MirValue, right: &MirValue) -> bool {
    c_value_identity(left) == c_value_identity(right)
}

fn c_value_identity(value: &MirValue) -> String {
    match value {
        MirValue::Param { name, .. } => format!("param:{name}"),
        MirValue::Local { name, .. } => format!("local:{name}"),
        MirValue::Temp { name, .. } => format!("temp:{name}"),
        MirValue::ConstInt { text, type_node } => {
            format!("const_int:{text}:{}", c_type_identity(type_node))
        }
        MirValue::ConstFloat { text, type_node } => {
            format!("const_float:{text}:{}", c_type_identity(type_node))
        }
        MirValue::ConstBool { value, .. } => format!("const_bool:{value}"),
    }
}

fn c_type_identity(type_node: &MirType) -> String {
    match type_node {
        MirType::Primitive(name) => c_primitive_type_identity(*name).to_string(),
        MirType::Pointer(element_type) => format!("ptr<{}>", c_type_identity(element_type)),
        MirType::Slice(element_type) => format!("slice<{}>", c_type_identity(element_type)),
        MirType::Struct(name) => format!("struct:{name}"),
        MirType::Void => "void".to_string(),
    }
}

fn c_primitive_type_identity(name: MirPrimitiveTypeName) -> &'static str {
    match name {
        MirPrimitiveTypeName::I32 => "i32",
        MirPrimitiveTypeName::I64 => "i64",
        MirPrimitiveTypeName::U32 => "u32",
        MirPrimitiveTypeName::U64 => "u64",
        MirPrimitiveTypeName::F64 => "f64",
        MirPrimitiveTypeName::Bool => "bool",
    }
}

fn is_i32_or_u32_type(type_node: &MirType) -> bool {
    matches!(
        type_node,
        MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32)
    )
}

fn is_stable_limit_value(value: &MirValue) -> bool {
    matches!(value, MirValue::Param { .. } | MirValue::Local { .. })
}

fn emit_c_terminator(terminator: &MirTerminator) -> Vec<String> {
    match terminator {
        MirTerminator::Return { value } => value.as_ref().map_or_else(
            || vec!["return;".to_string()],
            |value| vec![format!("return {};", c_value(value))],
        ),
        MirTerminator::Jump { label } => vec![format!("goto {label};")],
        MirTerminator::Branch {
            condition,
            then_label,
            else_label,
        } => vec![
            format!("if ({}) {{", c_value(condition)),
            format!("  goto {then_label};"),
            "} else {".to_string(),
            format!("  goto {else_label};"),
            "}".to_string(),
        ],
    }
}

fn emit_checked_c_terminator(terminator: &MirTerminator) -> Vec<String> {
    match terminator {
        MirTerminator::Return { value } => value.as_ref().map_or_else(
            || vec!["return CK_OK;".to_string()],
            |value| {
                vec![
                    format!("*ck_return = {};", c_value(value)),
                    "return CK_OK;".to_string(),
                ]
            },
        ),
        MirTerminator::Jump { label } => vec![format!("goto {label};")],
        MirTerminator::Branch {
            condition,
            then_label,
            else_label,
        } => vec![
            format!("if ({}) {{", c_value(condition)),
            format!("  goto {then_label};"),
            "} else {".to_string(),
            format!("  goto {else_label};"),
            "}".to_string(),
        ],
    }
}

fn collect_c_referenced_labels(function: &MirFunction) -> HashSet<String> {
    let mut labels = HashSet::new();
    for block in &function.blocks {
        match &block.terminator {
            MirTerminator::Jump { label } => {
                labels.insert(label.clone());
            }
            MirTerminator::Branch {
                then_label,
                else_label,
                ..
            } => {
                labels.insert(then_label.clone());
                labels.insert(else_label.clone());
            }
            MirTerminator::Return { .. } => {}
        }
    }
    labels
}

fn function_has_call(function: &MirFunction) -> bool {
    function.blocks.iter().any(|block| {
        block
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, MirInstruction::Call { .. }))
    })
}

fn collect_temps(function: &MirFunction) -> Vec<(String, MirType)> {
    let mut temps = Vec::new();
    let mut seen = HashSet::new();
    for block in &function.blocks {
        for instruction in &block.instructions {
            if let Some(MirValue::Temp { name, type_node }) = instruction_target(instruction)
                && seen.insert(name.clone())
            {
                temps.push((name.clone(), type_node.clone()));
            }
        }
    }
    temps
}

fn instruction_target(instruction: &MirInstruction) -> Option<&MirValue> {
    match instruction {
        MirInstruction::ConstInt { target, .. }
        | MirInstruction::ConstFloat { target, .. }
        | MirInstruction::ConstBool { target, .. }
        | MirInstruction::Move { target, .. }
        | MirInstruction::Binary { target, .. }
        | MirInstruction::Unary { target, .. }
        | MirInstruction::Compare { target, .. }
        | MirInstruction::Cast { target, .. }
        | MirInstruction::Address { target, .. }
        | MirInstruction::Load { target, .. }
        | MirInstruction::MakeSlice { target, .. }
        | MirInstruction::SliceData { target, .. }
        | MirInstruction::SliceLen { target, .. }
        | MirInstruction::Subslice { target, .. } => Some(target),
        MirInstruction::Call { target, .. } => target.as_ref(),
        MirInstruction::Store { .. } => None,
    }
}

fn c_value(value: &MirValue) -> String {
    match value {
        MirValue::Param { name, .. } | MirValue::Local { name, .. } => name.clone(),
        MirValue::Temp { name, .. } => c_temp_name(name),
        MirValue::ConstInt { text, .. } | MirValue::ConstFloat { text, .. } => text.clone(),
        MirValue::ConstBool { value, .. } => {
            if *value {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
    }
}

fn c_value_lvalue(value: &MirValue) -> String {
    match value {
        MirValue::Param { name, .. } | MirValue::Local { name, .. } => name.clone(),
        MirValue::Temp { name, .. } => c_temp_name(name),
        MirValue::ConstInt { .. } | MirValue::ConstFloat { .. } | MirValue::ConstBool { .. } => {
            panic!("cannot assign to MIR constant")
        }
    }
}

fn c_temp_name(name: &str) -> String {
    if let Some(suffix) = name.strip_prefix('t')
        && !suffix.is_empty()
        && suffix.chars().all(|character| character.is_ascii_digit())
    {
        return format!("ik_tmp{suffix}");
    }

    let sanitized = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("ik_tmp_{sanitized}")
}

fn c_place(place: &MirPlace) -> String {
    match place {
        MirPlace::Param { name, .. } | MirPlace::Local { name, .. } => name.clone(),
        MirPlace::Deref { pointer, .. } => format!("(*{})", c_value(pointer)),
        MirPlace::Index { base, index, .. } => format!("{}[{}]", c_place(base), c_value(index)),
        MirPlace::SliceIndex { .. } => {
            unreachable!("slice modules must use the planned C emitter")
        }
        MirPlace::Field {
            base, field_name, ..
        } => match &**base {
            MirPlace::Deref { pointer, .. } => format!("{}->{field_name}", c_value(pointer)),
            _ => format!("{}.{}", c_place(base), field_name),
        },
    }
}

fn c_type(type_node: &MirType) -> String {
    match type_node {
        MirType::Primitive(MirPrimitiveTypeName::I32) => "int32_t".to_string(),
        MirType::Primitive(MirPrimitiveTypeName::I64) => "int64_t".to_string(),
        MirType::Primitive(MirPrimitiveTypeName::U32) => "uint32_t".to_string(),
        MirType::Primitive(MirPrimitiveTypeName::U64) => "uint64_t".to_string(),
        MirType::Primitive(MirPrimitiveTypeName::F64) => "double".to_string(),
        MirType::Primitive(MirPrimitiveTypeName::Bool) => "bool".to_string(),
        MirType::Pointer(element_type) => format!("{}*", c_type(element_type)),
        MirType::Slice(_) => unreachable!("slice modules must use the planned C emitter"),
        MirType::Struct(name) => name.clone(),
        MirType::Void => "void".to_string(),
    }
}

fn c_binary_op(op: MirBinaryOp) -> &'static str {
    match op {
        MirBinaryOp::Add => "+",
        MirBinaryOp::Sub => "-",
        MirBinaryOp::Mul => "*",
        MirBinaryOp::Div => "/",
        MirBinaryOp::Mod => "%",
    }
}

fn c_compare_op(op: MirCompareOp) -> &'static str {
    match op {
        MirCompareOp::Eq => "==",
        MirCompareOp::Ne => "!=",
        MirCompareOp::Lt => "<",
        MirCompareOp::Le => "<=",
        MirCompareOp::Gt => ">",
        MirCompareOp::Ge => ">=",
    }
}

fn c_unary_op(op: MirUnaryOp) -> &'static str {
    match op {
        MirUnaryOp::Neg => "-",
        MirUnaryOp::Not => "!",
    }
}
