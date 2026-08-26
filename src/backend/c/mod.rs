mod checked;
mod emit;
mod layout;
mod names;
mod options;

pub use emit::{emit_c_header, emit_c_module, emit_c_module_with_header};
pub use options::{BoundsMode, EmitCOptions, OverflowMode};
