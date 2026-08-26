#![allow(dead_code)]

use std::path::{Path, PathBuf};

use calckernel::{
    MirModule, MirPassBoundsMode, MirPassContext, MirPassOverflowMode, MirPassTargetBackend,
    SourceFile, build_mir_optimization_pipeline, check, lower_to_mir, run_mir_pass_pipeline,
};

pub(crate) fn optimized_module(
    source_text: &str,
    opt_level: u8,
    overflow_mode: MirPassOverflowMode,
    bounds_mode: MirPassBoundsMode,
    target_backend: MirPassTargetBackend,
) -> MirModule {
    let checked = check(&SourceFile::new("test.ck", source_text));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR lowering should succeed");
    let pipeline = build_mir_optimization_pipeline(opt_level);
    let optimized = run_mir_pass_pipeline(
        mir,
        &pipeline,
        &MirPassContext {
            opt_level,
            overflow_mode,
            bounds_mode,
            target_backend,
            debug: Default::default(),
        },
    );
    assert_eq!(optimized.validation_errors, []);
    optimized.module
}

pub(crate) fn shared_library_path(path: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        path.with_extension("dylib")
    } else if cfg!(target_os = "windows") {
        path.with_extension("dll")
    } else {
        path.with_extension("so")
    }
}
