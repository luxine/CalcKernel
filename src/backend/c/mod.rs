mod checked;
mod emit;
mod layout;
mod names;
mod options;

pub(in crate::backend) use emit::emit_c_header_with_mode;
pub use emit::{emit_c_header, emit_c_module, emit_c_module_with_header, try_emit_c_module};
pub use options::{BoundsMode, EmitCOptions, OverflowMode};
