use std::{collections::BTreeSet, fs, path::Path, process::Command};

#[cfg(unix)]
use std::{os::unix::fs::PermissionsExt, path::PathBuf};

use sha2::{Digest, Sha256};

use super::support::oracle::repo_root;

fn read(path: &str) -> String {
    fs::read_to_string(repo_root().join(path))
        .unwrap_or_else(|error| panic!("failed to read {path}: {error}"))
}

#[test]
fn vector_differential_should_unload_dynamic_libraries_before_cleanup() {
    let differential = read("tests/native/differential.rs");
    let body = differential
        .split_once(
            "fn differential_vector_loop_should_match_o0_for_zero_short_exact_remainder_and_overlap_fallback() {",
        )
        .expect("vector differential function")
        .1
        .split_once("\n}\n\nfn compile_vector_library")
        .expect("vector differential function end")
        .0;
    let cleanup = body
        .rfind("fs::remove_dir_all(root)")
        .expect("vector differential cleanup");

    for unload in ["drop(o3);", "drop(o0);"] {
        let position = body
            .rfind(unload)
            .unwrap_or_else(|| panic!("missing explicit library unload {unload:?}"));
        assert!(
            position < cleanup,
            "{unload} must precede cleanup because Windows cannot delete a loaded DLL"
        );
    }
}

fn normalize_powershell_diagnostic(stderr: &str) -> String {
    let mut without_sgr = String::with_capacity(stderr.len());
    let mut remaining = stderr;
    while let Some(start) = remaining.find("\u{1b}[") {
        without_sgr.push_str(&remaining[..start]);
        let parameters = &remaining[start + 2..];
        let Some(end) = parameters.find('m') else {
            without_sgr.push_str(&remaining[start..]);
            remaining = "";
            break;
        };
        if parameters[..end]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b';' | b':'))
        {
            remaining = &parameters[end + 1..];
        } else {
            without_sgr.push('\u{1b}');
            remaining = &remaining[start + 1..];
        }
    }
    without_sgr.push_str(remaining);
    without_sgr
        .replace('|', "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn powershell_diagnostic_normalization_should_ignore_ansi_sgr() {
    let stderr = concat!(
        "native artifact audit: executable dependencies must be exactly",
        "\u{1b}[0m\n",
        "\u{1b}[31;1m| kernel32.dll\u{1b}[0m\n",
    );
    assert_eq!(
        normalize_powershell_diagnostic(stderr),
        "native artifact audit: executable dependencies must be exactly kernel32.dll"
    );
}

fn quoted_scalar(block: &str, key: &str) -> String {
    let prefix = format!("{key} = \"");
    block
        .lines()
        .find_map(|line| line.trim().strip_prefix(&prefix))
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or_else(|| panic!("missing {key:?} in provenance block:\n{block}"))
        .to_string()
}

fn quoted_array(block: &str, key: &str) -> Vec<String> {
    let prefix = format!("{key} = [");
    let raw = block
        .lines()
        .find_map(|line| line.trim().strip_prefix(&prefix))
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or_else(|| panic!("missing {key:?} in provenance block:\n{block}"));
    raw.split(',')
        .map(str::trim)
        .map(|value| {
            value
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .unwrap_or_else(|| panic!("invalid quoted value in {key}: {value}"))
                .to_string()
        })
        .collect()
}

fn sha256(path: &Path) -> String {
    format!(
        "{:x}",
        Sha256::digest(
            fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
        )
    )
}

#[test]
fn native_toolchain_cargo_profile_should_be_explicit_and_optional() {
    let cargo = read("Cargo.toml");

    assert!(
        cargo.contains("native-toolchain = []"),
        "Cargo.toml must declare an opt-in native-toolchain feature"
    );
    assert!(
        repo_root().join("build.rs").is_file(),
        "native bootstrap validation belongs in build.rs"
    );
}

#[test]
fn sanitizer_bridge_build_should_link_the_platform_runtime_explicitly() {
    let build = read("build.rs");
    let script = read("scripts/test-sanitized-ownership.sh");
    for required in [
        "configure_sanitizer_linkage(target)",
        "cargo::rustc-link-lib=asan",
        "cargo::rustc-link-lib=ubsan",
    ] {
        assert!(
            build.contains(required),
            "sanitized bridge linkage must contain {required:?}"
        );
    }
    for required in [
        "detect_leaks=1:halt_on_error=1",
        "sanitized ownership is a Linux-only gate",
        "$(uname -s)\" == Linux",
    ] {
        assert!(
            script.contains(required),
            "sanitized ownership runner must contain {required:?}"
        );
    }
}

#[test]
fn native_toolchain_manifest_should_pin_the_official_llvm_source() {
    let manifest = read("native/llvm/manifest.toml");

    for required in [
        "version = \"22.1.8\"",
        "tag = \"llvmorg-22.1.8\"",
        "commit = \"ca7933e47d3a3451d81e72ac174dcb5aa28b59d1\"",
        "archive = \"llvm-project-22.1.8.src.tar.xz\"",
        "sha256 = \"922f1817a0df7b1489272d18134ee0087a8b068828f87ac63b9861b1a9965888\"",
        "host_only = true",
        "static_only = true",
        "include_clang = false",
        "link_components = [\"core\", \"native\", \"orcjit\", \"nativecodegen\", \"lto\"]",
        "minimum_os = \"11.0\"",
        "[profiles.oracle]",
        "include_clang = true",
    ] {
        assert!(
            manifest.contains(required),
            "native LLVM manifest must contain {required:?}"
        );
    }
}

#[test]
fn native_toolchain_bootstrap_should_cover_unix_and_windows() {
    for path in ["scripts/bootstrap-llvm.sh", "scripts/bootstrap-llvm.ps1"] {
        assert!(repo_root().join(path).is_file(), "missing {path}");
    }
    let unix = read("scripts/bootstrap-llvm.sh");
    assert!(unix.contains("CMAKE_OSX_DEPLOYMENT_TARGET=11.0"));
    assert!(unix.contains("ckc_components=(core native orcjit nativecodegen lto)"));
    assert!(
        unix.contains("if [[ -n \"$ckc_jobs\" ]]; then")
            && unix.contains("cmake --build \"$ckc_build_dir/build\" --parallel \"$ckc_jobs\"")
            && !unix.contains("ckc_parallel_args"),
        "Unix bootstrap must not expand an empty array under macOS Bash 3.2 set -u"
    );
    assert!(
        unix.contains("ckc_cmake_args=(")
            && unix.contains("ckc_cmake_args+=(")
            && unix.contains("cmake \"${ckc_cmake_args[@]}\"")
            && !unix.contains("ckc_runtime_args"),
        "Unix bootstrap optional CMake flags must extend one non-empty array under macOS Bash 3.2 set -u"
    );
    assert!(
        unix.contains("ckc_static_libs=(\"${ckc_lld_libs[@]}\" LLVMDTLTO \"${ckc_llvm_libs[@]}\")"),
        "Unix bootstrap must add LLVM 22 DTLTO after LLD and before its LLVM dependencies"
    );
    assert!(!unix.contains("--libnames all"));
    for required in [
        "native/runtime/common/runtime.c",
        "native/runtime/common/format_int.c",
        "native/runtime/common/format_float.c",
        "native/runtime/vendor/ryu/d2s.c",
        "native/runtime/linux/syscalls.S",
        "runtime_sha256",
    ] {
        assert!(unix.contains(required), "Unix bootstrap missing {required}");
    }

    let windows = read("scripts/bootstrap-llvm.ps1");
    assert!(windows.contains("core\", \"native\", \"orcjit\", \"nativecodegen\", \"lto"));
    for required in [
        "vswhere.exe",
        "VsDevCmd.bat",
        "Import-MsvcEnvironment",
        "$msvcHostArch = \"amd64\"",
        "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
        "Microsoft.VisualStudio.Component.VC.Tools.ARM64",
        "VSCMD_ARG_TGT_ARCH",
        "CKC_MSVC_TARGET",
        "Get-Command cl.exe",
        "Get-Command link.exe",
        "CMAKE_C_COMPILER_ID",
        "CMAKE_CXX_COMPILER_ID",
    ] {
        assert!(
            windows.contains(required),
            "Windows bootstrap must import and validate the target MSVC environment with {required}"
        );
    }
    for required in ["-DCMAKE_C_COMPILER=cl.exe", "-DCMAKE_CXX_COMPILER=cl.exe"] {
        assert!(
            windows.contains(required),
            "Windows bootstrap must bind CMake to the MSVC compiler with {required}"
        );
    }
    assert!(
        windows.contains("New-Item -ItemType Directory -Path $manifestDir -Force"),
        "Windows bootstrap must tolerate the runtime step having already created share/ckc"
    );
    assert!(
        windows.contains(
            "$staticLibraries = @(\"lldCOFF\", \"lldCommon\", \"LLVMDTLTO\") + $llvmLibraries"
        ),
        "Windows bootstrap must add LLVM 22 DTLTO after LLD and before its LLVM dependencies"
    );
    assert!(!windows.contains("--libnames all"));
    for required in [
        "native/runtime/windows/process.c",
        "native/runtime/platform/kernel32.def",
        "runtime_platform_import_sha256",
        "llvm-lib.exe",
    ] {
        assert!(
            windows.contains(required),
            "Windows bootstrap missing {required}"
        );
    }
}

#[test]
fn release_toolchain_should_static_link_non_system_cpp_runtimes() {
    let unix = read("scripts/bootstrap-llvm.sh");
    assert!(
        unix.contains("LLVM_STATIC_LINK_CXX_STDLIB=ON"),
        "Linux LLVM bootstrap must request a static C++ standard library"
    );

    let windows = read("scripts/bootstrap-llvm.ps1");
    assert!(
        windows.contains("CMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded")
            && !windows.contains("LLVM_USE_CRT_RELEASE"),
        "Windows LLVM bootstrap must use the static release CRT"
    );

    let build = read("build.rs");
    for required in [
        "cpp_link_stdlib(None)",
        "static_crt(true)",
        "fn link_static_linux_cpp_runtime",
        "-print-file-name=libstdc++.a",
        "cargo::rustc-link-search=native=",
        "cargo::rustc-link-lib=static=stdc++",
        "cargo::rustc-link-lib=c++",
    ] {
        assert!(
            build.contains(required),
            "native bridge build must contain {required:?}"
        );
    }
}

#[test]
fn windows_static_crt_policy_should_cover_bootstrap_cache_and_cargo() {
    let bootstrap = read("scripts/bootstrap-llvm.ps1");
    assert!(bootstrap.contains("CMAKE_EXPORT_COMPILE_COMMANDS=ON"));
    assert!(
        bootstrap
            .find("Assert-MsvcCompileCommands")
            .expect("check actual flags")
            < bootstrap.find("& cmake @build").unwrap()
    );
    for path in [
        "scripts/bootstrap-llvm.ps1",
        "scripts/validate-llvm-prefix.ps1",
    ] {
        let text = read(path);
        for required in [
            "validate-msvc-crt.ps1",
            "Assert-MsvcStaticArchives",
            "msvc_runtime_library",
        ] {
            assert!(text.contains(required), "{path} must enforce {required}");
        }
    }
    assert!(
        bootstrap.contains("$linkComponents = $components + @(\"libdriver\", \"windowsmanifest\")")
    );
    assert!(bootstrap.contains("--libnames @linkComponents"));
    assert!(bootstrap.contains("--system-libs @linkComponents"));
    let build = read("build.rs");
    for required in [
        "CARGO_CFG_TARGET_FEATURE",
        "crt-static",
        "msvc_runtime_library",
        "MultiThreaded",
        "LLVMLibDriver",
        "LLVMWindowsManifest",
    ] {
        assert!(
            build.contains(required),
            "Native build must enforce {required}"
        );
    }
    let config = read(".cargo/config.toml");
    for target in ["x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc"] {
        let block = config
            .split_once(&format!("[target.{target}]"))
            .expect("MSVC target config")
            .1
            .split('[')
            .next()
            .unwrap();
        assert!(
            block.contains("rustflags = "),
            "{target} needs static Rust flags"
        );
    }
    assert_eq!(config.matches("target-feature=+crt-static").count(), 2);
    let action = read(".github/actions/bootstrap-ckc-llvm/action.yml");
    let digest = action
        .lines()
        .find(|line| line.contains("hashFiles("))
        .unwrap();
    for path in [
        "scripts/validate-msvc-crt.ps1",
        "scripts/validate-llvm-prefix.ps1",
    ] {
        assert!(digest.contains(path), "cache identity must include {path}");
    }
}

#[test]
fn windows_cache_hit_should_open_no_follow_entry_for_attribute_touch() {
    let store = read("src/cli/cache/store.rs");
    let windows_open = store
        .split_once("#[cfg(target_os = \"windows\")]\nfn open_read_nofollow")
        .map(|(_, suffix)| suffix)
        .and_then(|suffix| {
            suffix
                .split_once("#[cfg(all(not(unix)")
                .map(|(block, _)| block)
        })
        .expect("Windows cache entry opener");

    for required in [
        "const GENERIC_READ: u32 = 0x8000_0000;",
        "const FILE_WRITE_ATTRIBUTES: u32 = 0x0000_0100;",
        ".access_mode(GENERIC_READ | FILE_WRITE_ATTRIBUTES)",
        "const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;",
        ".custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)",
    ] {
        assert!(
            windows_open.contains(required),
            "Windows cache hit must retain no-follow read access and allow only mtime attribute writes with {required:?}"
        );
    }
    assert!(
        !windows_open.contains(".write(true)"),
        "cache hit must not request generic write access to immutable entry bytes"
    );
}

#[test]
fn windows_freestanding_runtime_should_close_optimizer_generated_memory_helpers() {
    let platform = read("native/runtime/windows/process.c");
    const FUNCTION_PRAGMA: &str = "#pragma function(memcpy, memset)";
    assert_eq!(platform.matches(FUNCTION_PRAGMA).count(), 1);
    assert_eq!(platform.matches("#pragma optimize(\"\", off)").count(), 1);
    assert_eq!(platform.matches("#pragma optimize(\"\", on)").count(), 1);
    assert!(
        platform.contains(
            "#if defined(_MSC_VER)\n#pragma function(memcpy, memset)\n#pragma optimize(\"\", off)"
        ),
        "MSVC must force calls for memcpy/memset before defining them under the local optimization boundary"
    );
    let helpers = platform
        .split_once("#pragma optimize(\"\", off)")
        .expect("MSVC memory helper optimization boundary")
        .1
        .split_once("#pragma optimize(\"\", on)")
        .expect("MSVC memory helper optimization restore")
        .0;
    for required in [
        "void *memcpy(void *destination, const void *source, size_t length)",
        "const unsigned char *input = (const unsigned char *)source;",
        "void *memset(void *destination, int value, size_t length)",
        "*output++ = (unsigned char)value;",
    ] {
        assert!(
            helpers.contains(required),
            "Windows freestanding runtime must define optimizer memory helper contract {required:?}"
        );
    }
    assert_eq!(
        helpers.matches("while (length != 0u)").count(),
        2,
        "both helpers must use a zero-length-safe byte loop"
    );
    assert_eq!(
        helpers.matches("return destination;").count(),
        2,
        "both helpers must return the original destination"
    );
    for forbidden in ["#include <string.h>", "memmove(", "malloc("] {
        assert!(
            !platform.contains(forbidden),
            "Windows platform object must remain freestanding: {forbidden}"
        );
    }

    let bootstrap = read("scripts/bootstrap-llvm.ps1");
    let sources = bootstrap
        .split_once("$runtimeSources = @(")
        .expect("Windows runtime source manifest")
        .1
        .split_once("\n)\n$runtimeObjects = @()")
        .expect("Windows runtime source manifest end")
        .0;
    assert_eq!(
        sources.matches("    @(\"").count(),
        5,
        "freestanding memory helpers must stay inside the five-object runtime closure"
    );
    assert!(sources.contains(
        "@(\"platform.obj\", (Join-Path $repoRoot \"native/runtime/windows/process.c\"))"
    ));
    assert!(bootstrap.contains("cl.exe /nologo /c /TC /O2 /W3 /WX /GS- /Zl"));
}

#[test]
fn windows_floating_objects_should_own_one_coalescible_fltused_definition() {
    let bridge = read("native/bridge/ckc_llvm.cpp");
    for required in [
        "isWindowsMSVCEnvironment()",
        "llvm::GlobalValue::WeakODRLinkage",
        "getOrInsertComdat(\"_fltused\")",
        "llvm::Comdat::Any",
        "setAlignment(llvm::Align(4))",
    ] {
        assert!(
            bridge.contains(required),
            "MSVC object lowering must retain the coalescible compiler helper contract {required:?}"
        );
    }

    let format_float = read("native/runtime/common/format_float.c");
    assert!(
        format_float.contains("__declspec(selectany) int _fltused = 0;"),
        "the embedded runtime copy must coalesce with the generated object definition"
    );
    let provenance = read("native/runtime/provenance.toml");
    assert!(provenance.contains("compiler_helpers = [\"_fltused\"]"));
}

#[test]
fn windows_native_execution_should_separate_coff_jit_support_from_artifact_runtime() {
    let bootstrap = read("scripts/bootstrap-llvm.ps1");
    for required in [
        "native/runtime/windows/jit_image_base.c",
        "runtime_jit_support",
        "runtime_jit_support_sha256",
        "x86_64-pc-windows-msvc",
    ] {
        assert!(
            bootstrap.contains(required),
            "Windows bootstrap must bind x64 JIT image-base support with {required:?}"
        );
    }

    let verifier = read("scripts/validate-llvm-prefix.ps1");
    for required in [
        "runtime_jit_support",
        "runtime_jit_support_sha256",
        "jit_image_base.obj",
    ] {
        assert!(
            verifier.contains(required),
            "cache verification must bind JIT support with {required:?}"
        );
    }

    let build = read("build.rs");
    for required in [
        "runtime_jit_support",
        "runtime_jit_support_sha256",
        "CKC_RUNTIME_JIT_SUPPORT",
    ] {
        assert!(
            build.contains(required),
            "native build must verify JIT support with {required:?}"
        );
    }

    let runtime = read("src/backend/native_runtime.rs");
    assert!(runtime.contains("embedded_jit_objects"));
    assert!(runtime.contains("CKC_RUNTIME_JIT_SUPPORT"));
    assert!(
        runtime.lines().any(|line| {
            line.trim() == "let mut objects: Vec<&'static [u8]> = Vec::with_capacity(7);"
        }),
        "the Windows x64 JIT anchor and dispatch runtime must enter an explicitly slice-typed object collection"
    );
    assert!(
        runtime.contains("embedded_runtime_objects"),
        "the five artifact runtime objects remain a separate closed set"
    );
    assert!(
        runtime.contains("objects.push(embedded_dispatch_runtime_object())"),
        "the JIT graph must define the dispatch stack-capture symbol referenced by the Linux entry object"
    );

    let bridge = read("native/bridge/ckc_llvm.cpp");
    assert!(
        bridge
            .matches("arguments.emplace_back(\"/out:\" + *output_path)")
            .count()
            >= 2,
        "both COFF LLD entry points must use /out:<path>"
    );
    assert!(
        bridge.contains("runtime_object_count != 7"),
        "COFF x64 JIT must fail closed unless it receives anchor + five runtime objects + dispatch runtime"
    );
    assert!(
        bridge.contains("runtime_object_count != 6"),
        "other JIT targets must fail closed unless they receive five runtime objects + dispatch runtime"
    );
    let execute = bridge
        .split_once("extern \"C\" int32_t ckc_llvm_jit_execute(")
        .expect("native JIT execution bridge")
        .1;
    let materialization = execute
        .split_once("buffers.push_back(std::move(*program));")
        .expect("fully validated JIT object collection")
        .1
        .split_once("llvm::orc::ExecutorAddr entry_address;")
        .expect("JIT object materialization boundary")
        .0;
    let materialization = materialization
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    let coff_guard = materialization
        .find("defined(CKC_LLD_COFF)")
        .expect("eager anchor materialization must remain COFF-only");
    let msvc_x64_guard = materialization
        .find("defined(_M_X64)")
        .expect("eager anchor materialization must remain MSVC-x64-only");
    let clang_x64_guard = materialization
        .find("defined(__x86_64__)")
        .expect("eager anchor materialization must remain Clang-x64-only");
    let anchor_add = materialization
        .find("jit->value->addObjectFile(std::move(buffers.front()))")
        .expect("COFF x64 must add its image-base anchor first");
    let anchor_lookup = materialization
        .find("jit->value->lookupLinkerMangled(\"__ImageBase\")")
        .expect("COFF x64 must eagerly materialize its image-base anchor");
    let remaining_loop = materialization
        .find("for(size_tindex=1;index<buffers.size();++index)")
        .expect("COFF x64 must add the remaining objects after its anchor");
    assert!(
        coff_guard < anchor_add
            && msvc_x64_guard < anchor_add
            && clang_x64_guard < anchor_add
            && anchor_add < anchor_lookup
            && anchor_lookup < remaining_loop,
        "COFF x64 guard, anchor add, eager lookup, and remaining-object add must stay ordered"
    );
    assert!(
        materialization.contains(
            "if(!image_base_address){returnset_llvm_error(error,image_base_address.takeError());}"
        ),
        "COFF x64 image-base materialization failures must fail closed"
    );
    assert!(
        materialization.contains("#elsefor(auto&buffer:buffers)")
            && materialization.contains("#endif"),
        "non-COFF-x64 targets must retain the generic object-add loop"
    );

    let anchor = read("native/runtime/windows/jit_image_base.c");
    assert!(anchor.contains("__ImageBase"));
    for forbidden in ["main(", "malloc(", "printf(", "GetModuleHandle"] {
        assert!(
            !anchor.contains(forbidden),
            "JIT image-base support must not grow a runtime surface: {forbidden}"
        );
    }
}

#[test]
fn coff_x64_jit_should_route_allowed_process_calls_through_graph_local_stubs() {
    let bridge = read("native/bridge/ckc_llvm.cpp");
    for required in [
        "CkcCoffX64ProcessStubsPlugin",
        "add_coff_x64_process_stubs",
        "Config.PostPrunePasses.push_back",
        "G.getEdgeKindName(edge.getKind()) !=",
        "llvm::StringRef(\"PCRel32\")",
        "GetStdHandle",
        "WriteFile",
        "ExitProcess",
        "llvm::jitlink::x86_64::createAnonymousPointer",
        "llvm::jitlink::x86_64::createAnonymousPointerJumpStub",
        "llvm::jitlink::x86_64::Pointer64",
        "call opcode",
        "object_layer->addPlugin",
    ] {
        assert!(
            bridge.contains(required),
            "COFF x64 process-call range extension must retain {required:?}"
        );
    }
    assert_eq!(
        bridge.matches("CkcCoffX64ProcessStubsPlugin").count(),
        2,
        "the COFF x64 plugin must have one declaration and one installation"
    );
    assert!(
        bridge.contains("defined(CKC_LLD_COFF) &&")
            && bridge.contains("defined(_M_X64) || defined(__x86_64__)"),
        "process-call stubs must remain compile-time COFF x64 only"
    );
    assert!(
        bridge.contains("setLinkProcessSymbolsByDefault(false)"),
        "graph-local stubs must not reopen arbitrary process symbol lookup"
    );
}

#[test]
fn coff_arm64_rtdyld_should_preserve_official_orc_symbol_responsibility_contract() {
    let bridge = read("native/bridge/ckc_llvm.cpp");
    let arm64_coff_creator = bridge
        .split_once("if (use_coff_aarch64_rtdyld) {")
        .expect("COFF ARM64 RuntimeDyld branch")
        .1
        .split_once("        } else {")
        .expect("COFF ARM64 RuntimeDyld branch end")
        .0;
    let code_only = arm64_coff_creator
        .lines()
        .map(|line| line.split_once("//").map_or(line, |(code, _)| code))
        .collect::<String>();
    let compact = code_only
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();

    for required in [
        "CkcAuditedSectionMemoryManager",
        "autoobject_layer=",
        "std::make_unique<llvm::orc::RTDyldObjectLinkingLayer>",
        "object_layer->setOverrideObjectFlagsWithResponsibilityFlags(true);",
        "object_layer->setAutoClaimResponsibilityForObjectSymbols(true);",
        "std::unique_ptr<llvm::orc::ObjectLayer>(std::move(object_layer))",
    ] {
        assert!(
            compact.contains(required),
            "COFF ARM64 audited RuntimeDyld creator must preserve {required:?}"
        );
    }

    let typed_layer = compact
        .find("autoobject_layer=")
        .expect("typed RuntimeDyld layer");
    let override_flags = compact
        .find("object_layer->setOverrideObjectFlagsWithResponsibilityFlags(true);")
        .expect("COFF responsibility flag override");
    let auto_claim = compact
        .find("object_layer->setAutoClaimResponsibilityForObjectSymbols(true);")
        .expect("COFF object-symbol auto-claim");
    let returned_layer = compact
        .find("std::unique_ptr<llvm::orc::ObjectLayer>(std::move(object_layer))")
        .expect("configured RuntimeDyld layer return");
    assert!(
        typed_layer < override_flags && override_flags < auto_claim && auto_claim < returned_layer,
        "the two COFF responsibility settings must configure the audited layer before it is returned"
    );
}

#[test]
fn windows_static_crt_should_validate_actual_compile_commands() {
    let root = super::support::temp::temp_dir("ckc-crt-commands");
    fs::create_dir_all(&root).unwrap();
    let database = root.join("compile_commands.json");
    let script = repo_root().join("scripts/validate-msvc-crt.ps1");
    let run = || {
        Command::new("pwsh")
        .args(["-NoLogo", "-NoProfile", "-Command", "$ErrorActionPreference = 'Stop'; . $env:CKC_TEST_CRT_SCRIPT; Assert-MsvcCompileCommands -Path $env:CKC_TEST_COMMANDS"])
        .env("CKC_TEST_CRT_SCRIPT", &script)
        .env("CKC_TEST_COMMANDS", &database)
        .output().expect("run actual CMake flag guard")
    };
    for valid in [
        r#"[{"file":"unit.cpp","command":"cl.exe /MT /c unit.cpp"},{"file":"runtime.c","arguments":["cl.exe","/MT","/c","runtime.c"]}]"#,
        r#"[{"file":"unit.cpp","command":"cl.exe \"/MT\" /c unit.cpp"},{"file":"asm.S","command":"assembler asm.S"}]"#,
    ] {
        fs::write(&database, valid).unwrap();
        let output = run();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    for invalid in [
        r#"[{"file":"unit.cpp","command":"cl.exe /MD /c unit.cpp"}]"#,
        r#"[{"file":"unit.cpp","command":"cl.exe /MTd /c unit.cpp"}]"#,
        r#"[{"file":"unit.cpp","command":"cl.exe /MT /MD /c unit.cpp"}]"#,
        r#"[{"file":"unit.cpp","command":"cl.exe /c unit.cpp /Ipath/MT"}]"#,
        r#"[{"file":"unit.cpp","arguments":["cl.exe","/MDd","/c","unit.cpp"]}]"#,
        r#"[{"file":"unit.cpp","command":"cl.exe /MT /c unit.cpp"},{"file":"other.c","command":"cl.exe /MD /c other.c"}]"#,
        "[]",
    ] {
        fs::write(&database, invalid).unwrap();
        let output = run();
        assert!(!output.status.success(), "must reject {invalid}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("MSVC compile"));
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn windows_static_prefix_should_explicitly_disable_the_separate_c_api_dll() {
    assert!(
        read("native/llvm/manifest.toml").contains("build_llvm_c_dylib = false"),
        "the pinned static build manifest must also freeze the C API DLL option"
    );
    let bootstrap = read("scripts/bootstrap-llvm.ps1");
    let configure = bootstrap
        .split_once("$configure = @(")
        .unwrap()
        .1
        .split_once("& cmake @configure")
        .unwrap()
        .0;
    assert!(
        configure.contains("\"-DLLVM_BUILD_LLVM_C_DYLIB=OFF\""),
        "MSVC defaults LLVM_BUILD_LLVM_C_DYLIB to ON independently of LLVM_BUILD_LLVM_DYLIB"
    );
}

#[test]
fn windows_static_prefix_should_reject_dlls_in_both_install_directories() {
    let bootstrap = read("scripts/bootstrap-llvm.ps1");
    // Execute the real post-install guard without requiring MSVC on the test host.
    let marker = "if ($installedVersion -ne $llvmVersion) { throw \"installed llvm-config version mismatch\" }";
    let guard = bootstrap
        .split_once(marker)
        .unwrap()
        .1
        .split_once("$clang = Join-Path $Prefix \"bin/clang.exe\"")
        .unwrap()
        .0;
    let root = super::support::temp::temp_dir("ckc-bootstrap-static-layout");
    for directory in ["bin", "lib"] {
        fs::create_dir_all(root.join(directory)).unwrap();
    }
    let script =
        format!("$ErrorActionPreference = 'Stop'\n$Prefix = $env:CKC_TEST_PREFIX\n{guard}");
    let run = || {
        Command::new("pwsh")
            .args(["-NoLogo", "-NoProfile", "-Command", &script])
            .env("CKC_TEST_PREFIX", &root)
            .output()
            .expect("execute actual installation guard")
    };
    assert!(
        run().status.success(),
        "a DLL-free installation should pass the static guard"
    );
    for directory in ["bin", "lib"] {
        let dll = root.join(directory).join("LLVM-C.dll");
        fs::write(&dll, b"synthetic DLL marker, never loaded").unwrap();
        let output = run();
        assert!(
            !output.status.success(),
            "actual post-install guard must reject {directory}/LLVM-C.dll"
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("shared LLVM library"));
        fs::remove_file(dll).unwrap();
        assert!(run().status.success());
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_runtime_should_be_source_owned_hashed_and_auditable() {
    for path in [
        "native/runtime/include/ckc_runtime.h",
        "native/runtime/common/runtime.c",
        "native/runtime/common/format_int.c",
        "native/runtime/common/format_float.c",
        "native/runtime/darwin/process.c",
        "native/runtime/linux/syscalls.S",
        "native/runtime/windows/process.c",
        "native/runtime/platform/libSystem.tbd",
        "native/runtime/platform/kernel32.def",
        "native/runtime/provenance.toml",
        "native/runtime/vendor/ryu/d2s.c",
        "native/runtime/vendor/ryu/LICENSE-Apache2",
        "native/runtime/vendor/ryu/LICENSE-Boost",
        "scripts/audit-native-artifact.sh",
        "scripts/audit-native-artifact.ps1",
    ] {
        assert!(repo_root().join(path).is_file(), "missing {path}");
    }

    let build = read("build.rs");
    for required in [
        "runtime_objects",
        "runtime_sha256",
        "CKC_RUNTIME_OBJECT_",
        "runtime_platform_import_sha256",
        "CKC_RUNTIME_PLATFORM_IMPORT",
    ] {
        assert!(build.contains(required), "build.rs missing {required}");
    }

    let unix_audit = read("scripts/audit-native-artifact.sh");
    for line in unix_audit.lines().filter(|line| line.contains('|')) {
        assert!(
            !line.contains("grep -q") && !line.contains("grep -Eiq"),
            "pipefail audit must not use early-exit grep in a pipeline: {line}"
        );
    }

    let runtime = read("native/runtime/common/runtime.c");
    for code in 1..=6 {
        assert!(runtime.contains(&format!("CKR000{code}:")));
    }
    let combined = [
        runtime,
        read("native/runtime/common/format_int.c"),
        read("native/runtime/common/format_float.c"),
    ]
    .join("\n");
    for forbidden in [
        "malloc(",
        "calloc(",
        "realloc(",
        "free(",
        "printf(",
        "snprintf(",
        "setlocale(",
    ] {
        assert!(
            !combined.contains(forbidden),
            "native runtime must not use {forbidden}"
        );
    }
}

#[test]
fn linux_profile_runtime_hex_should_avoid_mixed_signedness_under_gcc_werror() {
    let linux = read("native/profile_runtime/platform/linux.c");
    let hex = linux
        .split_once("static char ck_profile_hex(uint8_t nibble) {")
        .expect("Linux profile hex helper")
        .1
        .split_once("\n}")
        .expect("Linux profile hex helper end")
        .0;

    for required in [
        "if (nibble < 10u)",
        "return (char)('0' + (int)nibble);",
        "return (char)('a' + (int)nibble - 10);",
    ] {
        assert!(
            hex.contains(required),
            "Linux GCC -Werror build requires warning-clean hex conversion {required:?}"
        );
    }
    assert!(
        !hex.contains('?'),
        "mixed signed/unsigned conditional arms regress GCC -Werror on AArch64 Linux"
    );
}

#[test]
fn darwin_profile_runtime_imports_should_cover_x86_64_inode64_fstat() {
    let darwin = read("native/profile_runtime/platform/darwin.c");
    let libsystem = read("native/runtime/platform/libSystem.tbd");

    assert!(
        darwin.contains("fstat(directory_fd, &metadata)"),
        "profile publication must validate the opened Darwin directory"
    );
    assert!(
        libsystem.contains("_fstat$INODE64"),
        "the freestanding Darwin import surface must resolve Clang's x86_64 inode64 spelling"
    );
}

#[test]
fn dispatch_runtime_should_have_independent_provenance_bootstrap_and_private_abi() {
    for path in [
        "native/dispatch_runtime/include/ckc_dispatch_runtime.h",
        "native/dispatch_runtime/dispatch_runtime.c",
        "native/dispatch_runtime/provenance.toml",
    ] {
        assert!(repo_root().join(path).is_file(), "missing {path}");
    }
    let provenance = read("native/dispatch_runtime/provenance.toml");
    assert!(provenance.contains("dispatch_runtime_schema = 1"));
    assert!(provenance.contains("compiler_private = true"));
    assert!(provenance.contains("failure_policy = \"baseline\""));

    let build = read("build.rs");
    for required in [
        "CKC_DISPATCH_RUNTIME_OBJECT",
        "CKC_DISPATCH_RUNTIME_SHA256",
        "dispatch_runtime_object",
        "compile_intermediates",
    ] {
        assert!(build.contains(required), "build.rs missing {required}");
    }
    for bootstrap in ["scripts/bootstrap-llvm.sh", "scripts/bootstrap-llvm.ps1"] {
        let text = read(bootstrap);
        for required in [
            "dispatch_runtime_schema",
            "dispatch_runtime_object",
            "dispatch_runtime_sha256",
        ] {
            assert!(text.contains(required), "{bootstrap} missing {required}");
        }
    }

    let runtime = read("native/dispatch_runtime/dispatch_runtime.c");
    for required in [
        "__ck_dispatch_detect_capabilities",
        "__ck_dispatch_select_ranked",
        "ck_dispatch_compare_exchange",
        "ldaxr",
        "stlxr",
        "CK_DISPATCH_BASELINE",
    ] {
        assert!(
            runtime.contains(required),
            "dispatch runtime missing {required}"
        );
    }
    assert!(
        !runtime.contains("__atomic_"),
        "the freestanding dispatch runtime must not import compiler atomic helpers on baseline AArch64"
    );
    for forbidden in ["getenv(", "malloc(", "free(", "printf(", "getauxval("] {
        assert!(
            !runtime.contains(forbidden),
            "dispatch runtime must not use {forbidden}"
        );
    }
}

#[test]
fn profile_runtime_atomics_should_be_freestanding_on_msvc_and_aarch64_linux() {
    let collector = read("native/profile_runtime/common/collector.c");
    let atomics = read("native/profile_runtime/include/ckc_profile_atomic.h");
    let provenance = read("native/profile_runtime/provenance.toml");
    let windows_bootstrap = read("scripts/bootstrap-llvm.ps1");

    assert!(collector.contains("ckc_profile_atomic_u64"));
    assert!(!collector.contains("#include <stdatomic.h>"));
    for required in [
        "_InterlockedCompareExchange64",
        "defined(__aarch64__) && defined(__linux__)",
        "ldxr",
        "stxr",
        "ATOMIC_LLONG_LOCK_FREE",
    ] {
        assert!(
            atomics.contains(required),
            "atomic portability layer missing {required}"
        );
    }
    assert!(
        provenance.contains("include/ckc_profile_atomic.h"),
        "profile runtime provenance must bind the atomic portability layer"
    );
    let profile_compile = windows_bootstrap
        .split_once("$profileRuntimeObject =")
        .expect("Windows profile runtime compile section")
        .1
        .split_once("$profileRuntimeHash =")
        .expect("Windows profile runtime compile boundary")
        .0;
    assert!(
        !profile_compile.contains("/std:c11"),
        "MSVC's C11 atomic header is unavailable for the freestanding profile runtime"
    );
}

#[test]
fn native_cache_schema4_contract_should_bind_complete_bundle_and_atomic_outputs() {
    let key = read("src/cli/cache/key.rs");
    let entry = read("src/cli/cache/entry.rs");
    let cache = read("src/cli/cache/mod.rs");
    let output = read("src/cli/output.rs");
    for required in [
        "const KEY_SCHEMA: u32 = 4",
        "profile_identity",
        "artifact_identity",
        "pgo_identity",
        "multiversion_identity",
        "dispatch_identity",
        "budget_identity",
    ] {
        assert!(key.contains(required), "cache key missing {required}");
    }
    for required in [
        "CKCOBJ03",
        "const MANIFEST_SCHEMA: u32 = 4",
        "CKCBND01",
        "dispatch_runtime_digest",
        "cache bundle variant order is invalid",
        "cache bundle has trailing data",
    ] {
        assert!(entry.contains(required), "cache entry missing {required}");
    }
    for required in [
        "load_multiversion_bundle",
        "store_multiversion_bundle",
        "object_manifest != expected_manifest",
        "Sha256::digest(&object)",
        "from_cached_objects",
    ] {
        assert!(cache.contains(required), "bundle cache missing {required}");
    }
    for required in [
        "canonical_output_identity",
        "existing_files_alias",
        "output destination identity changed before commit",
        "output transaction rolled back",
    ] {
        assert!(
            output.contains(required),
            "output transaction missing {required}"
        );
    }
}

#[cfg(unix)]
#[test]
fn windows_native_artifact_audit_should_use_only_the_pinned_coff_inspector() {
    let temp = super::support::temp::temp_dir("Rust_CalcKernel-native-artifact-audit");
    let root = temp.join("Rust_CalcKernel-artifacts");
    let runtime = root.join("runtime");
    let prefix_bin = temp.join("prefix/bin");
    fs::create_dir_all(&runtime).expect("create fake runtime artifact directory");
    fs::create_dir_all(&prefix_bin).expect("create fake pinned inspector directory");
    for relative in [
        "module.obj",
        "module-static.lib",
        "module.dll",
        "module-import.lib",
        "program.exe",
        "runtime/runtime.obj",
        "runtime/format_int.obj",
        "runtime/format_float.obj",
        "runtime/ryu.obj",
        "runtime/platform.obj",
        "runtime/kernel32.lib",
    ] {
        fs::write(root.join(relative), []).expect("write empty artifact fixture");
    }
    let empty_sha = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    let sums = [
        "runtime.obj",
        "format_int.obj",
        "format_float.obj",
        "ryu.obj",
        "platform.obj",
        "kernel32.lib",
    ]
    .into_iter()
    .map(|name| format!("{empty_sha}  {name}\n"))
    .collect::<String>();
    fs::write(runtime.join("SHA256SUMS"), sums).expect("write runtime checksums");

    let inspector = prefix_bin.join("llvm-readobj.exe");
    fs::write(
        &inspector,
        r#"#!/bin/sh
mode="${CKC_TEST_ARTIFACT_MODE:-allowed}"
file=$(basename "$2")
if [ "$mode" = nonzero ]; then exit 71; fi
case "$1:$file:$mode" in
  --coff-imports:program.exe:wrong-dependency)
    printf 'Import {\n  Name: USER32.dll\n}\n'
    ;;
  --coff-imports:program.exe:*)
    printf 'File: %s\nMetadata {\n  Name: VCRUNTIME140.dll\n}\nImport {\n  Name: KERNEL32.dll\n}\n' "$2"
    ;;
  --coff-imports:module.dll:module-import)
    printf 'Import {\n  Name: KERNEL32.dll\n}\n'
    ;;
  --coff-imports:module.dll:*)
    printf 'File: %s\nFormat: COFF-x86-64\n' "$2"
    ;;
  --coff-exports:module.dll:missing-export)
    printf 'Export {\n  Ordinal: 1\n  Name: other\n}\n'
    ;;
  --coff-exports:module.dll:forbidden-export)
    printf 'Export {\n  Ordinal: 1\n  Name: answer\n}\nExport {\n  Ordinal: 2\n  Name: __ck_hidden\n}\n'
    ;;
  --coff-exports:module.dll:*)
    printf 'File: %s\nMetadata {\n  Name: CalcKernelProbe\n}\nExport {\n  Ordinal: 1\n  Name: answer\n}\n' "$2"
    ;;
  --symbols:runtime.obj:forbidden-symbol)
    printf 'Symbols [\n  Symbol {\n    Name: malloc\n    Section: IMAGE_SYM_UNDEFINED (0)\n  }\n]\n'
    ;;
  --symbols:runtime.obj:empty-symbols)
    printf 'File: C:/free/runtime.obj\nFormat: COFF-x86-64\nSymbols [\n]\n'
    ;;
  --symbols:runtime.obj:missing-symbol-container)
    printf 'File: C:/free/runtime.obj\nFormat: COFF-x86-64\n'
    ;;
  --symbols:runtime.obj:unclosed-symbol-container)
    printf 'Symbols [\n  Symbol {\n    Name: __ck_clean\n  }\n'
    ;;
  --symbols:runtime.obj:missing-symbol-name)
    printf 'Symbols [\n  Symbol {\n    Section: .text (1)\n  }\n]\n'
    ;;
  --symbols:runtime.obj:duplicate-symbol-name)
    printf 'Symbols [\n  Symbol {\n    Name: __ck_clean\n    Name: __ck_other\n  }\n]\n'
    ;;
  --symbols:*.obj:*)
    printf 'File: C:/free/runtime.obj\nSymbols [\n  Symbol {\n    Name: __ck_clean\n    Section: .text (1)\n    AuxSymbolCount: 1\n    AuxSectionDef {\n      Name: free\n    }\n  }\n]\n'
    ;;
  *) exit 72 ;;
esac
"#,
    )
    .expect("write fake llvm-readobj");
    let mut permissions = fs::metadata(&inspector)
        .expect("stat fake llvm-readobj")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&inspector, permissions).expect("make fake inspector executable");

    let run = |mode: &str| {
        Command::new("pwsh")
            .args(["-NoLogo", "-NoProfile", "-File"])
            .arg(repo_root().join("scripts/audit-native-artifact.ps1"))
            .arg("-Path")
            .arg(&root)
            .env("CKC_LLVM_PREFIX", temp.join("prefix"))
            .env("CKC_TEST_ARTIFACT_MODE", mode)
            .output()
            .expect("run Windows native artifact audit")
    };
    let allowed = run("allowed");
    assert!(
        allowed.status.success(),
        "pinned inspector rejected valid artifact fixtures:\n{}",
        String::from_utf8_lossy(&allowed.stderr)
    );
    let audit = read("scripts/audit-native-artifact.ps1");
    for required in [
        "llvm-readobj.exe",
        "--coff-imports",
        "--coff-exports",
        "--symbols",
    ] {
        assert!(
            audit.contains(required),
            "Windows artifact audit must retain {required:?}"
        );
    }
    assert!(
        !audit.contains("Get-Command dumpbin.exe"),
        "Windows artifact audit must not depend on an initialized SDK PATH"
    );
    for (mode, evidence) in [
        (
            "wrong-dependency",
            "dependencies must be exactly kernel32.dll",
        ),
        ("module-import", "computation DLL must have no imports"),
        ("missing-export", "does not export answer"),
        ("forbidden-export", "forbidden computation DLL export"),
        ("forbidden-symbol", "forbidden runtime symbol"),
        ("empty-symbols", "no symbol descriptors"),
        ("missing-symbol-container", "malformed symbol table"),
        ("unclosed-symbol-container", "malformed symbol table"),
        ("missing-symbol-name", "malformed symbol table"),
        ("duplicate-symbol-name", "malformed symbol table"),
        ("nonzero", "llvm-readobj --coff-imports failed"),
    ] {
        let output = run(mode);
        assert!(!output.status.success(), "artifact audit accepted {mode}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        let normalized_stderr = normalize_powershell_diagnostic(&stderr);
        assert!(
            normalized_stderr.contains(evidence),
            "artifact audit rejection for {mode} omitted {evidence:?}:\n{}",
            stderr
        );
    }

    fs::remove_dir_all(temp).expect("remove fake artifact audit tree");
}

#[test]
fn darwin_lc_main_should_use_the_generated_c_abi_entry_without_a_raw_stack_stub() {
    let bridge = read("native/bridge/ckc_llvm.cpp");
    assert!(
        bridge.contains("arguments.emplace_back(\"_main\")"),
        "dyld invokes LC_MAIN through the normal C ABI"
    );
    assert!(
        !read("native/runtime/darwin/process.c").contains("__ck_start"),
        "modern LC_MAIN does not require legacy raw-stack entry glue"
    );
}

#[cfg(unix)]
fn write_executable(path: &Path, source: &str) {
    fs::write(path, source).unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
        .unwrap_or_else(|error| panic!("chmod {}: {error}", path.display()));
}

#[test]
fn darwin_target_should_override_the_jit_large_code_model_for_shared_objects() {
    let bridge = read("native/bridge/ckc_llvm.cpp");
    let creation = bridge
        .split("int32_t finish_target_machine(")
        .nth(1)
        .expect("shared target-machine constructor")
        .split("llvm::Type *llvm_type")
        .next()
        .expect("target configuration helper boundary");
    assert!(creation.contains("builder.setRelocationModel(llvm::Reloc::PIC_)"));
    assert!(
        creation.contains("builder.setCodeModel(llvm::CodeModel::Small)"),
        "Mach-O needs an explicit small code model: JIT Large + PIC emits absolute text fixups"
    );
    assert_eq!(
        bridge.matches("return finish_target_machine(").count(),
        2,
        "host and explicit feature targets must share the same relocation/code-model policy"
    );
}

#[test]
fn provenance_inputs_should_preserve_blob_bytes_under_windows_autocrlf() {
    for path in [
        "third_party/licenses/RUST-COPYRIGHT",
        "native/llvm/LICENSE.TXT",
        "native/runtime/vendor/ryu/d2s.c",
        "native/runtime/common/runtime.c",
        "Cargo.lock",
        "benches/baselines/v0_10_compiler.toml",
    ] {
        let revision = format!("HEAD:{path}");
        let blob = std::process::Command::new("git")
            .current_dir(repo_root())
            .args(["cat-file", "blob", &revision])
            .output()
            .expect("read canonical Git blob");
        let checkout = std::process::Command::new("git")
            .current_dir(repo_root())
            .args([
                "-c",
                "core.autocrlf=true",
                "cat-file",
                "--filters",
                &revision,
            ])
            .output()
            .expect("simulate Windows checkout filters");
        assert!(blob.status.success() && checkout.status.success());
        assert!(
            blob.stdout == checkout.stdout,
            "checkout must preserve canonical hash/fixture bytes for {path}"
        );
    }
}

#[cfg(unix)]
fn mocked_elf_audit_root() -> (PathBuf, PathBuf) {
    let root = super::support::temp::temp_dir("ckc-elf-audit-contract");
    let artifacts = root.join("artifacts");
    let runtime = artifacts.join("runtime");
    let tools = root.join("tools");
    fs::create_dir_all(&runtime).expect("create mock runtime artifacts");
    fs::create_dir_all(&tools).expect("create mock audit tools");
    for relative in [
        "module.o",
        "libmodule.a",
        "libmodule.so",
        "program",
        "runtime/runtime.o",
        "runtime/SHA256SUMS",
    ] {
        fs::write(artifacts.join(relative), b"fixture").expect("write mock artifact");
    }

    let dispatcher = r#"#!/usr/bin/env bash
set -euo pipefail
case "$(basename "$0")" in
  uname) printf 'Linux\n' ;;
  sha256sum) exit 0 ;;
  file) printf '%s: current ar archive\n' "$1" ;;
  nm)
    if [[ " $* " == *' -D --defined-only '* ]]; then
      printf '00000000 T answer\n'
    fi
    ;;
  readelf)
    case "$1:$2" in
      -h:*module.o) printf '  Type:                              REL (Relocatable file)\n' ;;
      -h:*libmodule.so) printf '  Type:                              DYN (Shared object file)\n' ;;
      -h:*program) printf '  Type:                              EXEC (Executable file)\n' ;;
      -d:*) printf 'There is no dynamic section in this file.\n' ;;
      -p:.comment)
        printf "String dump of section '.comment':\n  [     0]  %s\n" "${CKC_TEST_ELF_COMMENT:-Linker: LLD 22.1.8}"
        ;;
      -SW:*) printf '  [ 1] .comment PROGBITS 00000000 000040 000013 01  %s  0   0  1\n' "${CKC_TEST_ELF_COMMENT_FLAGS:-MS}" ;;
      *) printf 'unexpected readelf arguments: %s\n' "$*" >&2; exit 64 ;;
    esac
    ;;
  *) printf 'unexpected mock tool: %s\n' "$0" >&2; exit 64 ;;
esac
"#;
    for tool in ["uname", "sha256sum", "file", "nm", "readelf"] {
        write_executable(&tools.join(tool), dispatcher);
    }
    (artifacts, tools)
}

#[cfg(unix)]
fn run_mocked_elf_audit(comment: &str, flags: &str) -> std::process::Output {
    let (artifacts, tools) = mocked_elf_audit_root();
    let inherited_path = std::env::var_os("PATH").expect("PATH must be set");
    let path = std::env::join_paths(
        std::iter::once(tools.clone()).chain(std::env::split_paths(&inherited_path)),
    )
    .expect("construct mock PATH");
    let output = Command::new("bash")
        .arg(repo_root().join("scripts/audit-native-artifact.sh"))
        .arg(&artifacts)
        .env("PATH", path)
        .env("CKC_TEST_ELF_COMMENT", comment)
        .env("CKC_TEST_ELF_COMMENT_FLAGS", flags)
        .output()
        .expect("run native ELF audit with pinned LLD provenance");
    let root = artifacts.parent().expect("mock audit parent");
    let _ = fs::remove_dir_all(root);
    output
}

#[cfg(unix)]
#[test]
fn native_elf_audit_should_accept_pinned_non_alloc_lld_provenance() {
    let output = run_mocked_elf_audit("Linker: LLD 22.1.8", "MS");
    assert!(
        output.status.success(),
        "pinned non-ALLOC LLD provenance is metadata, not a runtime dependency:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
#[test]
fn native_elf_audit_should_reject_allocated_or_unpinned_provenance() {
    for (comment, flags, expected) in [
        (
            "Linker: LLD 22.1.8",
            "AMS",
            "ELF producer metadata must be non-ALLOC",
        ),
        (
            "Linker: LLD 22.1.7",
            "MS",
            "missing pinned ELF linker provenance",
        ),
        (
            "Linker: LLD 22.1.80",
            "MS",
            "missing pinned ELF linker provenance",
        ),
    ] {
        let output = run_mocked_elf_audit(comment, flags);
        assert!(
            !output.status.success(),
            "audit unexpectedly accepted {comment}"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected),
            "audit did not report {expected:?}:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn native_toolchain_notices_should_be_repository_owned() {
    for path in [
        "native/llvm/LICENSE.TXT",
        "native/llvm/LLD-LICENSE.TXT",
        "native/llvm/third-party/BLAKE3-LICENSE",
        "native/llvm/third-party/COPYRIGHT.regex",
        "src/backend/llvm/notices.rs",
    ] {
        assert!(repo_root().join(path).is_file(), "missing {path}");
    }
}

#[test]
fn cargo_and_rust_provenance_should_be_complete_hashed_and_embedded() {
    let provenance = read("third_party/cargo/provenance.toml");
    let notices = read("THIRD_PARTY_NOTICES.md");
    let lock = read("Cargo.lock");
    let expected: BTreeSet<&str> = [
        "block-buffer",
        "bumpalo",
        "cc",
        "cfg-if",
        "cpufeatures",
        "crypto-common",
        "digest",
        "find-msvc-tools",
        "generic-array",
        "leb128fmt",
        "libc",
        "memchr",
        "proc-macro2",
        "quote",
        "sha2",
        "shlex",
        "syn",
        "thiserror",
        "thiserror-impl",
        "typenum",
        "unicode-ident",
        "unicode-width",
        "version_check",
        "wasm-encoder",
        "wast",
        "wat",
    ]
    .into_iter()
    .collect();
    let blocks: Vec<&str> = provenance.split("[[cargo]]").skip(1).collect();
    let actual: BTreeSet<String> = blocks
        .iter()
        .map(|block| quoted_scalar(block, "name"))
        .collect();
    assert_eq!(
        actual,
        expected.iter().map(ToString::to_string).collect(),
        "Cargo source/build dependency provenance must be exact"
    );

    for block in blocks {
        let name = quoted_scalar(block, "name");
        let version = quoted_scalar(block, "version");
        let checksum = quoted_scalar(block, "crate_sha256");
        let lock_identity = format!("name = \"{name}\"\nversion = \"{version}\"");
        assert!(
            lock.contains(&lock_identity),
            "Cargo.lock is missing {name} {version}"
        );
        assert!(
            lock.contains(&format!("checksum = \"{checksum}\"")),
            "Cargo.lock checksum drift for {name} {version}"
        );

        let license_files = quoted_array(block, "license_files");
        let license_hashes = quoted_array(block, "license_sha256");
        assert_eq!(license_files.len(), license_hashes.len());
        for (path, expected_hash) in license_files.iter().zip(&license_hashes) {
            let path = repo_root().join(path);
            assert!(path.is_file(), "missing license file {}", path.display());
            assert_eq!(
                sha256(&path),
                *expected_hash,
                "stale license file for {name}"
            );
        }
        for required in [&name, &version, &checksum] {
            assert!(
                notices.contains(required),
                "third-party notice index does not reference {name} {required}"
            );
        }
    }

    for required in [
        "version = \"1.90.0\"",
        "rust-src-1.90.0.tar.xz",
        "cde088d57064d151b2236f4619aea4a8207e0709eb3035ddc6617d609ab7d453",
        "third_party/licenses/RUST-COPYRIGHT",
        "third_party/licenses/RUST-LICENSE-MIT",
    ] {
        assert!(
            provenance.contains(required),
            "Rust provenance missing {required:?}"
        );
    }

    let source = read("src/backend/llvm/notices.rs");
    for required in [
        "THIRD_PARTY_NOTICES.md",
        "third_party/cargo/provenance.toml",
        "RUST-COPYRIGHT",
        "RUST-LICENSE-MIT",
        "LICENSE-UNICODE",
    ] {
        assert!(
            source.contains(required),
            "embedded notices missing {required:?}"
        );
    }
    let build = read("build.rs");
    assert!(build.contains("validate_third_party_provenance"));

    let output = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .arg("licenses")
        .output()
        .expect("run ckc licenses");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("license output is UTF-8");
    assert!(
        stdout.contains(&notices),
        "ckc licenses must embed the exact third-party notice index"
    );
}

#[test]
fn llvm_and_runtime_provenance_should_pin_sources_and_license_hashes() {
    let llvm = read("native/llvm/manifest.toml");
    let runtime = read("native/runtime/provenance.toml");
    let notices = read("THIRD_PARTY_NOTICES.md");

    for required in [
        "license_files = [\"LICENSE.TXT\", \"LLD-LICENSE.TXT\", \"third-party/BLAKE3-LICENSE\", \"third-party/COPYRIGHT.regex\"]",
        "license_sha256 = [\"3340babe8ac7bc6ae294d93aa01c310a250d43d5b760e5c12954882d4e5c83c7\"",
        "f7891568956e34643eb6a0db1462db30820d40d7266e2a78063f2fe233ece5a0",
        "6a94bedb8b707ed97f6e310d0d015ab14e0683ffa0a612b02958581b9cc9fc0e",
        "0424e57d4303164dc59a8509c20dae0518b853692e5c2b0e98b11816fdbc97c7",
    ] {
        assert!(
            llvm.contains(required),
            "LLVM provenance missing {required:?}"
        );
    }
    for required in [
        "source_sha256 = [\"f50df6ebc19075d2aa7b2ff5114bb6b2d953ee905bf2b2d4d8deb5390a36c631\"",
        "license_sha256 = [\"c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4\"",
        "c9bff75738922193e67fa726fa225535870d2aa1059f91452c411736284ad566",
    ] {
        assert!(
            runtime.contains(required),
            "runtime provenance missing {required:?}"
        );
    }
    for component in ["LLVM 22.1.8", "LLD 22.1.8", "BLAKE3", "regex", "Ryu"] {
        assert!(
            notices.contains(component),
            "third-party notice index must enumerate {component}"
        );
    }
}

#[test]
fn native_toolchain_bootstrap_outputs_should_remain_untracked() {
    let ignore = read(".gitignore");

    assert!(
        ignore.lines().any(|line| line == "/build/llvm/"),
        "ignore deterministic LLVM bootstrap outputs"
    );
    let tracked = Command::new("git")
        .arg("ls-files")
        .arg("build/llvm")
        .current_dir(repo_root())
        .output()
        .expect("inspect tracked LLVM bootstrap outputs");
    assert!(tracked.status.success(), "git ls-files failed");
    assert!(
        tracked.stdout.is_empty(),
        "LLVM bootstrap output must not be tracked in the source tree"
    );
}

#[test]
fn native_toolchain_bridge_should_define_owned_c_abi_results() {
    let header = read("native/bridge/ckc_llvm.h");

    for required in [
        "CKC_LLVM_BRIDGE_ABI_VERSION",
        "CkcLlvmOwnedBytes",
        "CkcLlvmError",
        "ckc_llvm_bridge_info",
        "ckc_llvm_test_error",
        "ckc_llvm_owned_bytes_dispose",
        "static_assert",
        "_Static_assert",
    ] {
        assert!(
            header.contains(required),
            "native bridge header must contain {required:?}"
        );
    }

    for path in [
        "native/bridge/ckc_llvm.cpp",
        "native/bridge/ownership_smoke.cpp",
        "src/backend/llvm/ffi.rs",
        "src/backend/llvm/error.rs",
    ] {
        assert!(repo_root().join(path).is_file(), "missing {path}");
    }
}

#[test]
fn x86_integer_reduction_handoff_should_pin_backend_interleave_width() {
    let bridge = read("native/bridge/ckc_llvm.cpp");
    for required in [
        "attach_x86_integer_reduction_interleave",
        "llvm.loop.interleave.count",
        "CKC_X86_REDUCTION_INTERLEAVE = 8",
    ] {
        assert!(
            bridge.contains(required),
            "x86 integer-reduction handoff omitted {required}"
        );
    }
}
