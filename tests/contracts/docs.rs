use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

#[cfg(feature = "native-toolchain")]
use std::process::Command;

#[cfg(feature = "native-toolchain")]
use super::support::command::run_stdout;
use super::support::oracle::repo_root;
#[cfg(feature = "native-toolchain")]
use super::support::temp::unique_id;

#[test]
fn durable_docs_should_be_recursive_bilingual_mirrors() {
    let docs = repo_root().join("docs");
    let english = relative_markdown_files(&docs, Some(Path::new("zh-CN")));
    let chinese = relative_markdown_files(&docs.join("zh-CN"), None);
    assert_eq!(
        english, chinese,
        "English and zh-CN Markdown trees must mirror"
    );
}

#[test]
fn durable_docs_should_have_resolvable_local_links() {
    let mut failures = Vec::new();
    for source in markdown_files(&repo_root().join("docs")) {
        let text = fs::read_to_string(&source)
            .unwrap_or_else(|error| panic!("read {}: {error}", source.display()));
        for target in markdown_targets(&text) {
            if target.starts_with('#')
                || target.starts_with("http://")
                || target.starts_with("https://")
                || target.starts_with("mailto:")
            {
                continue;
            }
            let path_only = target.split('#').next().unwrap_or_default();
            if path_only.is_empty() {
                continue;
            }
            let resolved = source.parent().expect("Markdown parent").join(path_only);
            if !resolved.exists() {
                failures.push(format!("{} -> {target}", source.display()));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "local Markdown links must resolve:\n{}",
        failures.join("\n")
    );
}

#[test]
fn durable_docs_should_use_current_contract_wording() {
    let forbidden = [
        "Phase 10",
        "Phase 11",
        "Phase 12",
        "Phase 13",
        "Phase 14",
        "Phase 16",
        "Phase 20",
        "Phase 21",
        "TypeScript-era",
        "if the language gains a length-carrying pointer type",
        "如果语言引入携带长度的 pointer type",
        "approved forward design",
        "已获准的未来设计",
        "not the current V0.9 implementation contract",
        "不是当前 V0.9 implementation contract",
    ];
    let mut failures = Vec::new();
    for path in markdown_files(&repo_root().join("docs")) {
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        for phrase in forbidden {
            if text.contains(phrase) {
                failures.push(format!("{} contains {phrase:?}", path.display()));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "durable docs must describe only the current contract:\n{}",
        failures.join("\n")
    );
}

#[test]
fn v0_11_docs_should_freeze_language_cli_mir_kir_abi_runtime_and_distribution() {
    let required_by_file = [
        (
            "docs/reference/language.md",
            &[
                "CalcKernel 0.11",
                "fn main() -> void",
                "fn main() -> i32",
                "print_i32",
                "print_f64",
                "reachable print",
                "slice(data, len)",
                "start <= end <= len",
                "unsafe fn",
                "noalias",
                "effects",
            ][..],
        ),
        (
            "docs/reference/cli.md",
            &[
                "ckc run",
                "ckc cache clean",
                "--version --verbose",
                "ckc licenses",
                "executable|dynamic|static|object",
                "CKC_LLVM_PREFIX",
                "host triple",
                "emit-kir",
                "--sanitize-contracts",
            ][..],
        ),
        (
            "docs/reference/mir.md",
            &[
                "CalcKernel 0.11",
                "entry",
                "runtime effect",
                "print",
                "checked",
                "Memory SSA",
                "Proof",
            ][..],
        ),
        (
            "docs/abi/llvm.md",
            &[
                "Native C ABI",
                "LLVM 22.1.8",
                "host-only",
                "export thunk",
                "JITLink",
                "RuntimeDyld",
                "fact audit",
                "KIR v1",
            ][..],
        ),
        (
            "docs/abi/c.md",
            &["emit-c", "source-only", "reachable print", "CK_Status"][..],
        ),
        (
            "docs/abi/wasm.md",
            &["reachable print", "rejected", "caller-owned", "slice<T>"][..],
        ),
        (
            "docs/abi/modes.md",
            &[
                "Native",
                "CK_ERR_OVERFLOW",
                "CK_ERR_OUT_OF_BOUNDS",
                "first error",
                "CKR0001: integer overflow",
                "CKR0006: native child terminated abnormally",
                "CKR0007: unsafe contract violation",
            ][..],
        ),
        (
            "docs/project/compatibility.md",
            &["0.11.x", "0.10.0", "0.9.0", "build-llvm", "Native C ABI"][..],
        ),
        (
            "docs/guides/performance.md",
            &["95%", "10%", "3%", "8%", "97%", "2x", "3x", "Clang 22.1.8"][..],
        ),
        (
            "docs/project/release.md",
            &["0.11.0", "native-toolchain", "ckc licenses", "six archives"][..],
        ),
    ];
    for (path, required) in required_by_file {
        let text = read(path);
        for phrase in required {
            assert!(text.contains(phrase), "{path} must contain {phrase:?}");
        }
    }

    for path in [
        "README.md",
        "README.zh-CN.md",
        "docs/index.md",
        "docs/zh-CN/index.md",
    ] {
        assert!(read(path).contains("0.11.0"), "{path} must identify 0.11.0");
    }

    let language = read("docs/reference/language.md");
    for required in ["shortest-round-trip", "-0.0", "nan", "inf", "CKR0005"] {
        assert!(
            language.contains(required),
            "print contract needs {required:?}"
        );
    }

    let cli = read("docs/reference/cli.md");
    for required in ["SHA-256", "1 GiB", "atomic", "trust boundary"] {
        assert!(cli.contains(required), "cache contract needs {required:?}");
    }

    for path in [
        "README.md",
        "README.zh-CN.md",
        "docs/reference/cli.md",
        "docs/zh-CN/reference/cli.md",
        "docs/abi/llvm.md",
        "docs/zh-CN/abi/llvm.md",
    ] {
        let text = read(path);
        for forbidden in ["through `clang`", "通过 `clang`", "-> clang ->"] {
            assert!(
                !text.contains(forbidden),
                "{path} retains obsolete external-Clang product behavior"
            );
        }
    }
}

#[test]
fn v0_11_docs_should_define_canonical_slice_and_mode_contracts() {
    let language = read("docs/reference/language.md");
    assert!(language.contains(
        "C and Native support optional `--bounds checked` guards for slice indexing and"
    ));
    assert!(language.contains("Raw pointer indexing, `slice(data, len)`, and indexing through"));

    for path in ["docs/abi/wasm.md"] {
        let text = read(path);
        for required in ["`slice<T>`", "`--bounds unchecked`", "reject"] {
            assert!(text.contains(required), "{path} must contain {required:?}");
        }
        assert!(
            !text.contains("slices are unsupported"),
            "{path} must not deny slice support"
        );
    }

    let llvm = read("docs/abi/llvm.md");
    for required in ["`slice<T>`", "Checked modes", "Native C ABI"] {
        assert!(llvm.contains(required), "LLVM contract needs {required:?}");
    }

    let modes = read("docs/abi/modes.md");
    for required in [
        "CK_OK",
        "CK_ERR_OVERFLOW",
        "CK_ERR_DIV_BY_ZERO",
        "CK_ERR_NULL_POINTER",
        "CK_ERR_OUT_OF_BOUNDS",
        "overflow before bounds",
        "Raw pointer",
    ] {
        assert!(
            modes.contains(required),
            "modes contract needs {required:?}"
        );
    }
}

#[test]
fn diagnostic_reference_should_cover_every_display_code() {
    let source = read("src/frontend/diagnostics.rs");
    let reference = read("docs/reference/diagnostics.md");
    let mut codes = BTreeSet::new();
    for line in source.lines() {
        if let Some(start) = line.find("\"CK") {
            let rest = &line[start + 1..];
            if let Some(end) = rest.find('"') {
                let code = &rest[..end];
                if code.len() == 6
                    && code[2..]
                        .chars()
                        .all(|character| character.is_ascii_digit())
                {
                    codes.insert(code.to_owned());
                }
            }
        }
    }
    assert!(
        !codes.is_empty(),
        "diagnostic source must expose stable codes"
    );
    for code in codes {
        assert!(
            reference.contains(&format!("| `{code}` |")),
            "diagnostic reference must document {code}"
        );
    }
}

#[test]
fn readmes_should_describe_native_rust_ckc_release_surface() {
    for path in ["README.md", "README.zh-CN.md"] {
        let text = read(path);
        for required in [
            "native ckc",
            "docs/project/release.md",
            "cargo test --all-features --locked",
            "cargo build --release --features native-toolchain --locked",
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
    let checklist = read("docs/project/release-checklist.md");
    for required in [
        "cargo fmt --check",
        "cargo clippy --all-targets --all-features --locked -- -D warnings",
        "cargo test --all-features --locked",
        "cargo build --release --features native-toolchain --locked",
        "SHA256",
        "GitHub Release",
    ] {
        assert!(
            checklist.contains(required),
            "release checklist must include {required:?}"
        );
    }

    let chinese = read("docs/zh-CN/project/release-checklist.md");
    assert!(checklist.contains("portable baseline CPU policy"));
    assert!(chinese.contains("portable baseline CPU policy"));
    assert!(!checklist.contains("baseline and native CPU policies"));

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
fn architecture_docs_should_describe_current_native_modules() {
    for path in [
        "docs/compiler/architecture.md",
        "docs/zh-CN/compiler/architecture.md",
    ] {
        let text = read(path);
        for required in [
            "src/frontend/",
            "src/ir/",
            "src/optimizer/",
            "src/backend/",
            "src/cli/",
        ] {
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
    for path in ["docs/reference/cli.md", "docs/zh-CN/reference/cli.md"] {
        let text = read(path);
        for required in ["emit-wasm", "WASM", "--bounds"] {
            assert!(text.contains(required), "{path} must mention {required:?}");
        }
    }

    for path in [
        "docs/abi/wasm.md",
        "docs/zh-CN/abi/wasm.md",
        "docs/guides/wasm-interop.md",
        "docs/zh-CN/guides/wasm-interop.md",
    ] {
        let text = read(path);
        for required in ["caller-owned", "`slice<T>`", "--bounds"] {
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
fn benchmark_schema_should_document_general_and_native_gate_outputs() {
    let schema = read("benches/summary-schema.md");
    for required in [
        "schemaVersion: 1",
        "schemaVersion: 4",
        "target/ckc-perf/results.json",
        "checked",
        "unchecked",
        "95%",
        "10%",
        "3%",
        "8%",
        "97%",
        "2x",
        "3x",
        "baselineV010",
        "sourceDigests",
        "scripts/check-native-performance.py",
    ] {
        assert!(
            schema.contains(required),
            "benchmark schema needs {required:?}"
        );
    }
}

#[test]
fn control_flow_docs_should_cover_break_continue_and_unreachable_rules() {
    for path in [
        "docs/reference/language.md",
        "docs/zh-CN/reference/language.md",
    ] {
        let text = read(path);
        for required in ["`break;`", "`continue;`", "`CK2009`", "`CK2010`"] {
            assert!(text.contains(required), "{path} must mention {required:?}");
        }
    }

    for path in [
        "docs/compiler/architecture.md",
        "docs/zh-CN/compiler/architecture.md",
        "docs/reference/mir.md",
        "docs/zh-CN/reference/mir.md",
    ] {
        let text = read(path);
        for required in ["`break`", "`continue`", "`MirTerminator::Jump`"] {
            assert!(text.contains(required), "{path} must mention {required:?}");
        }
    }

    for path in ["README.md", "README.zh-CN.md"] {
        assert!(
            read(path).contains("examples/core/control_flow.ck"),
            "{path} must link the control-flow example"
        );
    }
    assert!(repo_root().join("examples/core/control_flow.ck").is_file());
}

#[test]
fn void_docs_should_cover_return_only_type_and_backend_abis() {
    for path in [
        "docs/reference/language.md",
        "docs/zh-CN/reference/language.md",
    ] {
        let text = read(path);
        for required in ["`-> void`", "`return;`", "`CK2011`", "return-only"] {
            assert!(text.contains(required), "{path} must mention {required:?}");
        }
    }

    for path in ["docs/reference/mir.md", "docs/zh-CN/reference/mir.md"] {
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

    for path in ["docs/abi/c.md", "docs/zh-CN/abi/c.md"] {
        let text = read(path);
        for required in ["void", "`CK_Status`", "`ck_return`"] {
            assert!(text.contains(required), "{path} must mention {required:?}");
        }
    }

    for path in ["docs/abi/modes.md", "docs/zh-CN/abi/modes.md"] {
        let text = read(path);
        for required in ["void", "`CK_OK`", "`CK_Status`"] {
            assert!(text.contains(required), "{path} must mention {required:?}");
        }
    }

    for path in ["docs/abi/wasm.md", "docs/zh-CN/abi/wasm.md"] {
        let text = read(path);
        for required in ["void", "`(result ...)`", "targetless"] {
            assert!(text.contains(required), "{path} must mention {required:?}");
        }
    }

    for path in ["docs/abi/llvm.md", "docs/zh-CN/abi/llvm.md"] {
        let text = read(path);
        for required in ["`void`", "`call void`", "`ret void`"] {
            assert!(text.contains(required), "{path} must mention {required:?}");
        }
    }

    for path in [
        "docs/compiler/architecture.md",
        "docs/zh-CN/compiler/architecture.md",
    ] {
        assert!(read(path).contains("`void`"), "{path} must cover void");
    }

    for path in ["README.md", "README.zh-CN.md"] {
        assert!(
            read(path).contains("examples/core/void.ck"),
            "{path} must link the void example"
        );
    }
    assert!(repo_root().join("examples/core/void.ck").is_file());
}

#[test]
fn slice_docs_should_define_ownership_bounds_and_backend_matrix() {
    for path in [
        "docs/reference/language.md",
        "docs/zh-CN/reference/language.md",
    ] {
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
        "docs/compiler/architecture.md",
        "docs/zh-CN/compiler/architecture.md",
        "docs/reference/mir.md",
        "docs/zh-CN/reference/mir.md",
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
        "docs/abi/c.md",
        "docs/zh-CN/abi/c.md",
        "docs/abi/modes.md",
        "docs/zh-CN/abi/modes.md",
        "docs/abi/wasm.md",
        "docs/zh-CN/abi/wasm.md",
        "docs/abi/llvm.md",
        "docs/zh-CN/abi/llvm.md",
        "docs/compiler/optimizer.md",
        "docs/zh-CN/compiler/optimizer.md",
        "docs/guides/wasm-interop.md",
        "docs/zh-CN/guides/wasm-interop.md",
    ] {
        let text = read(path);
        assert!(text.contains("`slice<T>`"), "{path} must cover slices");
        assert!(text.contains("--bounds"), "{path} must cover bounds mode");
    }

    for path in ["README.md", "README.zh-CN.md"] {
        assert!(
            read(path).contains("examples/core/slices.ck"),
            "{path} must link the slice example"
        );
    }
}

#[cfg(feature = "native-toolchain")]
#[test]
fn slice_example_should_run_with_equal_valid_results_across_backends() {
    let example = repo_root().join("examples/core/slices.ck");
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

    let unique = unique_id();
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

#[cfg(feature = "native-toolchain")]
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

fn relative_markdown_files(root: &Path, skip: Option<&Path>) -> BTreeSet<PathBuf> {
    markdown_files(root)
        .into_iter()
        .filter_map(|path| {
            let relative = path.strip_prefix(root).expect("file below root");
            if skip.is_some_and(|prefix| relative.starts_with(prefix)) {
                None
            } else {
                Some(relative.to_path_buf())
            }
        })
        .collect()
}

fn markdown_targets(text: &str) -> Vec<&str> {
    let mut targets = Vec::new();
    let mut remaining = text;
    while let Some(start) = remaining.find("](") {
        remaining = &remaining[start + 2..];
        let Some(end) = remaining.find(')') else {
            break;
        };
        let target = remaining[..end]
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim_matches(['<', '>']);
        targets.push(target);
        remaining = &remaining[end + 1..];
    }
    targets
}

fn read(path: &str) -> String {
    fs::read_to_string(repo_root().join(path))
        .unwrap_or_else(|error| panic!("read {path}: {error}"))
}
