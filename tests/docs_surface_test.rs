use std::{fs, path::Path};

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
