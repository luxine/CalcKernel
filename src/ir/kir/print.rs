use std::fmt::{self, Write};

use crate::ir::print::MirTypeDisplay;

use super::*;

#[cfg(test)]
thread_local! {
    static KIR_FORMAT_DISPATCHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

// Observe generic formatting only in unit tests; release expansion is exactly
// the standard write macro. This protects the hot ID writers from regressions.
macro_rules! write {
    ($output:expr, $($arguments:tt)*) => {{
        #[cfg(test)]
        KIR_FORMAT_DISPATCHES.with(|counter| counter.set(counter.get() + 1));
        std::write!($output, $($arguments)*)
    }};
}

/// Stable textual and in-memory KIR contract version for the current compiler.
pub const KIR_FORMAT_VERSION: u32 = 3;

#[must_use]
pub fn print_kir_module(module: &KirModule) -> String {
    let mut output = String::new();
    // All writers use an infallible String sink and standard data formatters.
    // Append directly: these same canonical bytes also identify optimizer states.
    let _ = writeln!(
        output,
        "kir-v{} consumer={} overflow={} bounds={} sanitizer={} profile-schema={} profile-sha256={}",
        KIR_FORMAT_VERSION,
        print_consumer(module.config.consumer),
        print_overflow_mode(module.config.overflow_mode),
        print_bounds_mode(module.config.bounds_mode),
        print_sanitizer_mode(module.config.sanitizer_mode),
        module.profile.schema_version(),
        module.profile.digest_hex(),
    );
    if let Some(entry) = &module.entry {
        let _ = writeln!(
            output,
            "entry {} -> {:?}",
            entry.function_name, entry.result
        );
    }
    for struct_info in &module.structs {
        let _ = write!(output, "struct {} {{", struct_info.name);
        for field in &struct_info.fields {
            let _ = write!(
                output,
                " {}: {};",
                field.name,
                MirTypeDisplay(&field.type_node)
            );
        }
        output.push_str(" }\n");
    }
    if let Some(layout) = &module.tune_layout {
        for function in &layout.functions {
            write_index(&mut output, "tune-layout f", function.function.index(), "");
            for block in &function.blocks {
                write_index(&mut output, " b", block.index(), "");
            }
            output.push('\n');
        }
    }
    for function in &module.functions {
        output.push('\n');
        write_kir_function(&mut output, function);
        output.push('\n');
    }
    output
}

pub(super) fn print_kir_function(function: &KirFunction) -> String {
    let mut output = String::new();
    write_kir_function(&mut output, function);
    output
}

fn write_kir_function(output: &mut String, function: &KirFunction) {
    let exported = if function.exported { "export " } else { "" };
    let tune_noinline = if function.tune_noinline {
        "tune-noinline "
    } else {
        ""
    };
    output.push_str(exported);
    output.push_str(tune_noinline);
    write_index(output, "fn f", function.id.index(), " ");
    output.push_str(&function.name);
    output.push('(');
    for (index, param) in function.params.iter().enumerate() {
        if index != 0 {
            output.push_str(", ");
        }
        write_index(output, "v", param.value.index(), " ");
        let _ = write!(
            output,
            "{}: {}",
            param.name,
            MirTypeDisplay(&param.type_node)
        );
    }
    let _ = write!(output, ") -> {} {{", MirTypeDisplay(&function.return_type));
    for region in &function.regions {
        output.push('\n');
        write_region(output, region);
    }
    for memory in &function.initial_memory {
        write_index(output, "\ninitial_memory r", memory.region.index(), " = m");
        write_index(output, "", memory.version.index(), "");
    }
    for block in &function.blocks {
        output.push('\n');
        write_kir_block(output, block);
    }
    output.push_str("\n}");
}

fn write_kir_block(output: &mut String, block: &KirBlock) {
    output.push_str(&block.label);
    write_index(output, " b", block.id.index(), "(");
    for (index, param) in block.params.iter().enumerate() {
        if index != 0 {
            output.push_str(", ");
        }
        write_index(output, "v", param.value.index(), " ");
        output.push_str(&param.slot);
        output.push_str(": ");
        write_kir_value_type(output, &param.type_node);
    }
    if !block.memory_params.is_empty() {
        if !block.params.is_empty() {
            output.push_str("; ");
        }
        output.push_str("memory ");
        for (index, param) in block.memory_params.iter().enumerate() {
            if index != 0 {
                output.push_str(", ");
            }
            write_index(output, "m", param.version.index(), ": r");
            write_index(output, "", param.region.index(), "");
        }
    }
    output.push_str("):");
    for instruction in &block.instructions {
        output.push_str("\n  ");
        write_kir_instruction(output, instruction);
    }
    output.push_str("\n  ");
    write_kir_terminator(output, &block.terminator);
}

fn write_region(output: &mut String, region: &KirMemoryRegion) {
    write_index(output, "region r", region.id.index(), " ");
    match region.origin {
        KirMemoryRegionOrigin::Conservative => output.push_str("conservative"),
        KirMemoryRegionOrigin::Parameter(value) => {
            write_index(output, "parameter(v", value.index(), ")");
        }
        KirMemoryRegionOrigin::RawSlice(value) => {
            write_index(output, "raw_slice(v", value.index(), ")");
        }
        KirMemoryRegionOrigin::Subslice(value) => {
            write_index(output, "subslice(v", value.index(), ")");
        }
    }
    write_index(output, " partition=r", region.partition.index(), "");
    if let Some(parent) = region.parent {
        write_index(output, " parent=r", parent.index(), "");
    }
    if let Some(interval) = &region.byte_interval {
        let _ = write!(
            output,
            " interval=[v{}*sizeof({}), v{}*sizeof({}))",
            interval.start.index(),
            MirTypeDisplay(&interval.element_type),
            interval.end.index(),
            MirTypeDisplay(&interval.element_type)
        );
    }
}

fn write_kir_instruction(output: &mut String, instruction: &KirInstruction) {
    write_index(output, "i", instruction.id.index(), " ");
    for (index, result) in instruction.results.iter().enumerate() {
        if index != 0 {
            output.push_str(", ");
        }
        write_index(output, "v", result.value.index(), ": ");
        write_kir_value_type(output, &result.type_node);
    }
    if !instruction.results.is_empty() {
        output.push_str(" = ");
    }
    match &instruction.kind {
        KirInstructionKind::Undef { slot } => {
            let _ = write!(output, "undef {slot}");
        }
        KirInstructionKind::ConstInt { value } => {
            let _ = write!(output, "const_int {value}");
        }
        KirInstructionKind::ConstFloat { value } => {
            let _ = write!(output, "const_float {value}");
        }
        KirInstructionKind::ConstBool { value } => {
            let _ = write!(output, "const_bool {value}");
        }
        KirInstructionKind::Copy { value } => {
            write_index(output, "copy v", value.index(), "");
        }
        KirInstructionKind::Binary {
            op,
            left,
            right,
            semantics,
        } => {
            let _ = write!(output, "{op:?}.{} ", print_arithmetic_semantics(*semantics));
            write_index(output, "v", left.index(), ", v");
            write_index(output, "", right.index(), "");
        }
        KirInstructionKind::Unary {
            op,
            operand,
            semantics,
        } => {
            let _ = write!(output, "{op:?}.{} ", print_arithmetic_semantics(*semantics));
            write_index(output, "v", operand.index(), "");
        }
        KirInstructionKind::Compare { op, left, right } => {
            let _ = write!(output, "{op:?} ");
            write_index(output, "v", left.index(), ", v");
            write_index(output, "", right.index(), "");
        }
        KirInstructionKind::Cast { op, value } => {
            let _ = write!(output, "cast {op:?} ");
            write_index(output, "v", value.index(), "");
        }
        KirInstructionKind::CheckCondition { kind, args } => {
            let _ = write!(output, "check_condition {kind:?} ");
            write_values(output, args);
        }
        KirInstructionKind::Guard { condition, failure } => {
            write_index(output, "guard v", condition.index(), " else ");
            let _ = write!(output, "{failure:?}");
        }
        KirInstructionKind::Address { place } => {
            output.push_str("address ");
            write_place(output, place);
        }
        KirInstructionKind::Load { place } => {
            output.push_str("load ");
            write_place(output, place);
        }
        KirInstructionKind::Store { place, value } => {
            output.push_str("store ");
            write_place(output, place);
            write_index(output, ", v", value.index(), "");
        }
        KirInstructionKind::MakeSlice { data, len } => {
            write_index(output, "make_slice v", data.index(), ", v");
            write_index(output, "", len.index(), "");
        }
        KirInstructionKind::SliceData { slice } => {
            write_index(output, "slice_data v", slice.index(), "");
        }
        KirInstructionKind::SliceLen { slice } => {
            write_index(output, "slice_len v", slice.index(), "");
        }
        KirInstructionKind::Subslice { slice, start, end } => {
            write_index(output, "subslice v", slice.index(), ", v");
            write_index(output, "", start.index(), ", v");
            write_index(output, "", end.index(), "");
        }
        KirInstructionKind::Call {
            function_name,
            args,
        } => {
            let _ = write!(output, "call {function_name}(");
            write_values(output, args);
            output.push(')');
        }
        KirInstructionKind::RuntimeCall { intrinsic, args } => {
            let _ = write!(output, "runtime_call {intrinsic:?}(");
            write_values(output, args);
            output.push(')');
        }
        KirInstructionKind::VersionPredicate { predicate } => {
            let _ = write!(output, "version_predicate bits={} ", predicate.address_bits);
            for (index, conjunct) in predicate.conjuncts.iter().enumerate() {
                if index != 0 {
                    output.push_str("&&");
                }
                match conjunct {
                    KirVersionPredicateConjunct::TripThreshold { value, minimum } => {
                        let _ = write!(output, "trip(v{}>={minimum})", value.index());
                    }
                    KirVersionPredicateConjunct::AddressIntervalsDisjoint {
                        left,
                        left_count,
                        left_element_bytes,
                        right,
                        right_count,
                        right_element_bytes,
                    } => {
                        let _ = write!(
                            output,
                            "disjoint(v{}[v{}*{}],v{}[v{}*{}])",
                            left.index(),
                            left_count.index(),
                            left_element_bytes,
                            right.index(),
                            right_count.index(),
                            right_element_bytes
                        );
                    }
                }
            }
        }
        KirInstructionKind::VectorSplat { scalar, region } => {
            let _ = write!(
                output,
                "vector_splat v{} [vr{}]",
                scalar.index(),
                region.index()
            );
        }
        KirInstructionKind::VectorLoad { access, region } => {
            output.push_str("vector_load ");
            write_vector_access(output, access);
            let _ = write!(output, " [vr{}]", region.index());
        }
        KirInstructionKind::VectorStore {
            access,
            value,
            region,
        } => {
            output.push_str("vector_store ");
            write_vector_access(output, access);
            let _ = write!(output, ", v{} [vr{}]", value.index(), region.index());
        }
        KirInstructionKind::VectorBinary {
            op,
            left,
            right,
            semantics,
            no_failure_proof,
            region,
        } => {
            write_lowercase(
                output,
                format_args!(
                    "vector_{op:?}.{} v{}, v{} [proof=",
                    print_arithmetic_semantics(*semantics),
                    left.index(),
                    right.index()
                ),
            );
            write_optional_proof(output, *no_failure_proof);
            let _ = write!(output, " vr{}]", region.index());
        }
        KirInstructionKind::VectorUnary {
            op,
            operand,
            semantics,
            no_failure_proof,
            region,
        } => {
            write_lowercase(
                output,
                format_args!(
                    "vector_{op:?}.{} v{} [proof=",
                    print_arithmetic_semantics(*semantics),
                    operand.index()
                ),
            );
            write_optional_proof(output, *no_failure_proof);
            let _ = write!(output, " vr{}]", region.index());
        }
        KirInstructionKind::VectorCompare {
            op,
            left,
            right,
            region,
        } => {
            write_lowercase(
                output,
                format_args!(
                    "vector_compare_{op:?} v{}, v{} [vr{}]",
                    left.index(),
                    right.index(),
                    region.index()
                ),
            );
        }
        KirInstructionKind::VectorSelect {
            mask,
            when_true,
            when_false,
            region,
        } => {
            let _ = write!(
                output,
                "vector_select v{}, v{}, v{} [vr{}]",
                mask.index(),
                when_true.index(),
                when_false.index(),
                region.index()
            );
        }
        KirInstructionKind::VectorCast { op, value, region } => {
            write_lowercase(
                output,
                format_args!(
                    "vector_cast_{op:?} v{} [vr{}]",
                    value.index(),
                    region.index()
                ),
            );
        }
        KirInstructionKind::VectorInsert {
            vector,
            scalar,
            lane_index,
            region,
        } => {
            let _ = write!(
                output,
                "vector_insert v{}, v{}, lane={} [vr{}]",
                vector.index(),
                scalar.index(),
                lane_index,
                region.index()
            );
        }
        KirInstructionKind::VectorExtract {
            vector,
            lane_index,
            region,
        } => {
            let _ = write!(
                output,
                "vector_extract v{}, lane={} [vr{}]",
                vector.index(),
                lane_index,
                region.index()
            );
        }
        KirInstructionKind::VectorReduce {
            op,
            vector,
            semantics,
            region,
        } => {
            write_lowercase(
                output,
                format_args!(
                    "vector_reduce_{op:?}.{} v{} [vr{}]",
                    print_arithmetic_semantics(*semantics),
                    vector.index(),
                    region.index()
                ),
            );
        }
    }
    if let Some(memory) = &instruction.memory {
        write_index(output, " [memory r", memory.region.index(), " m");
        write_index(output, "", memory.input.index(), "");
        if let Some(version) = memory.output {
            write_index(output, " -> m", version.index(), "");
        }
        output.push(']');
    }
    if let Some(effect) = &instruction.effect {
        let _ = write!(output, " [effect {} {:?}]", effect.order, effect.kind);
    }
}

fn write_lowercase(output: &mut String, arguments: fmt::Arguments<'_>) {
    let start = output.len();
    let _ = output.write_fmt(arguments);
    // Only the newly formatted fragment changes case, never identifiers that
    // already precede it. String's prior length is always a UTF-8 boundary.
    output[start..].make_ascii_lowercase();
}

fn write_optional_proof(output: &mut String, proof: Option<ProofId>) {
    if let Some(proof) = proof {
        write_index(output, "p", proof.index(), "");
    } else {
        output.push_str("none");
    }
}

fn write_kir_value_type(output: &mut String, type_node: &KirValueType) {
    match type_node {
        KirValueType::Scalar(type_node) => {
            let _ = write!(output, "{}", MirTypeDisplay(type_node));
        }
        KirValueType::FixedVector { lane, lanes } => {
            write_lowercase(output, format_args!("vector<{lane:?}, {lanes}>"));
        }
        KirValueType::Mask { lanes } => {
            let _ = write!(output, "mask<{lanes}>");
        }
    }
}

fn print_arithmetic_semantics(semantics: KirArithmeticSemantics) -> &'static str {
    match semantics {
        KirArithmeticSemantics::Modular => "modular",
        KirArithmeticSemantics::Checked => "checked",
        KirArithmeticSemantics::StrictFloat => "strict",
    }
}

fn write_vector_access(output: &mut String, access: &KirVectorMemoryAccess) {
    write_lowercase(
        output,
        format_args!(
            "slice=v{} start=v{} end=v{} lane={:?} lanes={} bytes={} align={}/{}",
            access.slice.index(),
            access.start.index(),
            access.end.index(),
            access.lane,
            access.lanes,
            access.byte_footprint,
            access.known_alignment,
            access.required_alignment
        ),
    );
}

fn write_values(output: &mut String, values: &[ValueId]) {
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            output.push_str(", ");
        }
        write_index(output, "v", value.index(), "");
    }
}

fn write_place(output: &mut String, place: &KirPlace) {
    match place {
        KirPlace::Value { value, .. } => {
            write_index(output, "value(v", value.index(), ")");
        }
        KirPlace::Deref { pointer, .. } => {
            write_index(output, "deref(v", pointer.index(), ")");
        }
        KirPlace::Index { base, index, .. } => {
            output.push_str("index(");
            write_place(output, base);
            write_index(output, ", v", index.index(), ")");
        }
        KirPlace::SliceIndex { slice, index, .. } => {
            write_index(output, "slice_index(v", slice.index(), ", v");
            write_index(output, "", index.index(), ")");
        }
        KirPlace::Field {
            base, field_name, ..
        } => {
            output.push_str("field(");
            write_place(output, base);
            let _ = write!(output, ", {field_name})");
        }
    }
}

fn write_kir_terminator(output: &mut String, terminator: &KirTerminator) {
    match terminator {
        KirTerminator::Return {
            value,
            memory,
            effect_order,
        } => {
            output.push_str("return");
            if let Some(value) = value {
                write_index(output, " v", value.index(), "");
            }
            let _ = write!(output, " [effect {effect_order}]");
            write_return_memory(output, memory);
        }
        KirTerminator::Jump { edge } => {
            output.push_str("jump ");
            write_edge(output, edge);
        }
        KirTerminator::Branch {
            condition,
            then_edge,
            else_edge,
        } => {
            write_index(output, "branch v", condition.index(), ", ");
            write_edge(output, then_edge);
            output.push_str(", ");
            write_edge(output, else_edge);
        }
    }
}

fn write_edge(output: &mut String, edge: &KirEdge) {
    write_index(output, "b", edge.target.index(), "(");
    write_values(output, &edge.args);
    if !edge.memory_args.is_empty() {
        output.push_str("; memory ");
        for (index, version) in edge.memory_args.iter().enumerate() {
            if index != 0 {
                output.push_str(", ");
            }
            write_index(output, "m", version.index(), "");
        }
    }
    output.push(')');
}

fn write_return_memory(output: &mut String, memory: &[(MemoryRegionId, MemoryVersionId)]) {
    if !memory.is_empty() {
        output.push_str(" [memory ");
        for (index, (region, version)) in memory.iter().enumerate() {
            if index != 0 {
                output.push_str(", ");
            }
            write_index(output, "r", region.index(), "=m");
            write_index(output, "", version.index(), "");
        }
        output.push(']');
    }
}

fn write_index(output: &mut String, prefix: &str, mut index: u32, suffix: &str) {
    output.push_str(prefix);
    if index < 10 {
        output.push(char::from(b'0' + index as u8));
    } else {
        // An index has at most ten decimal digits, independent of its value.
        // Keep canonical decimal output without general formatting dispatch.
        let mut digits = [0_u8; 10];
        let mut first = digits.len();
        loop {
            first -= 1;
            digits[first] = b'0' + (index % 10) as u8;
            index /= 10;
            if index == 0 {
                break;
            }
        }
        output.push_str(std::str::from_utf8(&digits[first..]).expect("decimal digits are UTF-8"));
    }
    output.push_str(suffix);
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

#[cfg(test)]
#[path = "../../../tests/ir/kir_decimal.rs"]
mod decimal_tests;
