# Main Branch Governance and Continuous Integration Design

Status: approved in conversation on 2026-08-26

## Context

`luxine/Rust_CalcKernel` is a public GitHub repository whose default branch is
`main`. The repository currently has no branch protection rule or repository
ruleset. The repository owner has administrator access and wants to retain an
emergency direct-push path while requiring ordinary contributors to use reviewed,
passing pull requests.

The existing `.github/workflows/native-release.yml` workflow is intentionally a
release pipeline. It runs only for `v*` tags or manual dispatch, verifies the Rust
project, builds six native platform artifacts, and optionally publishes them to a
GitHub Release. It should not become the day-to-day pull request workflow.

## Goals

- Run the Rust quality gate for every pull request targeting `main` and every
  push to `main`.
- Require ordinary contributors to merge through pull requests.
- Require one approving review, current passing CI, and resolved conversations.
- Reject force pushes and deletion of `main`.
- Keep repository administrators outside enforcement so they retain emergency
  direct-push access.
- Preserve the existing native release workflow and its tag/manual triggers.
- Configure and then verify the remote GitHub rule rather than relying on local
  configuration alone.

## Non-Goals

- Changing CK / CalcKernel compiler behavior or release artifacts.
- Replacing the native release workflow.
- Adding code coverage, dependency auditing, artifact signing, or deployment.
- Introducing a repository-wide GitHub Ruleset when one protected branch is the
  current requirement.
- Reworking the repository's merge-method settings.

## Approaches Considered

### Separate CI workflow and classic branch protection

Add a focused pull request/push workflow and make its stable `quality` job a
required status check. Configure classic protection directly on `main`, leaving
administrator enforcement disabled. This is the selected approach because it is
small, explicit, easy to inspect through the GitHub API, and sufficient for one
protected branch.

### Repository Ruleset

A Ruleset could model administrator bypass actors and expand cleanly to multiple
branches. It is more configuration than this repository currently needs and would
make a single-branch policy harder to audit at a glance.

### Extend the native release workflow

Adding pull request and branch triggers to the release workflow would reduce the
number of YAML files, but would mix daily validation with six-platform artifact
production. Conditional logic would become harder to understand and accidental
runner usage would be more likely.

## Daily CI Workflow

Create `.github/workflows/ci.yml` with workflow name `CI` and a single job whose
stable display name is `quality`. The workflow triggers on:

- `pull_request` events whose base branch is `main`.
- `push` events whose branch is `main`.

The workflow uses read-only repository contents permission. A concurrency group
based on workflow and Git ref cancels an older in-progress run when a newer commit
arrives on the same pull request or branch.

The `quality` job runs on `ubuntu-24.04`, checks out the repository, installs the
stable Rust toolchain with rustfmt and Clippy, and executes the existing local
quality contract in this order:

1. `cargo fmt --check`
2. `cargo clippy --all-targets --all-features --locked -- -D warnings`
3. `cargo test --locked`
4. `cargo build --release --locked`
5. `./target/release/ckc --help`
6. `./target/release/ckc check examples/scalar.ck`
7. `./target/release/ckc emit-mir examples/scalar.ck -O3`

The job deliberately does not upload artifacts or run the release platform
matrix. The existing native release workflow remains unchanged.

## Repository Contract Test

Add a focused Rust integration test at `tests/ci_surface_test.rs`. It reads the
workflow as repository data and verifies the durable governance contract:

- The workflow is named `CI`.
- Pull requests targeting `main` and pushes to `main` are configured.
- The job exposes the stable `quality` check name.
- Permissions are read-only.
- Concurrency cancels stale runs.
- Every approved quality and smoke command is present.
- Release publication and artifact upload steps are absent.

The test is written and run before the workflow exists so its initial failure
proves that it detects the missing CI surface. After adding the workflow, the
focused test and full suite must pass.

## Main Branch Protection

Protection is configured only after the first pushed CI run succeeds. GitHub may
display the combined label as `CI / quality`, while the Check Runs API normally
reports the job name as `quality`; the exact API-reported name is queried before
it is made required. The protection rule has these semantics:

- Required status checks are strict, so the pull request branch must be current
  with `main`.
- The `quality` check is required.
- One approving review is required.
- Approvals are dismissed when new commits make them stale.
- Code-owner review is not required because the repository has no CODEOWNERS
  policy in scope.
- Pull request conversations must be resolved before merge.
- Force pushes and branch deletion are disabled.
- Push restrictions are not limited to a named team or application.
- Administrator enforcement is disabled, preserving administrator direct pushes.

For non-administrators, the review requirement makes pull requests mandatory.
Administrators may bypass the rule for emergency direct pushes or recovery.

## Delivery Sequence

1. Add the failing repository contract test and confirm the expected failure.
2. Add the daily CI workflow and make the focused test pass.
3. Run formatting, strict Clippy, the full test suite, and the release build.
4. Commit the CI change separately from this design commit.
5. Push the commit to `main` while the branch is still unprotected.
6. Wait for the first `CI` workflow run and require a successful `quality` check.
7. Query the successful commit's check runs to capture the exact context name.
8. Configure classic branch protection through the GitHub API.
9. Read the remote rule back and compare every material setting with this design.

GitHub API is the primary configuration path because it is exact and auditable.
The signed-in Edge browser is a fallback if the API cannot express or apply a
required field.

## Failure Handling and Recovery

- Do not protect `main` if the initial CI run fails. Diagnose and fix the workflow
  on the still-unprotected branch first.
- Do not guess the required status context. Query the successful check run and use
  its exact name.
- If remote protection differs from the intended design, update only the incorrect
  fields and read the rule back again.
- If protection blocks all expected paths, an administrator can temporarily amend
  or remove the protection through the GitHub API or repository settings because
  administrator enforcement remains disabled.
- Never weaken or replace the native release workflow as part of recovery.

## Success Criteria

- A pull request targeting `main` automatically runs `quality`.
- A push to `main` automatically runs `quality`.
- The local repository contract and complete Rust quality gate pass.
- GitHub reports `quality` as a strict required status check.
- Non-administrators need one current approval and resolved conversations.
- Force pushes and deletion of `main` are disabled.
- Administrators remain able to push directly to `main`.
- The native release workflow still runs only for `v*` tags or manual dispatch.
