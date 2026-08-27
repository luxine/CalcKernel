use crate::MirModule;

use super::{EmitCOptions, c::emit_c_header_with_mode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeHeaderMode {
    Dynamic,
    StaticOrObject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HeaderExportMode {
    Dynamic,
    StaticOrObject,
}

/// Emits the authoritative Native C ABI header for a library artifact.
#[must_use]
pub fn emit_native_header(
    module: &MirModule,
    options: EmitCOptions,
    mode: NativeHeaderMode,
) -> String {
    emit_c_header_with_mode(
        module,
        options,
        match mode {
            NativeHeaderMode::Dynamic => HeaderExportMode::Dynamic,
            NativeHeaderMode::StaticOrObject => HeaderExportMode::StaticOrObject,
        },
    )
}
