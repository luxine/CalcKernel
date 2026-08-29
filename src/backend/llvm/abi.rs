use crate::{MirFunction, MirModule, MirType};

use super::{
    builder::{NativeBuilder, NativeFunction, NativeType, NativeValue},
    context::NativeContext,
    error::{NativeError, NativeStage},
    lower_shared::TypeRegistry,
    module::NativeModule,
    target::NativeTarget,
};
use crate::backend::native_abi::{
    NativeAbiClassifier, NativeAbiExtension, NativeAbiPassMode, NativeAbiRegister,
    NativeAbiRegisterClass, NativeAbiTarget,
};

pub(super) fn implementation_name(function: &MirFunction) -> String {
    if function.exported {
        format!("__ck_impl_{}", function.name)
    } else {
        function.name.clone()
    }
}

pub(super) fn add_export_thunks<'module, 'context>(
    context: &'context NativeContext,
    module: &'module NativeModule<'context>,
    target: &NativeTarget,
    mir: &MirModule,
    types: &TypeRegistry<'context>,
    implementations: &std::collections::HashMap<String, NativeFunction<'module>>,
    checked: bool,
) -> Result<(), NativeError> {
    let triple = target.triple()?;
    let abi_target = NativeAbiTarget::from_triple(&triple).map_err(abi_error)?;
    let classifier = NativeAbiClassifier::new(abi_target, &mir.structs).map_err(abi_error)?;
    for function in mir.functions.iter().filter(|function| function.exported) {
        let implementation = implementations
            .get(&function.name)
            .copied()
            .ok_or_else(|| {
                abi_lowering_error(format!("missing implementation '{}'", function.name))
            })?;
        add_export_thunk(
            context,
            module,
            types,
            &classifier,
            function,
            implementation,
            checked,
        )?;
    }
    Ok(())
}

struct ParameterBinding<'context> {
    source_type: MirType,
    source_llvm_type: NativeType<'context>,
    external_types: Vec<NativeType<'context>>,
    indirect: bool,
    by_value: bool,
    alignment: u32,
    extension: NativeAbiExtension,
}

struct ThunkSignature<'context> {
    return_type: NativeType<'context>,
    return_coercion: Option<NativeType<'context>>,
    source_return_type: NativeType<'context>,
    parameters: Vec<NativeType<'context>>,
    bindings: Vec<ParameterBinding<'context>>,
    hidden_result: bool,
    checked_result: bool,
    result_alignment: u32,
    return_extension: NativeAbiExtension,
}

fn add_export_thunk<'module, 'context>(
    context: &'context NativeContext,
    module: &'module NativeModule<'context>,
    types: &TypeRegistry<'context>,
    classifier: &NativeAbiClassifier,
    function: &MirFunction,
    implementation: NativeFunction<'module>,
    checked: bool,
) -> Result<(), NativeError> {
    let signature = thunk_signature(context, types, classifier, function, checked)?;
    let external = module.add_function(
        &function.name,
        signature.return_type,
        &signature.parameters,
        true,
    )?;
    if matches!(
        classifier.target(),
        NativeAbiTarget::WindowsX86_64 | NativeAbiTarget::WindowsArm64
    ) {
        external.set_dll_export()?;
    }
    external.add_return_extension(signature.return_extension)?;

    let mut attribute_index = 0;
    if signature.hidden_result {
        external.add_sret(0, signature.source_return_type, signature.result_alignment)?;
        attribute_index = 1;
    }
    for binding in &signature.bindings {
        if binding.indirect {
            if binding.by_value {
                external.add_byval(attribute_index, binding.source_llvm_type, binding.alignment)?;
            }
            attribute_index += 1;
        } else {
            if binding.external_types.len() == 1 {
                external.add_param_extension(attribute_index, binding.extension)?;
            }
            attribute_index += binding.external_types.len();
        }
    }

    let entry = external.append_block("entry")?;
    let mut builder = NativeBuilder::new(context, module)?;
    builder.position(entry)?;
    let mut external_index = 0;
    let hidden_result = if signature.hidden_result {
        let value = external.param(0, "ck_sret")?;
        external_index = 1;
        Some(value)
    } else {
        None
    };
    let mut internal_arguments = Vec::new();
    for (source_index, binding) in signature.bindings.iter().enumerate() {
        if matches!(binding.source_type, MirType::Slice(_)) {
            internal_arguments.push(external.param(external_index, "slice.data")?);
            internal_arguments.push(external.param(external_index + 1, "slice.len")?);
            external_index += 2;
            continue;
        }
        if binding.indirect {
            let pointer = external.param(external_index, &format!("arg{source_index}.indirect"))?;
            external_index += 1;
            internal_arguments.push(builder.load(
                binding.source_llvm_type,
                pointer,
                &format!("arg{source_index}.value"),
            )?);
            continue;
        }
        if is_aggregate(&binding.source_type) {
            let coerced = if binding.external_types.len() == 1 {
                let value = external.param(external_index, &format!("arg{source_index}.abi"))?;
                external_index += 1;
                value
            } else {
                let coercion = NativeType::literal_struct(context, &binding.external_types)?;
                let mut value = builder.undef(coercion)?;
                for (part_index, _) in binding.external_types.iter().enumerate() {
                    let part = external.param(
                        external_index,
                        &format!("arg{source_index}.part{part_index}"),
                    )?;
                    external_index += 1;
                    value = builder.insert_value(
                        value,
                        part,
                        part_index as u32,
                        &format!("arg{source_index}.joined{part_index}"),
                    )?;
                }
                value
            };
            internal_arguments.push(reinterpret_value(
                &mut builder,
                coerced,
                binding.source_llvm_type,
                &format!("arg{source_index}"),
            )?);
        } else {
            internal_arguments.push(external.param(external_index, &format!("arg{source_index}"))?);
            external_index += 1;
        }
    }
    if signature.checked_result {
        internal_arguments.push(external.param(external_index, "ck_return")?);
        external_index += 1;
    }
    debug_assert_eq!(external_index, signature.parameters.len());

    let result = builder.call(
        implementation,
        &internal_arguments,
        if !checked && matches!(function.return_type, MirType::Void) {
            ""
        } else {
            "ck.impl"
        },
    )?;
    if checked {
        return builder.return_value(result);
    }
    if matches!(function.return_type, MirType::Void) {
        return builder.return_void();
    }
    if let Some(pointer) = hidden_result {
        builder.store(result, pointer)?;
        return builder.return_void();
    }
    if let Some(coercion) = signature.return_coercion {
        let result = reinterpret_value(&mut builder, result, coercion, "result")?;
        return builder.return_value(result);
    }
    builder.return_value(result)
}

fn thunk_signature<'context>(
    context: &'context NativeContext,
    types: &TypeRegistry<'context>,
    classifier: &NativeAbiClassifier,
    function: &MirFunction,
    checked: bool,
) -> Result<ThunkSignature<'context>, NativeError> {
    let source_return_type = types.get(&function.return_type)?;
    let result = classifier
        .classify_return(&function.return_type)
        .map_err(abi_error)?;
    let mut parameters = Vec::new();
    let mut hidden_result = false;
    let mut result_alignment = result.layout.alignment;
    let (return_type, return_coercion, return_extension) = if checked {
        (types.i32, None, NativeAbiExtension::None)
    } else if matches!(function.return_type, MirType::Void) {
        (types.void, None, NativeAbiExtension::None)
    } else if is_aggregate(&function.return_type) {
        match &result.mode {
            NativeAbiPassMode::Indirect { alignment, .. } => {
                parameters.push(types.pointer);
                hidden_result = true;
                result_alignment = *alignment;
                (types.void, None, NativeAbiExtension::None)
            }
            NativeAbiPassMode::Direct { registers } => {
                let coercion = aggregate_coercion_type(context, classifier.target(), registers)?;
                (coercion, Some(coercion), NativeAbiExtension::None)
            }
        }
    } else {
        (source_return_type, None, result.extension)
    };

    let mut bindings = Vec::new();
    for parameter in &function.params {
        if matches!(parameter.type_node, MirType::Slice(_)) {
            let binding = ParameterBinding {
                source_type: parameter.type_node.clone(),
                source_llvm_type: types.get(&parameter.type_node)?,
                external_types: vec![types.pointer, types.i32],
                indirect: false,
                by_value: false,
                alignment: 8,
                extension: NativeAbiExtension::None,
            };
            parameters.extend(binding.external_types.iter().copied());
            bindings.push(binding);
            continue;
        }
        let classified = classifier
            .classify_parameter(&parameter.type_node)
            .map_err(abi_error)?;
        let source_llvm_type = types.get(&parameter.type_node)?;
        let binding = if is_aggregate(&parameter.type_node) {
            match &classified.mode {
                NativeAbiPassMode::Indirect {
                    by_value,
                    alignment,
                } => ParameterBinding {
                    source_type: parameter.type_node.clone(),
                    source_llvm_type,
                    external_types: vec![types.pointer],
                    indirect: true,
                    by_value: *by_value,
                    alignment: *alignment,
                    extension: NativeAbiExtension::None,
                },
                NativeAbiPassMode::Direct { registers } => {
                    let external_types =
                        aggregate_parameter_types(context, classifier.target(), registers)?;
                    ParameterBinding {
                        source_type: parameter.type_node.clone(),
                        source_llvm_type,
                        external_types,
                        indirect: false,
                        by_value: false,
                        alignment: classified.layout.alignment,
                        extension: NativeAbiExtension::None,
                    }
                }
            }
        } else {
            ParameterBinding {
                source_type: parameter.type_node.clone(),
                source_llvm_type,
                external_types: vec![source_llvm_type],
                indirect: false,
                by_value: false,
                alignment: classified.layout.alignment,
                extension: classified.extension,
            }
        };
        parameters.extend(binding.external_types.iter().copied());
        bindings.push(binding);
    }
    let checked_result = checked && !matches!(function.return_type, MirType::Void);
    if checked_result {
        parameters.push(types.pointer);
    }
    Ok(ThunkSignature {
        return_type,
        return_coercion,
        source_return_type,
        parameters,
        bindings,
        hidden_result,
        checked_result,
        result_alignment,
        return_extension,
    })
}

fn aggregate_parameter_types<'context>(
    context: &'context NativeContext,
    target: NativeAbiTarget,
    registers: &[NativeAbiRegister],
) -> Result<Vec<NativeType<'context>>, NativeError> {
    let register_types = register_types(context, registers)?;
    if matches!(
        target,
        NativeAbiTarget::SysvX86_64 | NativeAbiTarget::DarwinX86_64
    ) {
        return Ok(register_types);
    }
    Ok(vec![group_register_types(&register_types)?])
}

fn aggregate_coercion_type<'context>(
    context: &'context NativeContext,
    target: NativeAbiTarget,
    registers: &[NativeAbiRegister],
) -> Result<NativeType<'context>, NativeError> {
    let register_types = register_types(context, registers)?;
    if register_types.len() == 1 {
        return Ok(register_types[0]);
    }
    if matches!(
        target,
        NativeAbiTarget::SysvX86_64 | NativeAbiTarget::DarwinX86_64
    ) {
        NativeType::literal_struct(context, &register_types)
    } else {
        group_register_types(&register_types)
    }
}

fn group_register_types<'context>(
    register_types: &[NativeType<'context>],
) -> Result<NativeType<'context>, NativeError> {
    let Some(first) = register_types.first().copied() else {
        return Err(abi_lowering_error("aggregate has no ABI registers"));
    };
    if register_types.len() == 1 {
        Ok(first)
    } else {
        NativeType::array(first, register_types.len() as u32)
    }
}

fn register_types<'context>(
    context: &'context NativeContext,
    registers: &[NativeAbiRegister],
) -> Result<Vec<NativeType<'context>>, NativeError> {
    registers
        .iter()
        .map(|register| match register.class {
            NativeAbiRegisterClass::Integer => NativeType::int(context, u32::from(register.bits)),
            NativeAbiRegisterClass::Floating if register.bits == 64 => NativeType::f64(context),
            NativeAbiRegisterClass::Floating => Err(abi_lowering_error(format!(
                "unsupported floating ABI register width {}",
                register.bits
            ))),
        })
        .collect()
}

fn reinterpret_value<'module, 'context>(
    builder: &mut NativeBuilder<'module, 'context>,
    value: NativeValue<'module>,
    target: NativeType<'context>,
    name: &str,
) -> Result<NativeValue<'module>, NativeError> {
    let storage = builder.alloca(target, &format!("{name}.storage"))?;
    builder.store(value, storage)?;
    builder.load(target, storage, &format!("{name}.coerced"))
}

fn is_aggregate(type_node: &MirType) -> bool {
    matches!(type_node, MirType::Struct(_) | MirType::Slice(_))
}

fn abi_error(error: impl std::fmt::Display) -> NativeError {
    abi_lowering_error(error.to_string())
}

fn abi_lowering_error(message: impl Into<String>) -> NativeError {
    NativeError::new(NativeStage::Module, 1, message.into())
}
