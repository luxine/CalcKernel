mod kir;
mod layout;
mod names;
mod options;

pub(in crate::backend) use kir::emit_c_kir_header_with_mode;
pub use kir::{emit_c_kir_header, emit_c_kir_module, emit_c_kir_module_with_contracts};
pub use options::{BoundsMode, EmitCOptions, OverflowMode};
