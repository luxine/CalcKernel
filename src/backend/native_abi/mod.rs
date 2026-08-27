mod aapcs64;
mod darwin_x64;
mod model;
mod sysv_x64;
mod windows;

use std::collections::{HashMap, HashSet};

use crate::{MirFunction, MirPrimitiveTypeName, MirStruct, MirType};

use model::NativeAbiPosition;
pub use model::{
    NativeAbiArgument, NativeAbiArgumentRole, NativeAbiError, NativeAbiExtension,
    NativeAbiFunction, NativeAbiHiddenResult, NativeAbiLayout, NativeAbiPassMode,
    NativeAbiRegister, NativeAbiRegisterClass, NativeAbiTarget, NativeAbiValue,
};

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct AggregateChunk {
    class: Option<NativeAbiRegisterClass>,
    used_bits: u16,
}

/// Explicit target-family C ABI classifier for CK exported shapes.
#[derive(Debug, Clone)]
pub struct NativeAbiClassifier {
    target: NativeAbiTarget,
    structs: HashMap<String, MirStruct>,
}

impl NativeAbiClassifier {
    pub fn new(target: NativeAbiTarget, structs: &[MirStruct]) -> Result<Self, NativeAbiError> {
        let mut by_name = HashMap::new();
        for structure in structs {
            if by_name
                .insert(structure.name.clone(), structure.clone())
                .is_some()
            {
                return Err(NativeAbiError::new(format!(
                    "duplicate MIR struct '{}' in Native ABI classifier",
                    structure.name
                )));
            }
        }
        let classifier = Self {
            target,
            structs: by_name,
        };
        for structure in structs {
            classifier.layout(&MirType::Struct(structure.name.clone()))?;
        }
        Ok(classifier)
    }

    #[must_use]
    pub const fn target(&self) -> NativeAbiTarget {
        self.target
    }

    pub fn layout(&self, type_node: &MirType) -> Result<NativeAbiLayout, NativeAbiError> {
        self.layout_inner(type_node, &mut HashSet::new())
    }

    pub fn classify_parameter(
        &self,
        type_node: &MirType,
    ) -> Result<NativeAbiValue, NativeAbiError> {
        self.classify(type_node, NativeAbiPosition::Parameter)
    }

    pub fn classify_return(&self, type_node: &MirType) -> Result<NativeAbiValue, NativeAbiError> {
        self.classify(type_node, NativeAbiPosition::Return)
    }

    pub fn classify_function(
        &self,
        function: &MirFunction,
        checked: bool,
    ) -> Result<NativeAbiFunction, NativeAbiError> {
        let mut parameters = Vec::new();
        for (source_index, parameter) in function.params.iter().enumerate() {
            if let MirType::Slice(element) = &parameter.type_node {
                parameters.push(NativeAbiArgument {
                    role: NativeAbiArgumentRole::SliceData(source_index),
                    value: self.classify_parameter(&MirType::Pointer(element.clone()))?,
                });
                parameters.push(NativeAbiArgument {
                    role: NativeAbiArgumentRole::SliceLength(source_index),
                    value: self
                        .classify_parameter(&MirType::Primitive(MirPrimitiveTypeName::U32))?,
                });
            } else {
                parameters.push(NativeAbiArgument {
                    role: NativeAbiArgumentRole::Source(source_index),
                    value: self.classify_parameter(&parameter.type_node)?,
                });
            }
        }

        if checked {
            if !matches!(function.return_type, MirType::Void) {
                parameters.push(NativeAbiArgument {
                    role: NativeAbiArgumentRole::CheckedResult,
                    value: self.classify_parameter(&MirType::Pointer(Box::new(
                        function.return_type.clone(),
                    )))?,
                });
            }
            return Ok(NativeAbiFunction {
                return_value: self
                    .classify_return(&MirType::Primitive(MirPrimitiveTypeName::I32))?,
                parameters,
                hidden_result: None,
            });
        }

        let source_return = self.classify_return(&function.return_type)?;
        let (return_value, hidden_result) = match &source_return.mode {
            NativeAbiPassMode::Indirect { alignment, .. } => (
                self.classify_return(&MirType::Void)?,
                Some(NativeAbiHiddenResult {
                    alignment: *alignment,
                }),
            ),
            NativeAbiPassMode::Direct { .. } => (source_return, None),
        };
        Ok(NativeAbiFunction {
            return_value,
            parameters,
            hidden_result,
        })
    }

    fn classify(
        &self,
        type_node: &MirType,
        position: NativeAbiPosition,
    ) -> Result<NativeAbiValue, NativeAbiError> {
        let layout = self.layout(type_node)?;
        let extension = if matches!(type_node, MirType::Primitive(MirPrimitiveTypeName::Bool))
            && matches!(
                self.target,
                NativeAbiTarget::SysvX86_64
                    | NativeAbiTarget::DarwinX86_64
                    | NativeAbiTarget::Aapcs64Darwin
                    | NativeAbiTarget::WindowsX86_64
            ) {
            NativeAbiExtension::Zero
        } else {
            NativeAbiExtension::None
        };
        let mode = match type_node {
            MirType::Void => NativeAbiPassMode::Direct { registers: vec![] },
            MirType::Primitive(MirPrimitiveTypeName::F64) => NativeAbiPassMode::Direct {
                registers: vec![NativeAbiRegister::floating(64)],
            },
            MirType::Primitive(MirPrimitiveTypeName::Bool) => NativeAbiPassMode::Direct {
                registers: vec![NativeAbiRegister::integer(1)],
            },
            MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32) => {
                NativeAbiPassMode::Direct {
                    registers: vec![NativeAbiRegister::integer(32)],
                }
            }
            MirType::Primitive(MirPrimitiveTypeName::I64 | MirPrimitiveTypeName::U64)
            | MirType::Pointer(_) => NativeAbiPassMode::Direct {
                registers: vec![NativeAbiRegister::integer(64)],
            },
            MirType::Slice(_) | MirType::Struct(_) => match self.target {
                NativeAbiTarget::SysvX86_64 => sysv_x64::classify(self, type_node, position)?,
                NativeAbiTarget::DarwinX86_64 => darwin_x64::classify(self, type_node, position)?,
                NativeAbiTarget::Aapcs64Linux | NativeAbiTarget::Aapcs64Darwin => {
                    aapcs64::classify(self, type_node, position)?
                }
                NativeAbiTarget::WindowsX86_64 => windows::classify_x64(self, type_node, position)?,
                NativeAbiTarget::WindowsArm64 => {
                    windows::classify_arm64(self, type_node, position)?
                }
            },
        };
        Ok(NativeAbiValue {
            layout,
            mode,
            extension,
        })
    }

    fn layout_inner(
        &self,
        type_node: &MirType,
        active: &mut HashSet<String>,
    ) -> Result<NativeAbiLayout, NativeAbiError> {
        match type_node {
            MirType::Void => Ok(NativeAbiLayout {
                size: 0,
                alignment: 1,
            }),
            MirType::Primitive(MirPrimitiveTypeName::Bool) => Ok(NativeAbiLayout {
                size: 1,
                alignment: 1,
            }),
            MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32) => {
                Ok(NativeAbiLayout {
                    size: 4,
                    alignment: 4,
                })
            }
            MirType::Primitive(
                MirPrimitiveTypeName::I64 | MirPrimitiveTypeName::U64 | MirPrimitiveTypeName::F64,
            )
            | MirType::Pointer(_) => Ok(NativeAbiLayout {
                size: 8,
                alignment: 8,
            }),
            MirType::Slice(_) => Ok(NativeAbiLayout {
                size: 16,
                alignment: 8,
            }),
            MirType::Struct(name) => {
                if !active.insert(name.clone()) {
                    return Err(NativeAbiError::new(format!(
                        "directly recursive MIR struct '{name}' has no finite Native ABI layout"
                    )));
                }
                let structure = self.structure(name)?;
                let mut size = 0;
                let mut alignment = 1;
                for field in &structure.fields {
                    let field_layout = self.layout_inner(&field.type_node, active)?;
                    size = align_to(size, field_layout.alignment)?;
                    size = size.checked_add(field_layout.size).ok_or_else(|| {
                        NativeAbiError::new(format!("Native ABI struct '{name}' is too large"))
                    })?;
                    alignment = alignment.max(field_layout.alignment);
                }
                active.remove(name);
                Ok(NativeAbiLayout {
                    size: align_to(size, alignment)?,
                    alignment,
                })
            }
        }
    }

    pub(super) fn homogeneous_f64_members(
        &self,
        type_node: &MirType,
    ) -> Result<Option<u8>, NativeAbiError> {
        match type_node {
            MirType::Primitive(MirPrimitiveTypeName::F64) => Ok(Some(1)),
            MirType::Struct(name) => {
                let structure = self.structure(name)?;
                if structure.fields.is_empty() {
                    return Ok(None);
                }
                let mut total = 0u8;
                for field in &structure.fields {
                    let Some(count) = self.homogeneous_f64_members(&field.type_node)? else {
                        return Ok(None);
                    };
                    total = total.saturating_add(count);
                }
                Ok(Some(total))
            }
            _ => Ok(None),
        }
    }

    pub(super) fn classify_sysv_chunks(
        &self,
        type_node: &MirType,
        offset: u32,
        chunks: &mut [AggregateChunk],
    ) -> Result<(), NativeAbiError> {
        match type_node {
            MirType::Primitive(MirPrimitiveTypeName::F64) => {
                mark_chunk(chunks, offset, 8, NativeAbiRegisterClass::Floating)
            }
            MirType::Primitive(MirPrimitiveTypeName::Bool) => {
                mark_chunk(chunks, offset, 1, NativeAbiRegisterClass::Integer)
            }
            MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32) => {
                mark_chunk(chunks, offset, 4, NativeAbiRegisterClass::Integer)
            }
            MirType::Primitive(MirPrimitiveTypeName::I64 | MirPrimitiveTypeName::U64)
            | MirType::Pointer(_) => mark_chunk(chunks, offset, 8, NativeAbiRegisterClass::Integer),
            MirType::Slice(_) => {
                mark_chunk(chunks, offset, 8, NativeAbiRegisterClass::Integer);
                mark_chunk(chunks, offset + 8, 4, NativeAbiRegisterClass::Integer);
            }
            MirType::Struct(name) => {
                let structure = self.structure(name)?;
                let mut field_offset = offset;
                for field in &structure.fields {
                    let layout = self.layout(&field.type_node)?;
                    field_offset = align_to(field_offset, layout.alignment)?;
                    self.classify_sysv_chunks(&field.type_node, field_offset, chunks)?;
                    field_offset += layout.size;
                }
            }
            MirType::Void => {}
        }
        Ok(())
    }

    fn structure(&self, name: &str) -> Result<&MirStruct, NativeAbiError> {
        self.structs
            .get(name)
            .ok_or_else(|| NativeAbiError::new(format!("unknown MIR struct '{name}'")))
    }
}

fn align_to(value: u32, alignment: u32) -> Result<u32, NativeAbiError> {
    let mask = alignment - 1;
    value
        .checked_add(mask)
        .map(|value| value & !mask)
        .ok_or_else(|| NativeAbiError::new("Native ABI layout overflow"))
}

fn mark_chunk(
    chunks: &mut [AggregateChunk],
    offset: u32,
    size: u32,
    class: NativeAbiRegisterClass,
) {
    let index = (offset / 8) as usize;
    let chunk_offset = offset % 8;
    if let Some(chunk) = chunks.get_mut(index) {
        chunk.class = Some(match (chunk.class, class) {
            (Some(NativeAbiRegisterClass::Integer), _) | (_, NativeAbiRegisterClass::Integer) => {
                NativeAbiRegisterClass::Integer
            }
            _ => NativeAbiRegisterClass::Floating,
        });
        chunk.used_bits = chunk
            .used_bits
            .max(((chunk_offset + size.min(8 - chunk_offset)) * 8) as u16);
    }
    if chunk_offset + size > 8 {
        mark_chunk(
            chunks,
            offset + (8 - chunk_offset),
            size - (8 - chunk_offset),
            class,
        );
    }
}
