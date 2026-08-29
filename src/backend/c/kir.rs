use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::*;

use super::{
    layout::{c_generated_type_name, dependency_ordered_c_structs},
    names::CIdentifierAllocator,
};

struct KirCModuleLayout {
    slice_names: HashMap<MirType, String>,
    functions: BTreeMap<FunctionId, KirCFunctionLayout>,
}

struct KirCFunctionLayout {
    params: BTreeMap<ValueId, KirCParamLayout>,
    values: BTreeMap<ValueId, String>,
    return_pointer: String,
    call_status: BTreeMap<InstructionId, String>,
}

enum KirCParamLayout {
    Scalar(String),
    Slice { data: String, len: String },
}

impl KirCModuleLayout {
    fn new(module: &KirModule) -> Self {
        let slices = collect_slice_types(module);
        let mut globals = CIdentifierAllocator::default();
        for name in [
            "CK_Status",
            "CK_OK",
            "CK_ERR_OVERFLOW",
            "CK_ERR_DIV_BY_ZERO",
            "CK_ERR_NULL_POINTER",
            "CK_ERR_OUT_OF_BOUNDS",
            "CKC_UNUSED",
            "CKC_ASSUME_ALIGNED",
            "CKC_RESTRICT",
        ] {
            globals.reserve(name);
        }
        for structure in &module.structs {
            globals.reserve(&structure.name);
        }
        for function in &module.functions {
            globals.reserve(&function.name);
        }
        let slice_names = slices
            .into_iter()
            .map(|slice| {
                let MirType::Slice(element) = &slice else {
                    unreachable!("collected C KIR slice type")
                };
                let name =
                    globals.allocate(&format!("CK_Slice_{}", c_generated_type_name(element)));
                (slice, name)
            })
            .collect();
        let functions = module
            .functions
            .iter()
            .map(|function| (function.id, KirCFunctionLayout::new(function)))
            .collect();
        Self {
            slice_names,
            functions,
        }
    }

    fn function(&self, function: &KirFunction) -> &KirCFunctionLayout {
        self.functions
            .get(&function.id)
            .expect("every KIR function has C names")
    }

    fn type_name(&self, type_node: &MirType) -> String {
        match type_node {
            MirType::Primitive(MirPrimitiveTypeName::I32) => "int32_t".to_string(),
            MirType::Primitive(MirPrimitiveTypeName::I64) => "int64_t".to_string(),
            MirType::Primitive(MirPrimitiveTypeName::U32) => "uint32_t".to_string(),
            MirType::Primitive(MirPrimitiveTypeName::U64) => "uint64_t".to_string(),
            MirType::Primitive(MirPrimitiveTypeName::F64) => "double".to_string(),
            MirType::Primitive(MirPrimitiveTypeName::Bool) => "bool".to_string(),
            MirType::Pointer(element) => format!("{}*", self.type_name(element)),
            MirType::Slice(_) => self
                .slice_names
                .get(type_node)
                .cloned()
                .expect("every KIR slice type has a C name"),
            MirType::Struct(name) => name.clone(),
            MirType::Void => "void".to_string(),
        }
    }
}

impl KirCFunctionLayout {
    fn new(function: &KirFunction) -> Self {
        let mut allocator = CIdentifierAllocator::default();
        for param in &function.params {
            allocator.reserve(&param.name);
        }
        let params = function
            .params
            .iter()
            .map(|param| {
                let layout = if matches!(param.type_node, MirType::Slice(_)) {
                    KirCParamLayout::Slice {
                        data: allocator.allocate(&format!("{}_data", param.name)),
                        len: allocator.allocate(&format!("{}_len", param.name)),
                    }
                } else {
                    KirCParamLayout::Scalar(param.name.clone())
                };
                (param.value, layout)
            })
            .collect();
        let values = value_types(function)
            .into_keys()
            .map(|value| (value, allocator.allocate(&format!("ck_v{}", value.index()))))
            .collect();
        let return_pointer = allocator.allocate("ck_return");
        let call_status = function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Call { .. }))
            .map(|instruction| {
                (
                    instruction.id,
                    allocator.allocate(&format!("ck_status_{}", instruction.id.index())),
                )
            })
            .collect();
        Self {
            params,
            values,
            return_pointer,
            call_status,
        }
    }

    fn value(&self, value: ValueId) -> &str {
        &self.values[&value]
    }
}

#[must_use]
pub fn emit_c_kir_header(module: &KirModule) -> String {
    emit_c_kir_header_with_mode(module, true)
}

pub(in crate::backend) fn emit_c_kir_header_with_mode(module: &KirModule, dynamic: bool) -> String {
    let layout = KirCModuleLayout::new(module);
    let mut out = String::from(
        "#pragma once\n\n#include <stdbool.h>\n#include <stddef.h>\n#include <stdint.h>\n\n",
    );
    if dynamic {
        out.push_str("#if defined(_WIN32) || defined(__CYGWIN__)\n#ifdef CK_BUILD_DLL\n#define CK_API __declspec(dllexport)\n#else\n#define CK_API __declspec(dllimport)\n#endif\n#else\n#define CK_API __attribute__((visibility(\"default\")))\n#endif\n\n");
    } else {
        out.push_str("#if defined(_WIN32) || defined(__CYGWIN__)\n#define CK_API\n#else\n#define CK_API __attribute__((visibility(\"default\")))\n#endif\n\n");
    }
    emit_status_declarations(&mut out, status_abi(module));
    emit_type_declarations(&mut out, module, &layout);
    out.push_str("#ifdef __cplusplus\nextern \"C\" {\n#endif\n");
    for function in module.functions.iter().filter(|function| function.exported) {
        out.push_str(&format!(
            "\nCK_API {};\n",
            signature(function, status_abi(module), &BTreeSet::new(), &layout)
        ));
    }
    out.push_str("\n#ifdef __cplusplus\n}\n#endif\n");
    out
}

pub fn emit_c_kir_module(module: &KirModule) -> Result<String, String> {
    emit_c_kir_module_with_contracts(module, None)
}

pub fn emit_c_kir_module_with_contracts(
    module: &KirModule,
    contracts: Option<&ContractFactSet>,
) -> Result<String, String> {
    if let Some(runtime) = module
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.instructions)
        .find(|instruction| matches!(instruction.kind, KirInstructionKind::RuntimeCall { .. }))
    {
        return Err(format!(
            "C KIR artifact cannot lower runtime instruction i{}",
            runtime.id.index()
        ));
    }
    let status = status_abi(module);
    let layout = KirCModuleLayout::new(module);
    let mut out = String::from(
        "#include <stdbool.h>\n#include <stddef.h>\n#include <stdint.h>\n\n\
         #if defined(__GNUC__) || defined(__clang__)\n\
         #define CKC_UNUSED __attribute__((unused))\n\
         #define CKC_ASSUME_ALIGNED(pointer, alignment) __builtin_assume_aligned((pointer), (alignment))\n\
         #define CKC_RESTRICT __restrict__\n\
         #elif defined(_MSC_VER)\n\
         #define CKC_UNUSED\n\
         #define CKC_ASSUME_ALIGNED(pointer, alignment) (pointer)\n\
         #define CKC_RESTRICT __restrict\n\
         #else\n\
         #define CKC_UNUSED\n\
         #define CKC_ASSUME_ALIGNED(pointer, alignment) (pointer)\n\
         #define CKC_RESTRICT restrict\n\
         #endif\n\n",
    );
    emit_status_declarations(&mut out, status);
    emit_type_declarations(&mut out, module, &layout);
    for function in &module.functions {
        let fact_hints = function_fact_hints(function, contracts);
        out.push_str(&format!(
            "{};\n",
            signature(function, status, &fact_hints.restrict_params, &layout)
        ));
    }
    if !module.functions.is_empty() {
        out.push('\n');
    }
    for (index, function) in module.functions.iter().enumerate() {
        emit_function(&mut out, module, function, contracts, status, &layout)?;
        if index + 1 < module.functions.len() {
            out.push('\n');
        }
    }
    Ok(out)
}

fn status_abi(module: &KirModule) -> bool {
    module.config.overflow_mode == KirOverflowMode::Checked
        || module.config.bounds_mode == KirBoundsMode::Checked
        || module.functions.iter().any(|function| {
            function.blocks.iter().any(|block| {
                block
                    .instructions
                    .iter()
                    .any(|instruction| matches!(instruction.kind, KirInstructionKind::Guard { .. }))
            })
        })
}

fn emit_status_declarations(out: &mut String, status: bool) {
    if status {
        out.push_str(
            "typedef int32_t CK_Status;\n\n\
             #define CK_OK ((CK_Status)0)\n\
             #define CK_ERR_OVERFLOW ((CK_Status)1)\n\
             #define CK_ERR_DIV_BY_ZERO ((CK_Status)2)\n\
             #define CK_ERR_NULL_POINTER ((CK_Status)3)\n\
             #define CK_ERR_OUT_OF_BOUNDS ((CK_Status)4)\n\n",
        );
    }
}

fn emit_type_declarations(out: &mut String, module: &KirModule, layout: &KirCModuleLayout) {
    for structure in &module.structs {
        out.push_str(&format!("typedef struct {0} {0};\n", structure.name));
    }
    if !module.structs.is_empty() {
        out.push('\n');
    }
    let slices = collect_slice_types(module);
    for slice in &slices {
        let MirType::Slice(element) = slice else {
            continue;
        };
        out.push_str(&format!(
            "typedef struct {} {{\n  {}* data;\n  uint32_t len;\n}} {};\n\n",
            layout.type_name(slice),
            layout.type_name(element),
            layout.type_name(slice)
        ));
    }
    let type_module = MirModule {
        entry: module.entry.clone(),
        structs: module.structs.clone(),
        functions: Vec::new(),
    };
    for structure in dependency_ordered_c_structs(&type_module) {
        out.push_str(&format!("struct {} {{\n", structure.name));
        for field in &structure.fields {
            out.push_str(&format!(
                "  {} {};\n",
                layout.type_name(&field.type_node),
                field.name
            ));
        }
        out.push_str("};\n\n");
    }
}

fn collect_slice_types(module: &KirModule) -> Vec<MirType> {
    let mut result = HashSet::new();
    for structure in &module.structs {
        for field in &structure.fields {
            collect_slices(&field.type_node, &mut result);
        }
    }
    for function in &module.functions {
        for param in &function.params {
            collect_slices(&param.type_node, &mut result);
        }
        collect_slices(&function.return_type, &mut result);
        for block in &function.blocks {
            for param in &block.params {
                collect_slices(&param.type_node, &mut result);
            }
            for instruction in &block.instructions {
                for kir_result in &instruction.results {
                    collect_slices(&kir_result.type_node, &mut result);
                }
            }
        }
    }
    let mut result = result.into_iter().collect::<Vec<_>>();
    result.sort_by_key(type_identity);
    result
}

fn collect_slices(type_node: &MirType, result: &mut HashSet<MirType>) {
    match type_node {
        MirType::Pointer(element) => collect_slices(element, result),
        MirType::Slice(element) => {
            collect_slices(element, result);
            result.insert(type_node.clone());
        }
        MirType::Primitive(_) | MirType::Struct(_) | MirType::Void => {}
    }
}

fn signature(
    function: &KirFunction,
    status: bool,
    restrict_params: &BTreeSet<ValueId>,
    module_layout: &KirCModuleLayout,
) -> String {
    let prefix = if function.exported {
        ""
    } else {
        "static CKC_UNUSED "
    };
    let function_layout = module_layout.function(function);
    let mut params = Vec::new();
    for param in &function.params {
        match (&param.type_node, &function_layout.params[&param.value]) {
            (MirType::Slice(element), KirCParamLayout::Slice { data, len }) => {
                params.push(format!(
                    "{}*{} {data}",
                    module_layout.type_name(element),
                    if restrict_params.contains(&param.value) {
                        " CKC_RESTRICT"
                    } else {
                        ""
                    }
                ));
                params.push(format!("uint32_t {len}"));
            }
            (MirType::Pointer(element), KirCParamLayout::Scalar(name)) => params.push(format!(
                "{}*{} {name}",
                module_layout.type_name(element),
                if restrict_params.contains(&param.value) {
                    " CKC_RESTRICT"
                } else {
                    ""
                }
            )),
            (_, KirCParamLayout::Scalar(name)) => params.push(format!(
                "{} {name}",
                module_layout.type_name(&param.type_node)
            )),
            _ => unreachable!("KIR C parameter layout matches type"),
        }
    }
    if status && function.return_type != MirType::Void {
        params.push(format!(
            "{}* {}",
            module_layout.type_name(&function.return_type),
            function_layout.return_pointer
        ));
    }
    let params = if params.is_empty() {
        "void".to_string()
    } else {
        params.join(", ")
    };
    if status {
        format!("{prefix}CK_Status {}({params})", function.name)
    } else {
        format!(
            "{prefix}{} {}({params})",
            module_layout.type_name(&function.return_type),
            function.name
        )
    }
}

fn emit_function(
    out: &mut String,
    module: &KirModule,
    function: &KirFunction,
    contracts: Option<&ContractFactSet>,
    status: bool,
    module_layout: &KirCModuleLayout,
) -> Result<(), String> {
    let fact_hints = function_fact_hints(function, contracts);
    let function_layout = module_layout.function(function);
    out.push_str(&format!(
        "{} {{\n",
        signature(function, status, &fact_hints.restrict_params, module_layout)
    ));
    let types = value_types(function);
    let guard_conditions = function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .filter_map(|instruction| match instruction.kind {
            KirInstructionKind::Guard { condition, .. } => Some(condition),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    for (value, type_node) in &types {
        out.push_str(&format!(
            "  {} {} CKC_UNUSED = {{0}};\n",
            module_layout.type_name(type_node),
            function_layout.value(*value)
        ));
    }
    if status {
        for status_name in function_layout.call_status.values() {
            out.push_str(&format!("  CK_Status {status_name} CKC_UNUSED;\n"));
        }
    }
    if status && function.return_type != MirType::Void {
        out.push_str(&format!(
            "  if ({} == NULL) return CK_ERR_NULL_POINTER;\n",
            function_layout.return_pointer
        ));
    }
    if !types.is_empty() {
        out.push('\n');
    }
    for param in &function.params {
        match (&param.type_node, &function_layout.params[&param.value]) {
            (MirType::Slice(_), KirCParamLayout::Slice { data, len }) => {
                out.push_str(&format!(
                    "  {}.data = {};\n  {}.len = {len};\n",
                    function_layout.value(param.value),
                    aligned_parameter(data, fact_hints.alignment.get(&param.value).copied()),
                    function_layout.value(param.value)
                ));
            }
            (MirType::Pointer(_), KirCParamLayout::Scalar(name)) => out.push_str(&format!(
                "  {} = {};\n",
                function_layout.value(param.value),
                aligned_parameter(name, fact_hints.alignment.get(&param.value).copied())
            )),
            (_, KirCParamLayout::Scalar(name)) => out.push_str(&format!(
                "  {} = {name};\n",
                function_layout.value(param.value)
            )),
            _ => unreachable!("KIR C parameter layout matches type"),
        }
    }
    out.push_str(&format!("  goto b{};\n", function.blocks[0].id.index()));
    let callee_types = module
        .functions
        .iter()
        .map(|function| (function.name.as_str(), function))
        .collect::<BTreeMap<_, _>>();
    let instruction_context = CInstructionContext {
        types: &types,
        functions: &callee_types,
        guard_conditions: &guard_conditions,
        status,
        module_layout,
        function_layout,
    };
    for block in &function.blocks {
        out.push_str(&format!("b{}:\n", block.id.index()));
        for instruction in &block.instructions {
            for line in instruction_lines(instruction, &instruction_context)? {
                out.push_str("  ");
                out.push_str(&line);
                out.push('\n');
            }
        }
        for line in terminator_lines(
            function,
            &block.terminator,
            status,
            module_layout,
            function_layout,
        ) {
            out.push_str("  ");
            out.push_str(&line);
            out.push('\n');
        }
    }
    out.push_str("}\n");
    Ok(())
}

#[derive(Default)]
struct CFunctionFactHints {
    restrict_params: BTreeSet<ValueId>,
    alignment: BTreeMap<ValueId, u32>,
}

fn function_fact_hints(
    function: &KirFunction,
    contracts: Option<&ContractFactSet>,
) -> CFunctionFactHints {
    let Some(contracts) = contracts else {
        return CFunctionFactHints::default();
    };
    let pointer_params = function
        .params
        .iter()
        .filter(|param| matches!(param.type_node, MirType::Pointer(_) | MirType::Slice(_)))
        .map(|param| param.value)
        .collect::<Vec<_>>();
    let mut noalias_pairs = BTreeSet::new();
    let mut alignment: BTreeMap<ValueId, u32> = BTreeMap::new();
    for instance in contracts.instances().iter().filter(|instance| {
        instance.callee == function.id
            && matches!(instance.source, ContractInstanceSource::FunctionEntry)
    }) {
        for fact in instance
            .facts
            .iter()
            .filter_map(|fact| contracts.facts().get(*fact))
        {
            let FactPredicate::Contract(predicate) = &fact.predicate else {
                continue;
            };
            match predicate {
                ContractFactPredicate::NoAlias { left, right } => {
                    noalias_pairs.insert(ordered_values(*left, *right));
                }
                ContractFactPredicate::Aligned {
                    pointer,
                    alignment: value,
                } => {
                    let pointer = match pointer {
                        ContractFactPointer::Value(value)
                        | ContractFactPointer::SliceData(value) => *value,
                    };
                    alignment
                        .entry(pointer)
                        .and_modify(|current| *current = (*current).max(*value))
                        .or_insert(*value);
                }
                _ => {}
            }
        }
    }
    let complete_noalias = pointer_params.len() >= 2
        && pointer_params.iter().enumerate().all(|(index, left)| {
            pointer_params[index + 1..]
                .iter()
                .all(|right| noalias_pairs.contains(&ordered_values(*left, *right)))
        })
        && !matches!(
            function.return_type,
            MirType::Pointer(_) | MirType::Slice(_)
        )
        && !function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .any(|instruction| {
                matches!(
                    instruction.kind,
                    KirInstructionKind::RuntimeCall { .. } | KirInstructionKind::Call { .. }
                )
            });
    CFunctionFactHints {
        restrict_params: if complete_noalias {
            pointer_params.into_iter().collect()
        } else {
            BTreeSet::new()
        },
        alignment,
    }
}

fn ordered_values(left: ValueId, right: ValueId) -> (ValueId, ValueId) {
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}

fn aligned_parameter(name: &str, alignment: Option<u32>) -> String {
    alignment.map_or_else(
        || name.to_string(),
        |alignment| format!("CKC_ASSUME_ALIGNED({name}, {alignment})"),
    )
}

struct CInstructionContext<'a> {
    types: &'a BTreeMap<ValueId, MirType>,
    functions: &'a BTreeMap<&'a str, &'a KirFunction>,
    guard_conditions: &'a BTreeSet<ValueId>,
    status: bool,
    module_layout: &'a KirCModuleLayout,
    function_layout: &'a KirCFunctionLayout,
}

fn instruction_lines(
    instruction: &KirInstruction,
    context: &CInstructionContext<'_>,
) -> Result<Vec<String>, String> {
    let result = |index: usize| {
        context
            .function_layout
            .value(instruction.results[index].value)
            .to_string()
    };
    let value = |value: ValueId| context.function_layout.value(value).to_string();
    Ok(match &instruction.kind {
        KirInstructionKind::Undef { .. } => Vec::new(),
        KirInstructionKind::ConstInt { value: constant }
        | KirInstructionKind::ConstFloat { value: constant } => {
            vec![format!("{} = {constant};", result(0))]
        }
        KirInstructionKind::ConstBool { value: constant } => vec![format!(
            "{} = {};",
            result(0),
            if *constant { "true" } else { "false" }
        )],
        KirInstructionKind::Copy { value: source } => {
            vec![format!("{} = {};", result(0), value(*source))]
        }
        KirInstructionKind::Binary {
            op,
            left,
            right,
            semantics,
        } => binary_lines(instruction, *op, *left, *right, *semantics, context),
        KirInstructionKind::Unary {
            op,
            operand,
            semantics,
        } => unary_lines(instruction, *op, *operand, *semantics, context),
        KirInstructionKind::Compare { op, left, right } => vec![format!(
            "{} = {} {} {};",
            result(0),
            value(*left),
            compare_op(*op),
            value(*right)
        )],
        KirInstructionKind::Cast { value: source, .. } => vec![format!(
            "{} = ({}){};",
            result(0),
            context
                .module_layout
                .type_name(&instruction.results[0].type_node),
            value(*source)
        )],
        KirInstructionKind::CheckCondition { kind, args } => {
            if context
                .guard_conditions
                .contains(&instruction.results[0].value)
            {
                vec![format!(
                    "{} = {};",
                    result(0),
                    check_expression(*kind, args, context.types, context.function_layout)
                )]
            } else {
                Vec::new()
            }
        }
        KirInstructionKind::Guard { condition, failure } => {
            if !context.status {
                return Err("guard requires C status ABI".to_string());
            }
            vec![format!(
                "if ({}) return {};",
                value(*condition),
                failure_status(*failure)
            )]
        }
        KirInstructionKind::Address { place } => {
            vec![format!(
                "{} = &{};",
                result(0),
                c_place(place, context.function_layout)
            )]
        }
        KirInstructionKind::Load { place } => {
            vec![format!(
                "{} = {};",
                result(0),
                c_place(place, context.function_layout)
            )]
        }
        KirInstructionKind::Store {
            place,
            value: source,
        } => vec![format!(
            "{} = {};",
            c_place(place, context.function_layout),
            value(*source)
        )],
        KirInstructionKind::MakeSlice { data, len } => vec![
            format!("{}.data = {};", result(0), value(*data)),
            format!("{}.len = {};", result(0), value(*len)),
        ],
        KirInstructionKind::SliceData { slice } => {
            vec![format!("{} = {}.data;", result(0), value(*slice))]
        }
        KirInstructionKind::SliceLen { slice } => {
            vec![format!("{} = {}.len;", result(0), value(*slice))]
        }
        KirInstructionKind::Subslice { slice, start, end } => vec![
            format!(
                "{}.data = ({} == 0 ? {}.data : {}.data + {});",
                result(0),
                value(*start),
                value(*slice),
                value(*slice),
                value(*start)
            ),
            format!("{}.len = {} - {};", result(0), value(*end), value(*start)),
        ],
        KirInstructionKind::Call {
            function_name,
            args,
        } => call_lines(
            instruction,
            context
                .functions
                .get(function_name.as_str())
                .copied()
                .ok_or_else(|| format!("missing KIR callee '{function_name}'"))?,
            args,
            context.types,
            context.status,
            context.function_layout,
        ),
        KirInstructionKind::RuntimeCall { .. } => {
            return Err("C KIR artifact cannot lower native runtime calls".to_string());
        }
    })
}

fn binary_lines(
    instruction: &KirInstruction,
    op: MirBinaryOp,
    left: ValueId,
    right: ValueId,
    semantics: KirArithmeticSemantics,
    context: &CInstructionContext<'_>,
) -> Vec<String> {
    let target = context
        .function_layout
        .value(instruction.results[0].value)
        .to_string();
    let left = context.function_layout.value(left);
    let right = context.function_layout.value(right);
    if semantics == KirArithmeticSemantics::Checked
        && instruction.results.len() == 2
        && context
            .guard_conditions
            .contains(&instruction.results[1].value)
    {
        let overflow = context.function_layout.value(instruction.results[1].value);
        let builtin = match op {
            MirBinaryOp::Add => "__builtin_add_overflow",
            MirBinaryOp::Sub => "__builtin_sub_overflow",
            MirBinaryOp::Mul => "__builtin_mul_overflow",
            MirBinaryOp::Div | MirBinaryOp::Mod => unreachable!("division uses explicit checks"),
        };
        return vec![format!(
            "{overflow} = {builtin}({left}, {right}, &{target});"
        )];
    }
    let target_type = context
        .types
        .get(&instruction.results[0].value)
        .expect("binary result type");
    let expression = if semantics == KirArithmeticSemantics::Modular
        && matches!(
            target_type,
            MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::I64)
        ) {
        let unsigned = if matches!(target_type, MirType::Primitive(MirPrimitiveTypeName::I32)) {
            "uint32_t"
        } else {
            "uint64_t"
        };
        format!(
            "({})(({unsigned}){left} {} ({unsigned}){right})",
            context.module_layout.type_name(target_type),
            binary_op(op)
        )
    } else {
        format!("{left} {} {right}", binary_op(op))
    };
    vec![format!("{target} = {expression};")]
}

fn unary_lines(
    instruction: &KirInstruction,
    op: MirUnaryOp,
    operand: ValueId,
    semantics: KirArithmeticSemantics,
    context: &CInstructionContext<'_>,
) -> Vec<String> {
    let target = context
        .function_layout
        .value(instruction.results[0].value)
        .to_string();
    let operand = context.function_layout.value(operand);
    if semantics == KirArithmeticSemantics::Checked
        && instruction.results.len() == 2
        && context
            .guard_conditions
            .contains(&instruction.results[1].value)
    {
        let overflow = context.function_layout.value(instruction.results[1].value);
        return vec![format!(
            "{overflow} = __builtin_sub_overflow(({})0, {operand}, &{target});",
            context
                .module_layout
                .type_name(&instruction.results[0].type_node)
        )];
    }
    vec![format!(
        "{target} = {}{operand};",
        match op {
            MirUnaryOp::Neg => "-",
            MirUnaryOp::Not => "!",
        }
    )]
}

fn check_expression(
    kind: KirCheckConditionKind,
    args: &[ValueId],
    types: &BTreeMap<ValueId, MirType>,
    function_layout: &KirCFunctionLayout,
) -> String {
    let value = |index: usize| function_layout.value(args[index]);
    match kind {
        KirCheckConditionKind::ArithmeticOverflow => "false".to_string(),
        KirCheckConditionKind::DivisionByZero => format!("{} == 0", value(0)),
        KirCheckConditionKind::SignedDivisionOverflow => format!(
            "{} == {} && {} == -1",
            value(0),
            signed_min(types.get(&args[0]).expect("division type")),
            value(1)
        ),
        KirCheckConditionKind::SliceOutOfBounds => {
            format!("{} >= {}.len", value(1), value(0))
        }
        KirCheckConditionKind::InvalidSubslice if args[1] == args[2] => {
            format!("{} > {}.len", value(2), value(0))
        }
        KirCheckConditionKind::InvalidSubslice => format!(
            "{} > {} || {} > {}.len",
            value(1),
            value(2),
            value(2),
            value(0)
        ),
    }
}

fn call_lines(
    instruction: &KirInstruction,
    callee: &KirFunction,
    args: &[ValueId],
    types: &BTreeMap<ValueId, MirType>,
    status: bool,
    function_layout: &KirCFunctionLayout,
) -> Vec<String> {
    let mut arguments = Vec::new();
    for arg in args {
        let name = function_layout.value(*arg).to_string();
        if matches!(types.get(arg), Some(MirType::Slice(_))) {
            arguments.push(format!("{name}.data"));
            arguments.push(format!("{name}.len"));
        } else {
            arguments.push(name);
        }
    }
    if status {
        if let Some(result) = instruction.results.first() {
            arguments.push(format!("&{}", function_layout.value(result.value)));
        }
        let status_name = &function_layout.call_status[&instruction.id];
        vec![
            format!("{status_name} = {}({});", callee.name, arguments.join(", ")),
            format!("if ({status_name} != CK_OK) return {status_name};"),
        ]
    } else {
        let call = format!("{}({})", callee.name, arguments.join(", "));
        instruction.results.first().map_or_else(
            || vec![format!("{call};")],
            |result| vec![format!("{} = {call};", function_layout.value(result.value))],
        )
    }
}

fn terminator_lines(
    function: &KirFunction,
    terminator: &KirTerminator,
    status: bool,
    module_layout: &KirCModuleLayout,
    function_layout: &KirCFunctionLayout,
) -> Vec<String> {
    match terminator {
        KirTerminator::Return { value, .. } => {
            if status {
                let mut lines = value
                    .map(|value| {
                        format!(
                            "*{} = {};",
                            function_layout.return_pointer,
                            function_layout.value(value)
                        )
                    })
                    .into_iter()
                    .collect::<Vec<_>>();
                lines.push("return CK_OK;".to_string());
                lines
            } else {
                value.map_or_else(
                    || vec!["return;".to_string()],
                    |value| vec![format!("return {};", function_layout.value(value))],
                )
            }
        }
        KirTerminator::Jump { edge } => edge_lines(function, edge, module_layout, function_layout),
        KirTerminator::Branch {
            condition,
            then_edge,
            else_edge,
        } => {
            let mut lines = vec![format!("if ({}) {{", function_layout.value(*condition))];
            lines.extend(
                edge_lines(function, then_edge, module_layout, function_layout)
                    .into_iter()
                    .map(|line| format!("  {line}")),
            );
            lines.push("} else {".to_string());
            lines.extend(
                edge_lines(function, else_edge, module_layout, function_layout)
                    .into_iter()
                    .map(|line| format!("  {line}")),
            );
            lines.push("}".to_string());
            lines
        }
    }
}

fn edge_lines(
    function: &KirFunction,
    edge: &KirEdge,
    module_layout: &KirCModuleLayout,
    function_layout: &KirCFunctionLayout,
) -> Vec<String> {
    let target = function
        .blocks
        .iter()
        .find(|block| block.id == edge.target)
        .expect("validated edge target");
    let mut lines = vec!["{".to_string()];
    for (index, (param, argument)) in target.params.iter().zip(&edge.args).enumerate() {
        lines.push(format!(
            "  {} ck_edge_{}_{} = {};",
            module_layout.type_name(&param.type_node),
            edge.target.index(),
            index,
            function_layout.value(*argument)
        ));
    }
    for (index, param) in target.params.iter().enumerate() {
        lines.push(format!(
            "  {} = ck_edge_{}_{};",
            function_layout.value(param.value),
            edge.target.index(),
            index
        ));
    }
    lines.push(format!("  goto b{};", edge.target.index()));
    lines.push("}".to_string());
    lines
}

fn value_types(function: &KirFunction) -> BTreeMap<ValueId, MirType> {
    function
        .params
        .iter()
        .map(|param| (param.value, param.type_node.clone()))
        .chain(function.blocks.iter().flat_map(|block| {
            block
                .params
                .iter()
                .map(|param| (param.value, param.type_node.clone()))
                .chain(block.instructions.iter().flat_map(|instruction| {
                    instruction
                        .results
                        .iter()
                        .map(|result| (result.value, result.type_node.clone()))
                }))
        }))
        .collect()
}

fn c_place(place: &KirPlace, function_layout: &KirCFunctionLayout) -> String {
    match place {
        KirPlace::Value { value, .. } => function_layout.value(*value).to_string(),
        KirPlace::Deref { pointer, .. } => format!("*{}", function_layout.value(*pointer)),
        KirPlace::Index { base, index, .. } => {
            format!(
                "{}[{}]",
                parenthesized_place(base, function_layout),
                function_layout.value(*index)
            )
        }
        KirPlace::SliceIndex { slice, index, .. } => {
            format!(
                "{}.data[{}]",
                function_layout.value(*slice),
                function_layout.value(*index)
            )
        }
        KirPlace::Field {
            base, field_name, ..
        } => format!(
            "{}.{}",
            parenthesized_place(base, function_layout),
            field_name
        ),
    }
}

fn parenthesized_place(place: &KirPlace, function_layout: &KirCFunctionLayout) -> String {
    match place {
        KirPlace::Deref { .. } => format!("({})", c_place(place, function_layout)),
        _ => c_place(place, function_layout),
    }
}

fn type_identity(type_node: &MirType) -> String {
    match type_node {
        MirType::Primitive(name) => format!("{name:?}").to_lowercase(),
        MirType::Pointer(element) => format!("ptr_{}", type_identity(element)),
        MirType::Slice(element) => format!("slice_{}", type_identity(element)),
        MirType::Struct(name) => name.clone(),
        MirType::Void => "void".to_string(),
    }
}

fn binary_op(op: MirBinaryOp) -> &'static str {
    match op {
        MirBinaryOp::Add => "+",
        MirBinaryOp::Sub => "-",
        MirBinaryOp::Mul => "*",
        MirBinaryOp::Div => "/",
        MirBinaryOp::Mod => "%",
    }
}

fn compare_op(op: MirCompareOp) -> &'static str {
    match op {
        MirCompareOp::Eq => "==",
        MirCompareOp::Ne => "!=",
        MirCompareOp::Lt => "<",
        MirCompareOp::Le => "<=",
        MirCompareOp::Gt => ">",
        MirCompareOp::Ge => ">=",
    }
}

fn signed_min(type_node: &MirType) -> &'static str {
    match type_node {
        MirType::Primitive(MirPrimitiveTypeName::I32) => "INT32_MIN",
        MirType::Primitive(MirPrimitiveTypeName::I64) => "INT64_MIN",
        _ => "0",
    }
}

fn failure_status(failure: KirFailureKind) -> &'static str {
    match failure {
        KirFailureKind::Overflow => "CK_ERR_OVERFLOW",
        KirFailureKind::DivisionByZero => "CK_ERR_DIV_BY_ZERO",
        KirFailureKind::OutOfBounds => "CK_ERR_OUT_OF_BOUNDS",
        KirFailureKind::ContractViolation => "CK_ERR_OUT_OF_BOUNDS",
    }
}
