use std::{
    collections::{HashMap, HashSet, VecDeque},
    error::Error,
    fmt,
};

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirInstructionEffect {
    Pure,
    ReadMemory,
    WriteMemory,
    UnknownCall,
    ObservableOutput,
}

impl MirInstructionEffect {
    #[must_use]
    pub const fn invalidates_value_facts(self) -> bool {
        matches!(
            self,
            Self::WriteMemory | Self::UnknownCall | Self::ObservableOutput
        )
    }
}

#[must_use]
pub const fn instruction_effect(instruction: &MirInstruction) -> MirInstructionEffect {
    match instruction {
        MirInstruction::ConstInt { .. }
        | MirInstruction::ConstFloat { .. }
        | MirInstruction::ConstBool { .. }
        | MirInstruction::Move { .. }
        | MirInstruction::Binary { .. }
        | MirInstruction::Unary { .. }
        | MirInstruction::Compare { .. }
        | MirInstruction::Cast { .. }
        | MirInstruction::Address { .. }
        | MirInstruction::MakeSlice { .. }
        | MirInstruction::SliceData { .. }
        | MirInstruction::SliceLen { .. }
        | MirInstruction::Subslice { .. } => MirInstructionEffect::Pure,
        MirInstruction::Load { .. } => MirInstructionEffect::ReadMemory,
        MirInstruction::Store { .. } => MirInstructionEffect::WriteMemory,
        MirInstruction::Call { .. } => MirInstructionEffect::UnknownCall,
        MirInstruction::RuntimeCall { .. } => MirInstructionEffect::ObservableOutput,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirArtifactConsumer {
    C,
    WebAssembly,
    NativeLibrary,
    NativeExecutable,
}

impl fmt::Display for MirArtifactConsumer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::C => "C",
            Self::WebAssembly => "WebAssembly",
            Self::NativeLibrary => "native library",
            Self::NativeExecutable => "native executable",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirArtifactError {
    pub message: String,
}

impl fmt::Display for MirArtifactError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl Error for MirArtifactError {}

#[must_use]
pub fn exported_artifact_roots(module: &MirModule) -> Vec<String> {
    module
        .functions
        .iter()
        .filter(|function| function.exported)
        .map(|function| function.name.clone())
        .collect()
}

#[must_use]
pub fn optimizer_artifact_roots(module: &MirModule) -> Vec<String> {
    let mut roots = exported_artifact_roots(module);
    if let Some(entry) = &module.entry
        && !roots.contains(&entry.function_name)
    {
        roots.push(entry.function_name.clone());
    }
    roots
}

#[must_use]
pub fn reachable_function_names(module: &MirModule, roots: &[String]) -> Vec<String> {
    let functions = module
        .functions
        .iter()
        .map(|function| (function.name.as_str(), function))
        .collect::<HashMap<_, _>>();
    let mut reachable = HashSet::new();
    let mut queue = roots.iter().cloned().collect::<VecDeque<_>>();
    while let Some(name) = queue.pop_front() {
        if !reachable.insert(name.clone()) {
            continue;
        }
        let Some(function) = functions.get(name.as_str()) else {
            continue;
        };
        for block in &function.blocks {
            for instruction in &block.instructions {
                if let MirInstruction::Call { function_name, .. } = instruction
                    && !reachable.contains(function_name)
                {
                    queue.push_back(function_name.clone());
                }
            }
        }
    }
    module
        .functions
        .iter()
        .filter(|function| reachable.contains(&function.name))
        .map(|function| function.name.clone())
        .collect()
}

pub fn prepare_non_executable_artifact(
    module: &MirModule,
    consumer: MirArtifactConsumer,
) -> Result<MirModule, MirArtifactError> {
    let roots = exported_artifact_roots(module);
    prepare_non_executable_artifact_from_roots(module, consumer, &roots)
}

pub fn prepare_non_executable_artifact_from_roots(
    module: &MirModule,
    consumer: MirArtifactConsumer,
    roots: &[String],
) -> Result<MirModule, MirArtifactError> {
    prepare_artifact(module, roots, consumer, false, false)
}

pub fn prepare_executable_artifact(module: &MirModule) -> Result<MirModule, MirArtifactError> {
    let entry = module.entry.as_ref().ok_or_else(|| MirArtifactError {
        message: "native executable artifact requires MIR entry metadata".to_string(),
    })?;
    prepare_artifact(
        module,
        std::slice::from_ref(&entry.function_name),
        MirArtifactConsumer::NativeExecutable,
        true,
        true,
    )
}

fn prepare_artifact(
    module: &MirModule,
    roots: &[String],
    consumer: MirArtifactConsumer,
    allow_runtime: bool,
    preserve_entry: bool,
) -> Result<MirModule, MirArtifactError> {
    let functions = module
        .functions
        .iter()
        .map(|function| (function.name.as_str(), function))
        .collect::<HashMap<_, _>>();
    let mut visited = HashSet::new();
    let mut queue = roots
        .iter()
        .map(|root| (root.clone(), vec![root.clone()]))
        .collect::<VecDeque<_>>();

    while let Some((name, path)) = queue.pop_front() {
        if !visited.insert(name.clone()) {
            continue;
        }
        let function = functions
            .get(name.as_str())
            .ok_or_else(|| MirArtifactError {
                message: format!(
                    "{consumer} artifact root path '{}' names a missing function",
                    path.join(" -> ")
                ),
            })?;
        for block in &function.blocks {
            for instruction in &block.instructions {
                match instruction {
                    MirInstruction::RuntimeCall { intrinsic, .. } if !allow_runtime => {
                        return Err(MirArtifactError {
                            message: format!(
                                "{consumer} artifact root '{}' reaches native-only runtime intrinsic '{}' through {}.",
                                path.first().expect("artifact path has a root"),
                                print_runtime_intrinsic(*intrinsic),
                                path.join(" -> ")
                            ),
                        });
                    }
                    MirInstruction::Call { function_name, .. }
                        if !visited.contains(function_name) =>
                    {
                        let mut callee_path = path.clone();
                        callee_path.push(function_name.clone());
                        queue.push_back((function_name.clone(), callee_path));
                    }
                    MirInstruction::ConstInt { .. }
                    | MirInstruction::ConstFloat { .. }
                    | MirInstruction::ConstBool { .. }
                    | MirInstruction::Move { .. }
                    | MirInstruction::Binary { .. }
                    | MirInstruction::Unary { .. }
                    | MirInstruction::Compare { .. }
                    | MirInstruction::Cast { .. }
                    | MirInstruction::Address { .. }
                    | MirInstruction::Load { .. }
                    | MirInstruction::Store { .. }
                    | MirInstruction::MakeSlice { .. }
                    | MirInstruction::SliceData { .. }
                    | MirInstruction::SliceLen { .. }
                    | MirInstruction::Subslice { .. }
                    | MirInstruction::RuntimeCall { .. }
                    | MirInstruction::Call { .. } => {}
                }
            }
        }
    }

    let mut artifact = module.clone();
    artifact
        .functions
        .retain(|function| visited.contains(&function.name));
    if !preserve_entry {
        artifact.entry = None;
    }
    Ok(artifact)
}
