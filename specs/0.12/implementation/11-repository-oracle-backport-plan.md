# CK 0.12 Repository-Owned Oracle Backport Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore a valid CK 0.12 quality gate after the GitHub repository rename by freezing the reviewed TypeScript oracle inside the repository without changing any language, ABI, optimizer, performance, or exact-SHA acceptance requirement.

**Architecture:** The quality job owns a test-only source snapshot at `tests/oracles/typescript`, verifies its 85-entry SHA-256 manifest before dependency installation, builds it from the frozen pnpm lockfile, and runs the existing live differential tests through `CALCKERNEL_TS_ROOT`. Repository contracts prevent a future private cross-repository checkout, release contracts keep the oracle out of shipped artifacts, and current project metadata uses the canonical `luxine/CalcKernel` identity.

**Tech Stack:** Rust 1.90 contract tests, GitHub Actions YAML, Node.js 20.19.5, pnpm 9.15.9, SHA-256 manifests, Git worktrees, GitHub CLI.

---

## File map

- Create `tests/oracles/typescript/**`: immutable test-only TypeScript compiler source, selected fixtures, provenance, and source manifest copied from reviewed commit `94aad2d6af8cea394ad2d2b311cf97fdb8bfbf05`.
- Modify `tests/contracts/ci.rs`: enforce oracle provenance/content and the repository-owned quality bootstrap.
- Modify `.github/workflows/ci.yml`: replace the broken second checkout with manifest verification and a frozen local build.
- Modify `.gitignore`: exclude only oracle dependency and generated-output directories.
- Modify `tests/contracts/release.rs`: forbid both retired-repository coordinates and the repository oracle path from release automation.
- Modify `tests/contracts/repository.rs`: enforce the canonical GitHub/project identity.
- Modify `Cargo.toml`, `README.md`, and `README.zh-CN.md`: publish the canonical repository/name without changing version 0.12.0 claims.
- Modify five `specs/0.11/implementation/*.md` evidence files: update only renamed GitHub run links.
- Modify `docs/project/release-checklist.md` and `docs/zh-CN/project/release-checklist.md`: distinguish quality-oracle verification from self-contained release verification.
- Modify `specs/0.12/implementation/00-master-control.md`, `10-performance-ci-task.md`, `10-performance-ci-acceptance.md`, and `99-final-acceptance.md`: record the accepted blocker closure and retain the exact-SHA ten-job boundary.
- Use `target/acceptance/v0.12/final/` only for ignored local evidence; do not commit dynamic run identifiers or benchmark output.

### Task 1: Establish failing oracle and quality-bootstrap contracts

**Files:**
- Modify: `tests/contracts/ci.rs`

- [ ] **Step 1: Add the immutable-oracle contract before the snapshot exists**

Add `typescript_oracle_should_be_an_immutable_repository_fixture` with assertions for:

```rust
let manifest = read("tests/oracles/typescript/package.json");
for required in [
    "\"name\": \"calckernel\"",
    "\"version\": \"0.8.0\"",
    "\"packageManager\": \"pnpm@9.15.9\"",
    "\"main\": \"./dist/src/index.js\"",
    "\"ckc\": \"./dist/src/cli.js\"",
    "\"wabt\": \"^1.0.39\"",
] {
    assert!(manifest.contains(required));
}
let provenance = read("tests/oracles/typescript/PROVENANCE.md");
for required in [
    "luxine/CalcKernel_retire",
    "5e989939d89d75056e5f3bea25f3bf7204d5529a",
    "445743ef4d270ba7a26a5402243ce0bb606fb44b",
    "SOURCE_MANIFEST.sha256",
] {
    assert!(provenance.contains(required));
}
assert_eq!(read("tests/oracles/typescript/SOURCE_MANIFEST.sha256").lines().count(), 85);
```

Retain the reviewed package-lock integrity assertions and required source/fixture path assertions from CK 0.13 commit `94aad2d` so the test validates content rather than directory presence alone.

- [ ] **Step 2: Run the focused contract and observe the intended RED**

Run:

```bash
cargo test --locked --test contracts typescript_oracle_should_be_an_immutable_repository_fixture -- --nocapture
```

Expected: FAIL while reading `tests/oracles/typescript/package.json`; this proves the new contract detects the missing repository-owned snapshot.

- [ ] **Step 3: Change the existing quality contract before changing YAML**

Require these exact fragments in `daily_ci_should_keep_fast_quality_independent_of_llvm`:

```rust
"corepack prepare pnpm@9.15.9 --activate",
"pnpm install --frozen-lockfile --ignore-scripts",
"working-directory: tests/oracles/typescript",
"sha256sum --check SOURCE_MANIFEST.sha256",
"pnpm build",
```

Require `CALCKERNEL_TS_ROOT: ${{ github.workspace }}/tests/oracles/typescript` and forbid both `repository: luxine/CalcKernel_retire` and the retired commit checkout from the quality section.

- [ ] **Step 4: Run the quality contract and observe the intended RED**

Run:

```bash
cargo test --locked --test contracts daily_ci_should_keep_fast_quality_independent_of_llvm -- --nocapture
```

Expected: FAIL because the workflow still uses `/typescript-oracle`, a second checkout, and a script-enabled install.

- [ ] **Step 5: Preserve RED evidence**

Save command output under ignored `target/acceptance/v0.12/final/oracle-contract-red.log` and `quality-bootstrap-red.log`; do not add those files to Git.

### Task 2: Materialize and verify the reviewed source snapshot

**Files:**
- Create: `tests/oracles/typescript/**`
- Modify: `.gitignore`

- [ ] **Step 1: Copy only committed snapshot bytes from the reviewed CK 0.13 candidate**

Run:

```bash
git archive 94aad2d6af8cea394ad2d2b311cf97fdb8bfbf05 tests/oracles/typescript | tar -x
```

This copies source/configuration/fixtures/provenance/manifest but excludes ignored `dist/` and `node_modules/` generated in another worktree.

- [ ] **Step 2: Ignore only generated oracle state**

Append exactly:

```gitignore
/tests/oracles/typescript/node_modules/
/tests/oracles/typescript/dist/
```

- [ ] **Step 3: Verify identity and every included byte**

Run:

```bash
test "$(wc -l < tests/oracles/typescript/SOURCE_MANIFEST.sha256 | tr -d ' ')" = 85
cmp tests/oracles/typescript/PROVENANCE.md \
  <(git show 94aad2d6af8cea394ad2d2b311cf97fdb8bfbf05:tests/oracles/typescript/PROVENANCE.md)
cmp tests/oracles/typescript/SOURCE_MANIFEST.sha256 \
  <(git show 94aad2d6af8cea394ad2d2b311cf97fdb8bfbf05:tests/oracles/typescript/SOURCE_MANIFEST.sha256)
(cd tests/oracles/typescript && shasum -a 256 -c SOURCE_MANIFEST.sha256)
```

The manifest command must report all 85 paths as `OK`. Compare copied paths and bytes against `git archive 94aad2d` if any manifest entry differs; never edit oracle source to repair a mismatch.

- [ ] **Step 4: Run the immutable-oracle contract GREEN**

Run the Task 1 focused oracle contract again. Expected: PASS.

- [ ] **Step 5: Commit the frozen fixture boundary**

```bash
git add .gitignore tests/contracts/ci.rs tests/oracles/typescript
git commit -m "test(v0.12): freeze repository-owned TypeScript oracle"
```

### Task 3: Repair quality CI and preserve release isolation

**Files:**
- Modify: `.github/workflows/ci.yml`
- Modify: `tests/contracts/release.rs`
- Modify: `docs/project/release-checklist.md`
- Modify: `docs/zh-CN/project/release-checklist.md`

- [ ] **Step 1: Replace only the quality oracle bootstrap**

Use this YAML sequence after the candidate checkout:

```yaml
env:
  CALCKERNEL_TS_ROOT: ${{ github.workspace }}/tests/oracles/typescript
steps:
  - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262
  - uses: actions/setup-node@49933ea5288caeca8642d1e84afbd3f7d6820020
    with:
      node-version: 20.19.5
  - run: corepack enable
  - run: corepack prepare pnpm@9.15.9 --activate
  - run: sha256sum --check SOURCE_MANIFEST.sha256
    working-directory: tests/oracles/typescript
  - run: pnpm install --frozen-lockfile --ignore-scripts
    working-directory: tests/oracles/typescript
  - run: pnpm build
    working-directory: tests/oracles/typescript
```

Do not modify the remaining quality commands, six-host matrix, two performance jobs, schema-7 commands, thresholds, or diagnostic behavior.

- [ ] **Step 2: Keep release automation test-only-oracle free**

Add these forbidden strings to the native release workflow contract:

```rust
"repository: luxine/CalcKernel_retire",
"tests/oracles/typescript",
```

- [ ] **Step 3: Update both release checklists**

State that quality verifies the repository-owned oracle provenance/manifest and executes all live differential gates, while release verification remains self-contained and does not consume the test-only oracle.

- [ ] **Step 4: Run focused contracts GREEN**

```bash
cargo test --locked --test contracts daily_ci_should_keep_fast_quality_independent_of_llvm -- --nocapture
cargo test --locked --test contracts native_release_workflow_should_build_sign_and_archive_native_ckc_artifacts -- --nocapture
```

Expected: both PASS and the daily workflow still reports exactly ten required jobs through its existing matrix assertions.

- [ ] **Step 5: Commit the CI closure**

```bash
git add .github/workflows/ci.yml tests/contracts/release.rs docs/project/release-checklist.md docs/zh-CN/project/release-checklist.md
git commit -m "ci(v0.12): build the frozen oracle in quality"
```

### Task 4: Canonicalize repository identity with a failing contract first

**Files:**
- Modify: `tests/contracts/repository.rs`
- Modify: `Cargo.toml`
- Modify: `README.md`
- Modify: `README.zh-CN.md`
- Modify: `specs/0.11/implementation/11-interrupt-handoff-plan.md`
- Modify: `specs/0.11/implementation/11-release-candidate-acceptance.md`
- Modify: `specs/0.11/implementation/11-runtime-replay-plan.md`
- Modify: `specs/0.11/implementation/11-windows-static-link-plan.md`
- Modify: `specs/0.11/implementation/99-final-acceptance.md`

- [ ] **Step 1: Add the repository identity contract**

Require `repository = "https://github.com/luxine/CalcKernel"`, require both READMEs to start with `# CalcKernel`, reject `Rust CalcKernel` in current READMEs, and reject `https://github.com/luxine/Rust_CalcKernel` in the workflow and the five listed historical evidence documents.

- [ ] **Step 2: Run the identity contract RED**

```bash
cargo test --locked --test contracts repository_identity_should_use_the_canonical_github_name -- --nocapture
```

Expected: FAIL on missing Cargo repository metadata or the former README title.

- [ ] **Step 3: Apply the canonical identity without changing 0.12 claims**

Set the Cargo repository field, use `# CalcKernel`, describe it as a Rust-implemented compiler, and replace only the repository portion of old Actions URLs with `https://github.com/luxine/CalcKernel`. Keep all run/job identifiers and all 0.12.0 version text unchanged.

- [ ] **Step 4: Run the identity contract GREEN and scan the tree**

```bash
cargo test --locked --test contracts repository_identity_should_use_the_canonical_github_name -- --nocapture
! rg -n "https://github\.com/luxine/Rust_CalcKernel|Rust CalcKernel" README.md README.zh-CN.md specs/0.11/implementation
```

Expected: PASS and no matches.

- [ ] **Step 5: Commit identity maintenance**

```bash
git add Cargo.toml README.md README.zh-CN.md tests/contracts/repository.rs specs/0.11/implementation
git commit -m "docs: adopt canonical CalcKernel repository identity"
```

### Task 5: Record the blocker closure in the frozen 0.12 execution contract

**Files:**
- Modify: `specs/0.12/implementation/00-master-control.md`
- Modify: `specs/0.12/implementation/10-performance-ci-task.md`
- Modify: `specs/0.12/implementation/10-performance-ci-acceptance.md`
- Modify: `specs/0.12/implementation/99-final-acceptance.md`

- [ ] **Step 1: Add the accepted closure to master control**

Reference `specs/0.12/review/implementation-blocker-03.md`, the repository rename collision, the source/fixture snapshot, lockfile, provenance, manifest, and the unchanged language/ABI/performance/ten-job boundaries.

- [ ] **Step 2: Make Stage 10 task and acceptance explicit**

Require quality to verify `SOURCE_MANIFEST.sha256`, use a frozen script-disabled install, build locally, and execute unchanged C/WASM/CLI/fixture differential tests. State that private repository access, registry substitution, and skipped oracle tests are not valid acceptance.

- [ ] **Step 3: Extend final acceptance without signing it**

Add an unchecked item requiring the same repository-owned oracle sequence. Do not check any final item and do not write a run ID or candidate SHA into tracked documentation.

- [ ] **Step 4: Validate documentation contracts and formatting**

```bash
cargo test --locked --test contracts docs_ -- --nocapture
git diff --check
```

Expected: PASS; no normative 0.12 specification file changes.

- [ ] **Step 5: Commit execution-contract maintenance**

```bash
git add specs/0.12/implementation
git commit -m "docs(v0.12): record repository oracle acceptance closure"
```

### Task 6: Build and exercise the live oracle locally

**Files:**
- Generated and ignored: `tests/oracles/typescript/node_modules/**`
- Generated and ignored: `tests/oracles/typescript/dist/**`
- Evidence and ignored: `target/acceptance/v0.12/final/**`

- [ ] **Step 1: Verify before installing**

```bash
(cd tests/oracles/typescript && shasum -a 256 -c SOURCE_MANIFEST.sha256)
```

Expected: exactly 85 successful entries.

- [ ] **Step 2: Install and build from frozen inputs**

```bash
corepack enable
corepack prepare pnpm@9.15.9 --activate
pnpm --dir tests/oracles/typescript install --frozen-lockfile --ignore-scripts
pnpm --dir tests/oracles/typescript build
test -f tests/oracles/typescript/dist/src/cli.js
```

- [ ] **Step 3: Run every live oracle-dependent suite**

```bash
CALCKERNEL_TS_ROOT="$PWD/tests/oracles/typescript" cargo test --locked --test backend c_backend_should_preserve_typescript_oracle -- --nocapture
CALCKERNEL_TS_ROOT="$PWD/tests/oracles/typescript" cargo test --locked --test backend wasm_cli_should_match_typescript_oracle -- --nocapture
CALCKERNEL_TS_ROOT="$PWD/tests/oracles/typescript" cargo test --locked --test cli typescript_oracle_verifier -- --nocapture
CALCKERNEL_TS_ROOT="$PWD/tests/oracles/typescript" cargo test --locked --test performance typescript_oracle_fixtures_should_be_covered -- --nocapture
```

Expected: every selected live differential/readiness/coverage test executes and passes; no test may pass by an unset-root early return.

- [ ] **Step 4: Confirm generated files remain ignored**

```bash
git status --short --ignored tests/oracles/typescript | sed -n '1,160p'
git check-ignore tests/oracles/typescript/node_modules tests/oracles/typescript/dist
```

Expected: only `node_modules/` and `dist/` are ignored; source and manifest files are tracked.

### Task 7: Run the complete CK 0.12 local acceptance gate

**Files:**
- Evidence and ignored: `target/acceptance/v0.12/final/**`

- [ ] **Step 1: Resolve the pinned local toolchain**

Use the retained v0.10 worktree prefixes:

```bash
export CKC_LLVM_PREFIX=/Users/lynn/code/Rust_CalcKernel/.worktrees/native-toolchain-0.10/build/llvm/prefix-aarch64-apple-darwin11-release
export CKC_CLANG_ORACLE=/Users/lynn/code/Rust_CalcKernel/.worktrees/native-toolchain-0.10/build/llvm/prefix-aarch64-apple-darwin11-oracle/bin/clang
export CALCKERNEL_TS_ROOT="$PWD/tests/oracles/typescript"
```

Verify `bin/llvm-config`, `bin/clang`, and `dist/src/cli.js` exist before tests.

- [ ] **Step 2: Run Stage 10 functional, schema, and Python gates**

```bash
cargo test --locked --test performance -- --nocapture
python3 -m unittest discover -s tests/performance -p '*_test.py'
cargo test --locked --test contracts ci_ -- --nocapture
```

- [ ] **Step 3: Run full Rust quality gates**

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --locked
cargo test --all-features --locked
cargo build --release --features native-toolchain --locked
```

- [ ] **Step 4: Run security and artifact audits**

```bash
cargo test --all-features --locked --test native artifacts
scripts/test-sanitized-ownership.sh
scripts/audit-ckc-release.sh target/release/ckc
scripts/audit-native-artifact.sh target/native-acceptance
scripts/audit-jit-memory.sh target/release/ckc
git diff --check
```

On Darwin the sanitizer command must report the frozen Linux-only contract and exit zero; it is not evidence for the required Linux CI sanitizer execution.

- [ ] **Step 5: Verify the candidate tree**

```bash
git status --short
git diff --check HEAD
git ls-files tests/oracles/typescript/dist tests/oracles/typescript/node_modules
```

Expected: clean tracked tree, no generated oracle paths tracked, and all local gates green. If a gate fails, diagnose and fix the defect without changing thresholds, corpus, or required-job topology, then rerun every affected gate.

### Task 8: Publish one exact-SHA candidate and hand remote acceptance to the low-frequency monitor

**Files:**
- No tracked file changes after the final candidate commit.

- [ ] **Step 1: Create a final closure commit only if verification required tracked fixes**

```bash
git status --short
git diff --check
```

If tracked fixes remain, commit them with a precise message and rerun Task 7. Otherwise retain the current HEAD as the candidate.

- [ ] **Step 2: Push the feature branch**

```bash
git push origin feature/v0.12-vector-optimizer
```

- [ ] **Step 3: Dispatch exactly one workflow for the pushed branch**

```bash
gh workflow run ci.yml --ref feature/v0.12-vector-optimizer
```

Resolve the new run once, verify its `headSha` equals `git rev-parse HEAD`, and record the run ID/URL only in ignored acceptance evidence and the active heartbeat prompt.

- [ ] **Step 4: Keep unfinished remote work out of the foreground**

Update the existing 15-minute `CK v0.12 / v0.13 验收合并` heartbeat with the new v0.12 SHA/run. It must remain quiet while jobs are merely queued/running, report failures without lowering gates, and merge v0.12 then v0.13 only after each exact-SHA ten-job acceptance is genuinely complete.

- [ ] **Step 5: Preserve merge and cleanup boundaries**

Do not create a tag, Release, or PR. Do not merge or remove the v0.12 worktree until its exact-SHA ten-job workflow is fully green. Retain the v0.10 worktree while any remaining candidate depends on its pinned local toolchain.

## Self-review

- Spec coverage: Tasks 1–3 close provenance, manifest, build, differential, and release-isolation requirements; Task 4 closes canonical repository identity; Task 5 preserves frozen acceptance; Tasks 6–8 provide local and exact-SHA remote evidence.
- Scope: No CK specification, ABI, optimizer, benchmark corpus, threshold, schema-7 rule, host matrix, or job requirement is changed.
- TDD: Missing snapshot, stale workflow, and stale identity each receive an observed focused failure before implementation.
- Type/path consistency: All contracts use existing `read`/`repo_root` helpers; every oracle path is `tests/oracles/typescript`; all runtime consumers receive the same `CALCKERNEL_TS_ROOT`.
- Generated state: `dist/`, `node_modules/`, acceptance logs, LLVM prefixes, and performance output remain ignored.
- Remote boundary: one pushed candidate receives one workflow; tracked changes after dispatch invalidate the run and require a new complete attempt.
