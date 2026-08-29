use std::{ffi::OsString, fs, process::Command};

use calckernel::{
    EmitLlvmOptions, KirBoundsMode, KirBuildConfig, KirConsumer, KirOptimizationLevel,
    KirOverflowMode, KirSanitizerMode, NativeContext, NativeOptimizationLevel, NativeTarget,
    SourceFile, build_kir_module, check, import_contract_facts, link_native_dynamic_library,
    lower_native_kir_module, lower_to_mir, run_kir_pass_pipeline,
};

use super::support::temp::unique_id;

fn os(value: impl AsRef<std::ffi::OsStr>) -> OsString {
    value.as_ref().to_os_string()
}

fn fixture(source: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("ckc_contract_sanitizer_{}", unique_id()));
    fs::create_dir_all(&dir).expect("fixture dir");
    let path = dir.join("input.ck");
    fs::write(&path, source).expect("fixture source");
    (dir, path)
}

fn build_and_run(source: &str, opt: u8, sanitized: bool) -> std::process::Output {
    let (dir, source) = fixture(source);
    let executable = dir.join("program");
    let mut build = Command::new(env!("CARGO_BIN_EXE_ckc"));
    build.args([
        os("build"),
        os(&source),
        os("--out"),
        os(&executable),
        os("--kind"),
        os("executable"),
        os(format!("-O{opt}")),
    ]);
    if sanitized {
        build.arg("--sanitize-contracts");
    }
    let built = build.output().expect("build executable");
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    Command::new(executable)
        .env("PATH", "")
        .output()
        .expect("run executable")
}

#[test]
fn contract_sanitizer_should_check_internal_unsafe_calls_at_every_optimization_level() {
    let source = r#"
        unsafe fn positive(n: i64) -> i32
        contract { requires n > 0; requires multiple_of(n, 2); effects none; }
        { return 7; }
        fn main() -> i32 { unsafe { return positive(-2); } }
    "#;
    for opt in 0..=3 {
        let output = build_and_run(source, opt, true);
        assert_eq!(output.status.code(), Some(246), "O{opt}: {output:?}");
        assert_eq!(output.stdout, b"");
        assert_eq!(output.stderr, b"CKR0007: unsafe contract violation\n");
    }
}

#[test]
fn contract_sanitizer_should_accept_multiple_dynamic_scalar_predicates() {
    let source = r#"
        unsafe fn kernel(n: u32) -> i32
        contract {
          requires n + 1 <= 8;
          requires multiple_of(n, 2);
          effects none;
        }
        { return 1; }
        fn main() -> i32 {
          unsafe { return kernel(6) - 1; }
        }
    "#;
    for opt in 0..=3 {
        let output = build_and_run(source, opt, true);
        assert_eq!(output.status.code(), Some(0), "O{opt}: {output:?}");
        assert_eq!(output.stderr, b"");
    }
}

#[test]
fn contract_sanitizer_should_preserve_unbounded_affine_math_at_integer_extrema() {
    let source = r#"
        unsafe fn extreme(n: u64) -> i32
        contract {
          requires 184467440737095516170 * n > 184467440737095516169;
          requires multiple_of(184467440737095516170 * n, 184467440737095516170);
          effects none;
        }
        { return 0; }
        fn main() -> i32 {
          let n: u64 = 18446744073709551615;
          unsafe { return extreme(n); }
        }
    "#;
    let output = build_and_run(source, 3, true);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stderr, b"");
}

#[test]
fn contract_sanitizer_should_recheck_every_recursive_unsafe_entry() {
    let source = r#"
        unsafe fn recurse(n: i32) -> i32
        contract { requires n > 0; effects none; }
        {
          if n > 0 { unsafe { return recurse(n - 1); } }
          return 0;
        }
        fn main() -> i32 { unsafe { return recurse(1); } }
    "#;
    for opt in 0..=3 {
        let output = build_and_run(source, opt, true);
        assert_eq!(output.status.code(), Some(246), "O{opt}: {output:?}");
        assert_eq!(output.stderr, b"CKR0007: unsafe contract violation\n");
    }
}

#[test]
fn contract_sanitizer_normal_ir_should_not_contain_contract_guards() {
    let (_, source) = fixture(
        r#"
        export unsafe fn positive(n: i64) -> i32
        contract { requires n > 0; effects none; }
        { return 7; }
        "#,
    );
    for opt in 0..=3 {
        let output = Command::new(env!("CARGO_BIN_EXE_ckc"))
            .args([os("emit-llvm"), os(&source), os(format!("-O{opt}"))])
            .output()
            .expect("emit normal LLVM");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).expect("LLVM UTF-8");
        assert!(!text.contains("contract.sanitize"), "O{opt}:\n{text}");
        assert!(!text.contains("__ck_contract_"), "O{opt}:\n{text}");
    }
}

fn sanitized_library(source_text: &str) -> Vec<u8> {
    let checked = check(&SourceFile::new("contract-library.ck", source_text));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("lower sanitizer MIR");
    let mut kir = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::NativeLibrary,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .expect("build sanitizer KIR");
    // The public builder deliberately reserves sanitizer mode for executable
    // consumers. This backend-only harness toggles the already-pruned library
    // artifact solely to invoke an exported unsafe boundary from host Rust.
    kir.config.sanitizer_mode = KirSanitizerMode::Contracts;
    let facts = import_contract_facts(&kir, &checked.checked_program, 0)
        .expect("import sanitizer contracts");
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O3, Some(&facts));
    assert!(result.errors.is_empty(), "{:#?}", result.errors);
    let context = NativeContext::new().expect("native context");
    let target = NativeTarget::host().expect("native target");
    let module = lower_native_kir_module(&context, &target, &result, &EmitLlvmOptions::default())
        .expect("lower sanitizer LLVM");
    let verified = module.verify().expect("verify sanitizer module");
    let text = verified.to_ir_string().expect("sanitizer IR");
    for marker in ["contract.sanitize", "ptrtoint", "i192"] {
        assert!(text.contains(marker), "missing {marker}:\n{text}");
    }
    let object = target
        .emit_object(
            verified
                .audit()
                .expect("audit sanitizer module")
                .optimize(&target, NativeOptimizationLevel::O3)
                .expect("optimize sanitizer module"),
        )
        .expect("emit sanitizer object");
    link_native_dynamic_library(&object, &["validate".to_string()])
        .expect("link sanitizer test library")
        .as_bytes()
        .to_vec()
}

#[repr(align(32))]
struct Aligned([i32; 8]);

#[test]
fn contract_sanitizer_should_check_noalias_alignment_zero_length_and_address_wrap() {
    let bytes = sanitized_library(
        r#"
        export unsafe fn validate(a: slice<i32>, b: slice<i32>) -> i32
        contract {
          requires noalias(a, b);
          requires aligned(a.data, 32);
          requires aligned(b.data, 32);
          effects none;
        }
        { return 0; }
        "#,
    );
    let library = DynamicLibrary::from_bytes(&bytes);
    type Validate = unsafe extern "C" fn(*mut i32, u32, *mut i32, u32, *mut i32) -> i32;
    // SAFETY: `validate` has the exact flattened sanitizer status ABI above.
    let validate: Validate = unsafe { library.symbol("validate") };
    let mut left = Aligned([0; 8]);
    let mut right = Aligned([0; 8]);
    let mut result = -1;
    // SAFETY: the positive ranges are live for the call and match the CK types.
    assert_eq!(
        unsafe { validate(left.0.as_mut_ptr(), 1, right.0.as_mut_ptr(), 1, &mut result,) },
        0
    );
    // SAFETY: equal live ranges are intentionally passed to exercise noalias.
    assert_eq!(
        unsafe { validate(left.0.as_mut_ptr(), 1, left.0.as_mut_ptr(), 1, &mut result,) },
        7
    );
    // SAFETY: zero-length slices are empty even when their data addresses match.
    assert_eq!(
        unsafe { validate(left.0.as_mut_ptr(), 0, left.0.as_mut_ptr(), 0, &mut result,) },
        0
    );
    // SAFETY: `add(1)` remains within the aligned backing array and is not
    // dereferenced by the effects-none function.
    let misaligned = unsafe { right.0.as_mut_ptr().add(1) };
    // SAFETY: both ranges are live; the second address intentionally violates
    // the declared 32-byte alignment.
    assert_eq!(
        unsafe { validate(left.0.as_mut_ptr(), 1, misaligned, 1, &mut result) },
        7
    );
    let wrapping = std::ptr::without_provenance_mut::<i32>(usize::MAX - 31);
    // SAFETY: the forged address is never dereferenced; it is converted to an
    // integer solely to prove that end-address wrap becomes status 7.
    assert_eq!(
        unsafe { validate(wrapping, 16, right.0.as_mut_ptr(), 1, &mut result) },
        7
    );
}

struct DynamicLibrary {
    handle: *mut std::ffi::c_void,
    path: std::path::PathBuf,
}

impl DynamicLibrary {
    fn from_bytes(bytes: &[u8]) -> Self {
        let suffix = if cfg!(target_os = "windows") {
            ".dll"
        } else if cfg!(target_os = "macos") {
            ".dylib"
        } else {
            ".so"
        };
        let path = std::env::temp_dir().join(format!(
            "ckc-contract-sanitizer-library-{}-{suffix}",
            unique_id()
        ));
        fs::write(&path, bytes).expect("write sanitizer library");
        let handle = platform_loader::open(&path);
        Self { handle, path }
    }

    unsafe fn symbol<T: Copy>(&self, name: &str) -> T {
        let address = platform_loader::symbol(self.handle, name);
        assert_eq!(std::mem::size_of::<T>(), std::mem::size_of_val(&address));
        // SAFETY: the caller supplies the exact C ABI function pointer type.
        unsafe { std::mem::transmute_copy(&address) }
    }
}

impl Drop for DynamicLibrary {
    fn drop(&mut self) {
        platform_loader::close(self.handle);
        fs::remove_file(&self.path).expect("remove sanitizer library");
    }
}

#[cfg(unix)]
mod platform_loader {
    use std::{ffi::CString, path::Path};

    pub(super) fn open(path: &Path) -> *mut std::ffi::c_void {
        let path = CString::new(path.to_string_lossy().as_bytes()).expect("library path");
        // SAFETY: the path is NUL-terminated and valid for this call.
        let handle = unsafe { dlopen(path.as_ptr(), 2) };
        assert!(!handle.is_null(), "dlopen failed");
        handle
    }

    pub(super) fn symbol(handle: *mut std::ffi::c_void, name: &str) -> *mut std::ffi::c_void {
        let name = CString::new(name).expect("symbol name");
        // SAFETY: the loader handle is live and the name is NUL-terminated.
        let address = unsafe { dlsym(handle, name.as_ptr()) };
        assert!(!address.is_null(), "dlsym failed");
        address
    }

    pub(super) fn close(handle: *mut std::ffi::c_void) {
        // SAFETY: DynamicLibrary owns the live handle and closes it once.
        assert_eq!(unsafe { dlclose(handle) }, 0);
    }

    unsafe extern "C" {
        fn dlopen(path: *const std::ffi::c_char, mode: std::ffi::c_int) -> *mut std::ffi::c_void;
        fn dlsym(
            handle: *mut std::ffi::c_void,
            name: *const std::ffi::c_char,
        ) -> *mut std::ffi::c_void;
        fn dlclose(handle: *mut std::ffi::c_void) -> std::ffi::c_int;
    }
}

#[cfg(windows)]
mod platform_loader {
    use std::{ffi::CString, os::windows::ffi::OsStrExt, path::Path};

    pub(super) fn open(path: &Path) -> *mut std::ffi::c_void {
        let mut wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
        wide.push(0);
        // SAFETY: the path is NUL-terminated and valid for this call.
        let handle = unsafe { LoadLibraryW(wide.as_ptr()) };
        assert!(!handle.is_null(), "LoadLibraryW failed");
        handle
    }

    pub(super) fn symbol(handle: *mut std::ffi::c_void, name: &str) -> *mut std::ffi::c_void {
        let name = CString::new(name).expect("symbol name");
        // SAFETY: the loader handle is live and the name is NUL-terminated.
        let address = unsafe { GetProcAddress(handle, name.as_ptr()) };
        assert!(!address.is_null(), "GetProcAddress failed");
        address
    }

    pub(super) fn close(handle: *mut std::ffi::c_void) {
        // SAFETY: DynamicLibrary owns the live handle and closes it once.
        assert_ne!(unsafe { FreeLibrary(handle) }, 0);
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryW(path: *const u16) -> *mut std::ffi::c_void;
        fn GetProcAddress(handle: *mut std::ffi::c_void, name: *const i8) -> *mut std::ffi::c_void;
        fn FreeLibrary(handle: *mut std::ffi::c_void) -> i32;
    }
}
