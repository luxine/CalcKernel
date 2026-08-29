use std::{ffi::CString, fs};

use calckernel::{
    EmitLlvmOptions, NativeContext, NativeDynamicLibrary, NativeObject, NativeOptimizationLevel,
    NativeTarget, SourceFile, check, link_native_dynamic_library, lower_native_llvm_module,
    lower_to_mir,
};

fn native_object() -> NativeObject {
    let source = SourceFile::new("library.ck", "export fn answer() -> i32 { return 42; }");
    let checked = check(&source);
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("lower library MIR");
    let context = NativeContext::new().expect("native context");
    let target = NativeTarget::host().expect("native target");
    target
        .emit_object(
            lower_native_llvm_module(&context, &target, &mir, &EmitLlvmOptions::default())
                .expect("lower module")
                .verify()
                .expect("verify module")
                .audit()
                .expect("audit module facts")
                .optimize(&target, NativeOptimizationLevel::O3)
                .expect("optimize module"),
        )
        .expect("emit object")
}

#[test]
fn libraries_embedded_lld_should_link_and_system_loader_should_call_export_under_empty_path() {
    let object = native_object();
    let library =
        link_native_dynamic_library(&object, &["answer".to_string()]).expect("in-process LLD link");
    assert!(!library.as_bytes().is_empty());
    assert_eq!(
        library.import_library().is_some(),
        cfg!(target_os = "windows")
    );

    #[cfg(unix)]
    unsafe {
        let path = std::env::temp_dir().join(format!(
            "ckc-library-test-{}{}",
            std::process::id(),
            if cfg!(target_os = "macos") {
                ".dylib"
            } else {
                ".so"
            }
        ));
        fs::write(&path, library.as_bytes()).expect("write test library");
        let path = CString::new(path.to_string_lossy().as_bytes()).expect("library path");
        let handle = dlopen(path.as_ptr(), 2);
        assert!(!handle.is_null(), "dlopen failed");
        let symbol = CString::new("answer").expect("symbol");
        let address = dlsym(handle, symbol.as_ptr());
        assert!(!address.is_null(), "dlsym failed");
        let answer: unsafe extern "C" fn() -> i32 = std::mem::transmute(address);
        assert_eq!(answer(), 42);
        assert_eq!(dlclose(handle), 0);
        let path = std::path::PathBuf::from(path.to_string_lossy().as_ref());
        let _ = fs::remove_file(path);
    }
}

#[test]
fn libraries_lld_api_should_accept_only_verified_objects_and_checked_export_names() {
    let signature: fn(
        &NativeObject,
        &[String],
    ) -> Result<NativeDynamicLibrary, calckernel::NativeError> = link_native_dynamic_library;
    let _ = signature;
    let error = link_native_dynamic_library(&native_object(), &["bad/name".to_string()])
        .expect_err("reject non-identifier export before LLD");
    assert_eq!(error.stage, calckernel::NativeStage::Link);
    assert_eq!(error.code, 1);
    assert!(error.message.contains("invalid native export symbol"));
}

#[cfg(unix)]
unsafe extern "C" {
    fn dlopen(path: *const std::ffi::c_char, mode: std::ffi::c_int) -> *mut std::ffi::c_void;
    fn dlsym(
        handle: *mut std::ffi::c_void,
        symbol: *const std::ffi::c_char,
    ) -> *mut std::ffi::c_void;
    fn dlclose(handle: *mut std::ffi::c_void) -> std::ffi::c_int;
}
