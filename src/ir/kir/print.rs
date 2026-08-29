use crate::print_mir_type;

use super::*;

#[must_use]
pub fn print_kir_module(module: &KirModule) -> String {
    let mut output = format!(
        "kir consumer={} overflow={} bounds={} sanitizer={}\n",
        print_consumer(module.config.consumer),
        print_overflow_mode(module.config.overflow_mode),
        print_bounds_mode(module.config.bounds_mode),
        print_sanitizer_mode(module.config.sanitizer_mode),
    );
    if let Some(entry) = &module.entry {
        output.push_str(&format!(
            "entry {} -> {:?}\n",
            entry.function_name, entry.result
        ));
    }
    for struct_info in &module.structs {
        output.push_str(&format!("struct {} {{", struct_info.name));
        for field in &struct_info.fields {
            output.push_str(&format!(
                " {}: {};",
                field.name,
                print_mir_type(&field.type_node)
            ));
        }
        output.push_str(" }\n");
    }
    for function in &module.functions {
        output.push('\n');
        output.push_str(&print_kir_function(function));
        output.push('\n');
    }
    output
}

fn print_kir_function(function: &KirFunction) -> String {
    let exported = if function.exported { "export " } else { "" };
    let params = function
        .params
        .iter()
        .map(|param| {
            format!(
                "v{} {}: {}",
                param.value.index(),
                param.name,
                print_mir_type(&param.type_node)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let mut lines = vec![format!(
        "{exported}fn f{} {}({params}) -> {} {{",
        function.id.index(),
        function.name,
        print_mir_type(&function.return_type)
    )];
    for region in &function.regions {
        lines.push(print_region(region));
    }
    for memory in &function.initial_memory {
        lines.push(format!(
            "initial_memory r{} = m{}",
            memory.region.index(),
            memory.version.index()
        ));
    }
    for block in &function.blocks {
        lines.push(print_kir_block(block));
    }
    lines.push("}".to_string());
    lines.join("\n")
}

fn print_kir_block(block: &KirBlock) -> String {
    let params = block
        .params
        .iter()
        .map(|param| {
            format!(
                "v{} {}: {}",
                param.value.index(),
                param.slot,
                print_mir_type(&param.type_node)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let memory_params = block
        .memory_params
        .iter()
        .map(|param| format!("m{}: r{}", param.version.index(), param.region.index()))
        .collect::<Vec<_>>()
        .join(", ");
    let signature = if memory_params.is_empty() {
        params
    } else if params.is_empty() {
        format!("memory {memory_params}")
    } else {
        format!("{params}; memory {memory_params}")
    };
    let mut lines = vec![format!(
        "{} b{}({signature}):",
        block.label,
        block.id.index()
    )];
    for instruction in &block.instructions {
        lines.push(format!("  {}", print_kir_instruction(instruction)));
    }
    lines.push(format!("  {}", print_kir_terminator(&block.terminator)));
    lines.join("\n")
}

fn print_region(region: &KirMemoryRegion) -> String {
    let origin = match region.origin {
        KirMemoryRegionOrigin::Conservative => "conservative".to_string(),
        KirMemoryRegionOrigin::Parameter(value) => format!("parameter(v{})", value.index()),
        KirMemoryRegionOrigin::RawSlice(value) => format!("raw_slice(v{})", value.index()),
        KirMemoryRegionOrigin::Subslice(value) => format!("subslice(v{})", value.index()),
    };
    let parent = region
        .parent
        .map(|parent| format!(" parent=r{}", parent.index()))
        .unwrap_or_default();
    let interval = region
        .byte_interval
        .as_ref()
        .map(|interval| {
            format!(
                " interval=[v{}, v{})",
                interval.start.index(),
                interval.end.index()
            )
        })
        .unwrap_or_default();
    format!(
        "region r{} {origin} partition=r{}{}{interval}",
        region.id.index(),
        region.partition.index(),
        parent
    )
}

fn print_kir_instruction(instruction: &KirInstruction) -> String {
    let results = instruction
        .results
        .iter()
        .map(|result| {
            format!(
                "v{}: {}",
                result.value.index(),
                print_mir_type(&result.type_node)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let operation = match &instruction.kind {
        KirInstructionKind::Undef { slot } => format!("undef {slot}"),
        KirInstructionKind::ConstInt { value } => format!("const_int {value}"),
        KirInstructionKind::ConstFloat { value } => format!("const_float {value}"),
        KirInstructionKind::ConstBool { value } => format!("const_bool {value}"),
        KirInstructionKind::Copy { value } => format!("copy v{}", value.index()),
        KirInstructionKind::Binary {
            op,
            left,
            right,
            semantics,
        } => format!(
            "{:?}.{} v{}, v{}",
            op,
            match semantics {
                KirArithmeticSemantics::Modular => "modular",
                KirArithmeticSemantics::Checked => "checked",
                KirArithmeticSemantics::StrictFloat => "strict",
            },
            left.index(),
            right.index()
        ),
        KirInstructionKind::Unary {
            op,
            operand,
            semantics,
        } => format!(
            "{:?}.{} v{}",
            op,
            match semantics {
                KirArithmeticSemantics::Modular => "modular",
                KirArithmeticSemantics::Checked => "checked",
                KirArithmeticSemantics::StrictFloat => "strict",
            },
            operand.index(),
        ),
        KirInstructionKind::Compare { op, left, right } => {
            format!("{op:?} v{}, v{}", left.index(), right.index())
        }
        KirInstructionKind::Cast { op, value } => {
            format!("cast {op:?} v{}", value.index())
        }
        KirInstructionKind::CheckCondition { kind, args } => {
            format!("check_condition {kind:?} {}", print_values(args))
        }
        KirInstructionKind::Guard { condition, failure } => {
            format!("guard v{} else {failure:?}", condition.index())
        }
        KirInstructionKind::Address { place } => format!("address {}", print_place(place)),
        KirInstructionKind::Load { place } => format!("load {}", print_place(place)),
        KirInstructionKind::Store { place, value } => {
            format!("store {}, v{}", print_place(place), value.index())
        }
        KirInstructionKind::MakeSlice { data, len } => {
            format!("make_slice v{}, v{}", data.index(), len.index())
        }
        KirInstructionKind::SliceData { slice } => format!("slice_data v{}", slice.index()),
        KirInstructionKind::SliceLen { slice } => format!("slice_len v{}", slice.index()),
        KirInstructionKind::Subslice { slice, start, end } => format!(
            "subslice v{}, v{}, v{}",
            slice.index(),
            start.index(),
            end.index()
        ),
        KirInstructionKind::Call {
            function_name,
            args,
        } => format!("call {function_name}({})", print_values(args)),
        KirInstructionKind::RuntimeCall { intrinsic, args } => {
            format!("runtime_call {intrinsic:?}({})", print_values(args))
        }
    };
    let mut suffix = String::new();
    if let Some(memory) = &instruction.memory {
        suffix.push_str(&format!(
            " [memory r{} m{}{}]",
            memory.region.index(),
            memory.input.index(),
            memory
                .output
                .map(|version| format!(" -> m{}", version.index()))
                .unwrap_or_default()
        ));
    }
    if let Some(effect) = &instruction.effect {
        suffix.push_str(&format!(" [effect {} {:?}]", effect.order, effect.kind));
    }
    if results.is_empty() {
        format!("i{} {operation}{suffix}", instruction.id.index())
    } else {
        format!(
            "i{} {results} = {operation}{suffix}",
            instruction.id.index()
        )
    }
}

fn print_values(values: &[ValueId]) -> String {
    values
        .iter()
        .map(|value| format!("v{}", value.index()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn print_place(place: &KirPlace) -> String {
    match place {
        KirPlace::Value { value, .. } => format!("value(v{})", value.index()),
        KirPlace::Deref { pointer, .. } => format!("deref(v{})", pointer.index()),
        KirPlace::Index { base, index, .. } => {
            format!("index({}, v{})", print_place(base), index.index())
        }
        KirPlace::SliceIndex { slice, index, .. } => {
            format!("slice_index(v{}, v{})", slice.index(), index.index())
        }
        KirPlace::Field {
            base, field_name, ..
        } => format!("field({}, {field_name})", print_place(base)),
    }
}

fn print_kir_terminator(terminator: &KirTerminator) -> String {
    match terminator {
        KirTerminator::Return {
            value,
            memory,
            effect_order,
        } => {
            let memory = print_return_memory(memory);
            value.map_or_else(
                || format!("return [effect {effect_order}]"),
                |value| format!("return v{} [effect {effect_order}]", value.index()),
            ) + &memory
        }
        KirTerminator::Jump { edge } => format!("jump {}", print_edge(edge)),
        KirTerminator::Branch {
            condition,
            then_edge,
            else_edge,
        } => format!(
            "branch v{}, {}, {}",
            condition.index(),
            print_edge(then_edge),
            print_edge(else_edge)
        ),
    }
}

fn print_edge(edge: &KirEdge) -> String {
    let memory = if edge.memory_args.is_empty() {
        String::new()
    } else {
        format!(
            "; memory {}",
            edge.memory_args
                .iter()
                .map(|version| format!("m{}", version.index()))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    format!(
        "b{}({}{memory})",
        edge.target.index(),
        edge.args
            .iter()
            .map(|value| format!("v{}", value.index()))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn print_return_memory(memory: &[(MemoryRegionId, MemoryVersionId)]) -> String {
    if memory.is_empty() {
        String::new()
    } else {
        format!(
            " [memory {}]",
            memory
                .iter()
                .map(|(region, version)| format!("r{}=m{}", region.index(), version.index()))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

const fn print_consumer(consumer: KirConsumer) -> &'static str {
    match consumer {
        KirConsumer::C => "c",
        KirConsumer::WebAssembly => "wasm",
        KirConsumer::NativeLibrary => "native-library",
        KirConsumer::NativeExecutable => "native-executable",
        KirConsumer::Inspection => "inspection",
    }
}

const fn print_overflow_mode(mode: KirOverflowMode) -> &'static str {
    match mode {
        KirOverflowMode::Unchecked => "unchecked",
        KirOverflowMode::Checked => "checked",
    }
}

const fn print_bounds_mode(mode: KirBoundsMode) -> &'static str {
    match mode {
        KirBoundsMode::Unchecked => "unchecked",
        KirBoundsMode::Checked => "checked",
    }
}

const fn print_sanitizer_mode(mode: KirSanitizerMode) -> &'static str {
    match mode {
        KirSanitizerMode::Disabled => "disabled",
        KirSanitizerMode::Contracts => "contracts",
    }
}
