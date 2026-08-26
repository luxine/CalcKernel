use std::{
    fs,
    path::Path,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn readmes_should_describe_native_rust_ckc_release_surface() {
    for path in ["README.md", "README.zh-CN.md"] {
        let text = read(path);
        for required in [
            "native ckc",
            "docs/native-release.md",
            "cargo test --locked",
            "cargo build --release --locked",
        ] {
            assert!(text.contains(required), "{path} must mention {required:?}");
        }

        for forbidden in [
            "docs/npm-release.md",
            "npm run",
            "npm artifact",
            "npm package surface",
            "root JavaScript",
            "TypeScript package migration",
        ] {
            assert!(
                !text.contains(forbidden),
                "{path} must not mention {forbidden:?}"
            );
        }
    }
}

#[test]
fn formal_docs_should_have_simplified_chinese_counterparts() {
    let docs_root = repo_root().join("docs");
    let zh_root = docs_root.join("zh-CN");
    let mut missing = Vec::new();

    for entry in fs::read_dir(&docs_root).expect("read docs directory") {
        let entry = entry.expect("read docs entry");
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("md") {
            continue;
        }
        let name = path.file_name().expect("doc file name");
        if !zh_root.join(name).exists() {
            missing.push(name.to_string_lossy().into_owned());
        }
    }

    assert!(
        missing.is_empty(),
        "formal docs must have docs/zh-CN counterparts:\n{}",
        missing.join("\n")
    );
}

#[test]
fn native_release_docs_should_own_release_checklist_language() {
    let checklist = read("docs/RELEASE_CHECKLIST.md");
    for required in [
        "cargo fmt --check",
        "cargo clippy --all-targets --all-features --locked -- -D warnings",
        "cargo test --locked",
        "cargo build --release --locked",
        "SHA256",
        "GitHub Release",
    ] {
        assert!(
            checklist.contains(required),
            "release checklist must include {required:?}"
        );
    }

    for forbidden in [
        "package.json",
        "pnpm ",
        "npm ",
        "npm pack",
        "npm publish",
        "node_modules",
        "fresh-install",
    ] {
        assert!(
            !checklist.contains(forbidden),
            "release checklist must not mention {forbidden:?}"
        );
    }
}

#[test]
fn architecture_review_should_reflect_native_only_boundary() {
    for path in [
        "docs/architecture-review.md",
        "docs/zh-CN/architecture-review.md",
    ] {
        let text = read(path);
        for required in ["native ckc", "Cargo.toml", "src/main.rs", "No npm"] {
            assert!(text.contains(required), "{path} must mention {required:?}");
        }

        for forbidden in [
            "npm/",
            "package API",
            "JavaScript compatibility surface",
            "npm package replacement",
            "npm publish",
        ] {
            assert!(
                !text.contains(forbidden),
                "{path} must not mention {forbidden:?}"
            );
        }
    }
}

#[test]
fn wasm_docs_should_describe_artifacts_not_removed_helper_apis() {
    for path in [
        "docs/ckc-outputs.md",
        "docs/zh-CN/ckc-outputs.md",
        "docs/WASM_ABI.md",
        "docs/zh-CN/WASM_ABI.md",
        "docs/wasm-interop.md",
        "docs/zh-CN/wasm-interop.md",
    ] {
        let text = read(path);
        for required in [
            "ckc emit-wasm",
            "WebAssembly runtime",
            "caller-owned memory",
        ] {
            assert!(text.contains(required), "{path} must mention {required:?}");
        }

        for forbidden in [
            "CKWasmArena",
            "createCKWasmArena",
            "package-root",
            "package root",
            "from \"calckernel\"",
            "npm-distributed",
            "ready-to-publish npm",
        ] {
            assert!(
                !text.contains(forbidden),
                "{path} must not mention {forbidden:?}"
            );
        }
    }
}

#[test]
fn docs_should_not_reference_unshipped_benchmark_or_example_scripts() {
    let mut failures = Vec::new();
    for path in markdown_files(&repo_root().join("docs")) {
        let relative = path
            .strip_prefix(repo_root())
            .expect("doc under repo root")
            .display()
            .to_string();
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));

        for forbidden in [
            "node bench/perf/run.mjs",
            "node bench/pricing_baseline.js",
            "node bench/wasm_pricing_benchmark.mjs",
            "node examples/wasm/f64-sum/run.mjs",
            "node examples/wasm/f64-axpy/run.mjs",
            "node examples/wasm/pricing-soa/run.mjs",
            "node --test bench/perf/tests",
        ] {
            if text.contains(forbidden) {
                failures.push(format!(
                    "{relative} references unshipped script {forbidden}"
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "docs must not reference benchmark/example scripts absent from native ckc repo:\n{}",
        failures.join("\n")
    );
}

#[test]
fn control_flow_docs_should_cover_break_continue_and_unreachable_rules() {
    for path in ["docs/LANGUAGE_SPEC.md", "docs/zh-CN/LANGUAGE_SPEC.md"] {
        let text = read(path);
        for required in ["`break;`", "`continue;`", "`CK2009`", "`CK2010`"] {
            assert!(text.contains(required), "{path} must mention {required:?}");
        }
    }

    for path in [
        "docs/COMPILER_ARCHITECTURE.md",
        "docs/zh-CN/COMPILER_ARCHITECTURE.md",
        "docs/MIR.md",
        "docs/zh-CN/MIR.md",
    ] {
        let text = read(path);
        for required in [
            "`break`",
            "`continue`",
            "`MirTerminator::Jump`",
            "dispatcher",
        ] {
            assert!(text.contains(required), "{path} must mention {required:?}");
        }
    }

    for path in ["docs/ROADMAP.md", "docs/zh-CN/ROADMAP.md"] {
        let text = read(path);
        assert!(
            text.contains("`break` / `continue`"),
            "{path} must mark Phase A"
        );
    }

    for path in ["README.md", "README.zh-CN.md"] {
        assert!(
            read(path).contains("examples/control_flow.ck"),
            "{path} must link the control-flow example"
        );
    }
    assert!(repo_root().join("examples/control_flow.ck").is_file());
}

#[test]
fn void_docs_should_cover_return_only_type_and_backend_abis() {
    for path in ["docs/LANGUAGE_SPEC.md", "docs/zh-CN/LANGUAGE_SPEC.md"] {
        let text = read(path);
        for required in ["`-> void`", "`return;`", "`CK2011`", "return-only"] {
            assert!(text.contains(required), "{path} must mention {required:?}");
        }
    }

    for path in ["docs/MIR.md", "docs/zh-CN/MIR.md"] {
        let text = read(path);
        for required in [
            "`MirType::Void`",
            "`target: None`",
            "`value: None`",
            "synthetic",
        ] {
            assert!(text.contains(required), "{path} must mention {required:?}");
        }
    }

    for path in ["docs/ABI.md", "docs/zh-CN/ABI.md"] {
        let text = read(path);
        for required in ["C `void`", "`CK_Status`", "no `ck_return`"] {
            assert!(text.contains(required), "{path} must mention {required:?}");
        }
    }

    for path in [
        "docs/CHECKED_ARITHMETIC.md",
        "docs/zh-CN/CHECKED_ARITHMETIC.md",
    ] {
        let text = read(path);
        for required in ["void", "`CK_OK`", "status propagation"] {
            assert!(text.contains(required), "{path} must mention {required:?}");
        }
    }

    for path in ["docs/WASM_ABI.md", "docs/zh-CN/WASM_ABI.md"] {
        let text = read(path);
        for required in ["void", "no `(result ...)`", "targetless"] {
            assert!(text.contains(required), "{path} must mention {required:?}");
        }
    }

    for path in ["docs/LLVM_BACKEND.md", "docs/zh-CN/LLVM_BACKEND.md"] {
        let text = read(path);
        for required in ["`void`", "`call void`", "`ret void`"] {
            assert!(text.contains(required), "{path} must mention {required:?}");
        }
    }

    for path in [
        "docs/COMPILER_ARCHITECTURE.md",
        "docs/zh-CN/COMPILER_ARCHITECTURE.md",
        "docs/ROADMAP.md",
        "docs/zh-CN/ROADMAP.md",
    ] {
        assert!(read(path).contains("`void`"), "{path} must cover void");
    }

    for path in ["README.md", "README.zh-CN.md"] {
        assert!(
            read(path).contains("examples/void.ck"),
            "{path} must link the void example"
        );
    }
    assert!(repo_root().join("examples/void.ck").is_file());
}

#[test]
fn slice_docs_should_define_ownership_bounds_and_backend_matrix() {
    for path in ["docs/LANGUAGE_SPEC.md", "docs/zh-CN/LANGUAGE_SPEC.md"] {
        let text = read(path);
        for required in [
            "`slice<T>`",
            "`slice(data, len)`",
            "`items[start..end]`",
            "`u32`",
            "`start <= end <= items.len`",
            "read-only",
            "`.data`",
            "`CK2012`",
        ] {
            assert!(text.contains(required), "{path} must mention {required:?}");
        }
    }

    for path in [
        "docs/COMPILER_ARCHITECTURE.md",
        "docs/zh-CN/COMPILER_ARCHITECTURE.md",
        "docs/MIR.md",
        "docs/zh-CN/MIR.md",
    ] {
        let text = read(path);
        for required in [
            "`MirType::Slice`",
            "`MakeSlice`",
            "`SliceIndex`",
            "`Subslice`",
        ] {
            assert!(text.contains(required), "{path} must mention {required:?}");
        }
    }

    for path in [
        "README.md",
        "README.zh-CN.md",
        "docs/ABI.md",
        "docs/zh-CN/ABI.md",
        "docs/CHECKED_ARITHMETIC.md",
        "docs/zh-CN/CHECKED_ARITHMETIC.md",
        "docs/WASM_ABI.md",
        "docs/zh-CN/WASM_ABI.md",
        "docs/LLVM_BACKEND.md",
        "docs/zh-CN/LLVM_BACKEND.md",
        "docs/OPTIMIZATION.md",
        "docs/zh-CN/OPTIMIZATION.md",
        "docs/ROADMAP.md",
        "docs/zh-CN/ROADMAP.md",
        "docs/ckc-outputs.md",
        "docs/zh-CN/ckc-outputs.md",
        "docs/wasm-interop.md",
        "docs/zh-CN/wasm-interop.md",
    ] {
        let text = read(path);
        assert!(text.contains("`slice<T>`"), "{path} must cover slices");
        assert!(text.contains("--bounds"), "{path} must cover bounds mode");
    }

    for path in ["docs/MIGRATION.md", "docs/zh-CN/MIGRATION.md"] {
        let text = read(path);
        for keyword in ["`break`", "`continue`", "`void`", "`slice`"] {
            assert!(text.contains(keyword), "{path} must reserve {keyword}");
        }
    }

    for path in ["README.md", "README.zh-CN.md"] {
        assert!(
            read(path).contains("examples/slices.ck"),
            "{path} must link the slice example"
        );
    }
}

#[test]
fn slice_example_should_run_with_equal_valid_results_across_backends() {
    let example = repo_root().join("examples/slices.ck");
    let source = fs::read_to_string(&example).expect("read slice example");
    for required in [
        "slice(data, len)",
        "slice<Item>",
        "[start..end]",
        ".data",
        ".len",
        "selected[0].value",
    ] {
        assert!(
            source.contains(required),
            "slice example must contain {required:?}"
        );
    }

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_slice_example_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let c_path = dir.join("kernel.c");
    let h_path = dir.join("kernel.h");
    let ll_path = dir.join("kernel.ll");
    let wasm_path = dir.join("kernel.wasm");

    let emit_c = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .arg("emit-c")
        .arg(&example)
        .arg("--out")
        .arg(&c_path)
        .arg("--header")
        .arg(&h_path)
        .output()
        .expect("emit C example");
    assert!(
        emit_c.status.success(),
        "{}",
        String::from_utf8_lossy(&emit_c.stderr)
    );
    let c_harness = dir.join("c_harness.c");
    fs::write(
        &c_harness,
        r#"
#include <stdio.h>
#include "kernel.h"
int main(void) {
  Item items[3] = {{2}, {7}, {11}};
  printf("%d,%u\n", slice_sum(items, 3, 1, 3), slice_len(items, 3));
  return 0;
}
"#,
    )
    .expect("write C harness");
    let c_binary = dir.join("c_harness");
    compile_native(&[&c_path, &c_harness], &c_binary);
    let c_output = run_stdout(&c_binary);

    let emit_llvm = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .arg("emit-llvm")
        .arg(&example)
        .arg("--out")
        .arg(&ll_path)
        .output()
        .expect("emit LLVM example");
    assert!(
        emit_llvm.status.success(),
        "{}",
        String::from_utf8_lossy(&emit_llvm.stderr)
    );
    let llvm_harness = dir.join("llvm_harness.c");
    fs::write(
        &llvm_harness,
        r#"
#include <stdint.h>
#include <stdio.h>
typedef struct Item { int32_t value; } Item;
int32_t slice_sum(Item* data, uint32_t len, uint32_t start, uint32_t end);
uint32_t slice_len(Item* data, uint32_t len);
int main(void) {
  Item items[3] = {{2}, {7}, {11}};
  printf("%d,%u\n", slice_sum(items, 3, 1, 3), slice_len(items, 3));
  return 0;
}
"#,
    )
    .expect("write LLVM harness");
    let llvm_binary = dir.join("llvm_harness");
    compile_native(&[&ll_path, &llvm_harness], &llvm_binary);
    let llvm_output = run_stdout(&llvm_binary);

    let emit_wasm = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .arg("emit-wasm")
        .arg(&example)
        .arg("--out")
        .arg(&wasm_path)
        .output()
        .expect("emit WASM example");
    assert!(
        emit_wasm.status.success(),
        "{}",
        String::from_utf8_lossy(&emit_wasm.stderr)
    );
    let node = Command::new("node")
        .arg("-e")
        .arg(
            r#"
const fs = require("node:fs");
WebAssembly.instantiate(fs.readFileSync(process.argv[1])).then(({instance}) => {
  const view = new DataView(instance.exports.memory.buffer);
  view.setInt32(0, 2, true);
  view.setInt32(4, 7, true);
  view.setInt32(8, 11, true);
  console.log(`${instance.exports.slice_sum(0, 3, 1, 3)},${instance.exports.slice_len(0, 3)}`);
}).catch((error) => { console.error(error); process.exit(1); });
"#,
        )
        .arg(&wasm_path)
        .output()
        .expect("run WASM example");
    assert!(
        node.status.success(),
        "{}",
        String::from_utf8_lossy(&node.stderr)
    );
    let wasm_output = String::from_utf8(node.stdout).expect("WASM output UTF-8");

    assert_eq!(c_output, "18,3\n");
    assert_eq!(llvm_output, c_output);
    assert_eq!(wasm_output, c_output);
}

fn compile_native(inputs: &[&Path], output: &Path) {
    let compile = Command::new("clang")
        .args(inputs)
        .arg("-std=c11")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-Werror")
        .arg("-Wno-override-module")
        .arg("-o")
        .arg(output)
        .output()
        .expect("run clang");
    assert!(
        compile.status.success(),
        "clang failed:\n{}",
        String::from_utf8_lossy(&compile.stderr)
    );
}

fn run_stdout(binary: &Path) -> String {
    let output = Command::new(binary).output().expect("run native harness");
    assert!(output.status.success(), "native harness failed");
    String::from_utf8(output.stdout).expect("native output UTF-8")
}

fn markdown_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).unwrap_or_else(|error| panic!("read {}: {error}", dir.display()))
    {
        let path = entry.expect("read directory entry").path();
        if path.is_dir() {
            files.extend(markdown_files(&path));
        } else if path.extension().and_then(|value| value.to_str()) == Some("md") {
            files.push(path);
        }
    }
    files
}

fn read(path: &str) -> String {
    fs::read_to_string(repo_root().join(path))
        .unwrap_or_else(|error| panic!("read {path}: {error}"))
}
