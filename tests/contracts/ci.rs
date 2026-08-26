use std::fs;

use super::support::oracle::repo_root;

#[test]
fn daily_ci_should_gate_pull_requests_and_main_pushes() {
    let workflow_path = repo_root().join(".github/workflows/ci.yml");
    let workflow = fs::read_to_string(&workflow_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", workflow_path.display()));

    for required in [
        "name: CI",
        "pull_request:\n    branches: [main]",
        "push:\n    branches: [main]",
        "contents: read",
        "cancel-in-progress: true",
        "name: quality",
        "runs-on: ubuntu-24.04",
        "CALCKERNEL_TS_ROOT: ${{ github.workspace }}/typescript-oracle",
        "repository: luxine/CalcKernel",
        "ref: 5e989939d89d75056e5f3bea25f3bf7204d5529a",
        "path: typescript-oracle",
        "uses: actions/checkout@v7",
        "uses: actions/setup-node@v7",
        "node-version: 20",
        "corepack prepare pnpm@9.15.9 --activate",
        "pnpm install --frozen-lockfile",
        "pnpm build",
        "components: rustfmt, clippy",
        "cargo fmt --check",
        "cargo clippy --all-targets --all-features --locked -- -D warnings",
        "cargo test --locked",
        "cargo build --release --locked",
        "./target/release/ckc --help",
        "./target/release/ckc check examples/core/scalar.ck",
        "./target/release/ckc emit-mir examples/core/scalar.ck -O3",
        "./target/release/ckc emit-c examples/applications/pricing.ck --out build/pricing.c --header build/pricing.h",
        "./target/release/ckc emit-wasm examples/wasm/scalar.ck --out build/scalar.wasm",
        "./target/release/ckc emit-llvm examples/llvm/scalar.ck --out build/scalar.ll",
    ] {
        assert!(
            workflow.contains(required),
            "daily CI workflow must contain {required:?}"
        );
    }

    for forbidden in [
        "actions/upload-artifact",
        "actions/download-artifact",
        "gh release upload",
        "publish-release:",
        "build-artifacts:",
        "tags:",
    ] {
        assert!(
            !workflow.contains(forbidden),
            "daily CI workflow must not contain {forbidden:?}"
        );
    }
}
