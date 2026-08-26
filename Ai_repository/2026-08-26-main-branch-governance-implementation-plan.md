# Main Branch Governance and Continuous Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add required pull request/push CI and protect GitHub's `main` branch for ordinary contributors while preserving administrator direct-push access.

**Architecture:** A new focused GitHub Actions workflow exposes one stable `quality` check and reuses the repository's existing Rust quality contract. A Rust repository-contract test locks down the workflow surface. After the workflow succeeds on the remote `main` commit, classic GitHub branch protection requires that exact check, one current approval, and resolved conversations for non-administrators.

**Tech Stack:** Rust integration tests, GitHub Actions YAML, GitHub CLI and REST API, Git branch protection.

**Execution workspace:** Before Task 1, use `superpowers:using-git-worktrees` to create an isolated `codex/main-governance` worktree from the current `main`. Do not push the implementation branch; Task 4 fast-forwards the primary `main` worktree and pushes `main` once the local gate passes.

---

### Task 1: Add a failing CI repository-contract test

**Files:**
- Create: `tests/ci_surface_test.rs`
- Reference: `tests/release_surface_test.rs`
- Reference: `Ai_repository/2026-08-26-main-branch-governance-design.md`

- [ ] **Step 1: Verify that the daily CI workflow is absent**

Run:

```bash
test ! -e .github/workflows/ci.yml
```

Expected: exit 0, proving the test will describe a missing repository surface.

- [ ] **Step 2: Write the failing repository-contract test**

Create `tests/ci_surface_test.rs` with exactly:

```rust
use std::{fs, path::PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

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
        "components: rustfmt, clippy",
        "cargo fmt --check",
        "cargo clippy --all-targets --all-features --locked -- -D warnings",
        "cargo test --locked",
        "cargo build --release --locked",
        "./target/release/ckc --help",
        "./target/release/ckc check examples/scalar.ck",
        "./target/release/ckc emit-mir examples/scalar.ck -O3",
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
```

- [ ] **Step 3: Run the focused test and verify RED**

Run:

```bash
cargo test --test ci_surface_test -- --nocapture
```

Expected: FAIL because `.github/workflows/ci.yml` cannot be read. The failure must mention the missing workflow path; a compilation error is not an acceptable RED result.

### Task 2: Add the daily CI workflow

**Files:**
- Create: `.github/workflows/ci.yml`
- Test: `tests/ci_surface_test.rs`
- Reference: `.github/workflows/native-release.yml`

- [ ] **Step 1: Create the minimal workflow that satisfies the approved contract**

Create `.github/workflows/ci.yml` with exactly:

```yaml
name: CI

on:
  pull_request:
    branches: [main]
  push:
    branches: [main]

permissions:
  contents: read

concurrency:
  group: ci-${{ github.workflow }}-${{ github.event.pull_request.number || github.ref }}
  cancel-in-progress: true

defaults:
  run:
    shell: bash

env:
  CARGO_TERM_COLOR: always

jobs:
  quality:
    name: quality
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - run: cargo fmt --check
      - run: cargo clippy --all-targets --all-features --locked -- -D warnings
      - run: cargo test --locked
      - run: cargo build --release --locked
      - run: ./target/release/ckc --help
      - run: ./target/release/ckc check examples/scalar.ck
      - run: ./target/release/ckc emit-mir examples/scalar.ck -O3
```

- [ ] **Step 2: Run the focused test and verify GREEN**

Run:

```bash
cargo test --test ci_surface_test -- --nocapture
```

Expected: PASS with `1 passed; 0 failed`.

- [ ] **Step 3: Run formatting on the new Rust test**

Run:

```bash
cargo fmt --check
```

Expected: exit 0. If it fails, run `cargo fmt`, inspect the change, and rerun `cargo fmt --check`.

- [ ] **Step 4: Verify the existing release workflow remains unchanged**

Run:

```bash
git diff --exit-code HEAD -- .github/workflows/native-release.yml
```

Expected: exit 0 with no diff.

- [ ] **Step 5: Commit the test and workflow together**

Run:

```bash
git add tests/ci_surface_test.rs .github/workflows/ci.yml
git commit -m "ci: validate pull requests and main pushes"
```

Expected: one commit containing only the new test and workflow.

### Task 3: Run the complete local quality gate

**Files:**
- Verify: entire Rust workspace
- Verify: `.github/workflows/ci.yml`
- Verify: `.github/workflows/native-release.yml`

- [ ] **Step 1: Run the strict Rust quality gate**

Run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --locked
cargo build --release --locked
```

Expected: all commands exit 0; the test count increases from 114 to 115.

- [ ] **Step 2: Run the same release-binary smoke checks used by CI**

Run:

```bash
./target/release/ckc --help >/dev/null
./target/release/ckc check examples/scalar.ck
./target/release/ckc emit-mir examples/scalar.ck -O3 >/dev/null
```

Expected: exit 0 and `OK: examples/scalar.ck` from the check command.

- [ ] **Step 3: Verify the implementation commit is scoped**

Run:

```bash
git status --short
git show --stat --oneline HEAD
git diff HEAD^ -- .github/workflows/ci.yml tests/ci_surface_test.rs
```

Expected: clean worktree; the latest commit changes only the two approved files.

### Task 4: Fast-forward `main`, push, and validate the first remote CI run

**Files:**
- Remote workflow: `.github/workflows/ci.yml`
- Remote branch: `origin/main`

- [ ] **Step 1: Fast-forward the primary `main` worktree to the implementation branch**

From the primary repository worktree, run:

```bash
git status --short
git merge --ff-only codex/main-governance
```

Expected: the primary worktree is clean before the merge and `main` fast-forwards without a merge commit.

- [ ] **Step 2: Review every commit that will be pushed**

Run:

```bash
git log --oneline --decorate origin/main..main
git diff --stat origin/main..main
```

Expected: the approved design commit, implementation-plan commit, and CI implementation commit are present; no unrelated files appear.

- [ ] **Step 3: Push `main` while it is still unprotected**

Run:

```bash
git push origin main
```

Expected: `origin/main` advances to the local `main` commit.

- [ ] **Step 4: Locate the CI run for the exact pushed commit**

Run:

```bash
HEAD_SHA="$(git rev-parse HEAD)"
gh run list \
  --repo luxine/Rust_CalcKernel \
  --workflow ci.yml \
  --branch main \
  --limit 10 \
  --json databaseId,headSha,status,conclusion,url \
  --jq ".[] | select(.headSha == \"${HEAD_SHA}\")"
```

Expected: one run whose `headSha` equals `HEAD_SHA`. If the list is initially empty, poll again after GitHub registers the push; do not use a run for a different commit.

- [ ] **Step 5: Wait for the exact CI run to succeed**

Set `RUN_ID` to the matching run's `databaseId`, then run:

```bash
gh run watch "$RUN_ID" --repo luxine/Rust_CalcKernel --exit-status
```

Expected: exit 0 with the `quality` job completed successfully. If it fails, stop before Task 5 and repair CI on the unprotected branch.

- [ ] **Step 6: Query the exact required check-run context**

Run:

```bash
HEAD_SHA="$(git rev-parse HEAD)"
gh api \
  -H "Accept: application/vnd.github+json" \
  "repos/luxine/Rust_CalcKernel/commits/${HEAD_SHA}/check-runs" \
  --jq '.check_runs[] | [.name, .status, .conclusion, .app.slug] | @tsv'
```

Expected: a `quality` check run with `completed`, `success`, and `github-actions`. Use the API-reported name, not a guessed UI label, in Task 5.

### Task 5: Apply classic branch protection

**Files:**
- Remote GitHub setting: `main` branch protection
- No local files

- [ ] **Step 1: Confirm the current rule is still absent**

Run:

```bash
gh api repos/luxine/Rust_CalcKernel/branches/main/protection
```

Expected before configuration: HTTP 404 `Branch not protected`. If a rule now exists, stop and compare it with the approved design before overwriting anything.

- [ ] **Step 2: Apply the approved protection rule**

Run:

```bash
gh api \
  --method PUT \
  -H "Accept: application/vnd.github+json" \
  -H "X-GitHub-Api-Version: 2026-03-10" \
  repos/luxine/Rust_CalcKernel/branches/main/protection \
  --input - <<'JSON'
{
  "required_status_checks": {
    "strict": true,
    "contexts": ["quality"]
  },
  "enforce_admins": false,
  "required_pull_request_reviews": {
    "dismiss_stale_reviews": true,
    "require_code_owner_reviews": false,
    "required_approving_review_count": 1,
    "require_last_push_approval": false
  },
  "restrictions": null,
  "required_linear_history": false,
  "allow_force_pushes": false,
  "allow_deletions": false,
  "block_creations": false,
  "required_conversation_resolution": true,
  "lock_branch": false,
  "allow_fork_syncing": true
}
JSON
```

Expected: HTTP 200 response containing the configured protection rule. If the successful check-run name from Task 4 is not `quality`, substitute only that exact name in `contexts`.

### Task 6: Verify remote governance end to end

**Files:**
- Remote GitHub setting: `main` branch protection
- Remote GitHub workflow: `CI`
- No local files

- [ ] **Step 1: Assert every material protection field through the API**

Run:

```bash
gh api repos/luxine/Rust_CalcKernel/branches/main/protection | jq -e '
  .required_status_checks.strict == true and
  ((.required_status_checks.contexts // []) | index("quality") != null) and
  .enforce_admins.enabled == false and
  .required_pull_request_reviews.dismiss_stale_reviews == true and
  .required_pull_request_reviews.require_code_owner_reviews == false and
  .required_pull_request_reviews.required_approving_review_count == 1 and
  .required_conversation_resolution.enabled == true and
  .allow_force_pushes.enabled == false and
  .allow_deletions.enabled == false and
  .required_linear_history.enabled == false and
  .lock_branch.enabled == false
'
```

Expected: `jq` exits 0. If Task 4 discovered a context name other than `quality`, use that exact name in this assertion.

- [ ] **Step 2: Verify GitHub's high-level branch-protection view**

Run:

```bash
gh api graphql -f query='query {
  repository(owner: "luxine", name: "Rust_CalcKernel") {
    viewerPermission
    branchProtectionRules(first: 20) {
      nodes {
        pattern
        requiresApprovingReviews
        requiredApprovingReviewCount
        dismissesStaleReviews
        requiresStatusChecks
        requiresStrictStatusChecks
        requiresConversationResolution
        isAdminEnforced
        allowsForcePushes
        allowsDeletions
      }
    }
  }
}'
```

Expected: `viewerPermission` is `ADMIN`; the `main` rule requires one approval, strict status checks, and conversation resolution; admin enforcement, force pushes, and deletion are false.

- [ ] **Step 3: Verify CI and release workflows remain separately scoped**

Run:

```bash
gh workflow view ci.yml --repo luxine/Rust_CalcKernel --yaml
gh workflow view native-release.yml --repo luxine/Rust_CalcKernel --yaml
```

Expected: `CI` triggers only for pull requests/main pushes; `native ckc release` still triggers only for `v*` tags and manual dispatch.

- [ ] **Step 4: Record the final evidence**

Capture in the task handoff:

- Pushed `main` commit SHA.
- Successful CI run URL.
- Required check-run context.
- Branch-protection API verification result.
- Confirmation that `enforce_admins.enabled` is false.
- Local quality-gate result and final test count.

No additional commit is needed because Task 6 changes only remote settings.
