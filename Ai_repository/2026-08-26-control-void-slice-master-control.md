# CK Control Flow, Void, and Slice Master Control Plan

Status: ready for execution after plan self-review on 2026-08-26

Branch: `feature/control-void-slice`

Worktree: `/Users/lynn/code/Rust_CalcKernel/.worktrees/control-void-slice`

Baseline: `0799a4f34cb1906084da8e558b28122acd1fb933`

## Objective and authority

Implement the approved control-flow, void, and slice design without changing its
source semantics or promised backend matrix. Work is performed inline in this
worktree, without subagents. Planning documents are committed before production
code. The completed branch is committed but not merged into `main`.

Sources of truth, in descending order:

1. `Ai_repository/2026-08-26-control-flow-void-slice-design.md`
2. `Ai_repository/2026-08-26-control-flow-void-slice-adversarial-review.md`
3. this master control plan
4. the phase execution and acceptance documents listed below

When an implementation detail is absent from the design, the phase plan may make
a deterministic local choice that preserves all observable semantics. A genuine
design contradiction or infeasible contract is recorded here before the design
and affected phase documents are amended. Acceptance thresholds may not be
weakened to accommodate an implementation failure.

## Document set

| Role | Document |
| --- | --- |
| Phase A execution | `Ai_repository/2026-08-26-control-flow-void-slice-phase-a-execution.md` |
| Phase A acceptance | `Ai_repository/2026-08-26-control-flow-void-slice-phase-a-acceptance.md` |
| Phase B execution | `Ai_repository/2026-08-26-control-flow-void-slice-phase-b-execution.md` |
| Phase B acceptance | `Ai_repository/2026-08-26-control-flow-void-slice-phase-b-acceptance.md` |
| Phase C execution | `Ai_repository/2026-08-26-control-flow-void-slice-phase-c-execution.md` |
| Phase C acceptance | `Ai_repository/2026-08-26-control-flow-void-slice-phase-c-acceptance.md` |
| Final acceptance | `Ai_repository/2026-08-26-control-flow-void-slice-final-acceptance.md` |

## Controlled phase state machine

```text
planning committed
  -> Phase A red tests -> implementation -> Phase A acceptance -> phase commit
  -> Phase B red tests -> implementation -> Phase B acceptance -> phase commit
  -> Phase C red tests -> implementation -> Phase C acceptance -> phase commit
  -> final acceptance -> genuine repairs only -> final commit -> wait for review
```

No Phase B production change starts until Phase A acceptance is fully green. No
Phase C production change starts until Phase B acceptance is fully green. A
phase may add its red tests before production edits, but it may not alter tests
to hide a defect discovered during green implementation.

## Development rules

- Use red-green-refactor for every behavior slice. Run the named focused test and
  observe the expected failure before the corresponding production edit.
- Keep source concepts explicit in typed AST and MIR. Do not desugar slices into
  specially named scalar locals before backend emission.
- Preserve existing TypeScript-oracle fixtures unchanged. New syntax uses direct
  semantic and runtime assertions because the oracle cannot parse it.
- Add explicit diagnostic-code calls for new checker categories; do not extend
  the message-prefix router for the new features.
- Audit every exhaustive AST, type, MIR instruction, place, value, and terminator
  match when a variant changes. Compiler panics are acceptance failures.
- Run O0, O1, O2, and O3 for new control-flow and slice runtime fixtures.
- Update formal English and Simplified Chinese documents together.
- Keep generated artifacts and temporary runtime outputs outside the repository.
- Commit only related work. Never merge, rebase onto, or push `main` as part of
  this task.

## Repository-aligned change map

The repository is a compact single-crate compiler. Feature work stays in its
existing layers:

| Layer | Existing file | Planned responsibility |
| --- | --- | --- |
| Lexing | `src/lexer/mod.rs` | reserved words and `DotDot` |
| Parsing / AST | `src/parser.rs` | statements, void type, slice/range expressions |
| Diagnostics | `src/diagnostics.rs` | stable new checker codes |
| Checking | `src/typeck.rs` | flow summary and void/slice rules |
| MIR | `src/mir/mod.rs` | optional values, slice operations/places, validation |
| Optimization | `src/opt/mod.rs` | new use-def edges and bounds observability |
| Backends | `src/backend/mod.rs` | C, WASM, LLVM physical lowering |
| CLI | `src/main.rs` | bounds modes and error precedence |
| Public exports | `src/lib.rs` | existing module re-exports remain sufficient |
| Tests | `tests/*.rs` | focused syntax, MIR, optimizer, backend, CLI/runtime gates |
| Examples | `examples/*.ck` | additive feature demonstrations |
| Contracts | `README*`, `docs/`, `docs/zh-CN/` | bilingual language/ABI/backend documentation |

No new compiler layer or general module-system/ownership refactor is planned.
Focused private helpers may be extracted inside the existing source modules.

## Diagnostic allocation

Existing `CK0001`, `CK1001`, and `CK2001`–`CK2008` retain their meanings.
New checker categories use explicit codes:

| Code | Category |
| --- | --- |
| `CK2009` | invalid loop control placement |
| `CK2010` | unreachable statement |
| `CK2011` | void position, return, or call misuse |
| `CK2012` | slice type or operation misuse |

Parser recovery continues to use `CK1001`. General pre-existing type mismatches
continue to use their current code. The checker gains an `error_with_code`-style
entry point so these allocations do not depend on message text.

## Adversarial-risk closure matrix

| Review risk | Required execution task | Acceptance owner |
| --- | --- | --- |
| R1 range/float lexing | C1 | Phase C lexer/parser gate |
| R2 flow/checker-lowering split | A2–A3 | Phase A flow/MIR gate |
| R3 void synthetic values | B2–B4 | Phase B MIR/backend gate |
| R4 WASM compound slice values | C7 | Phase C WASM gate |
| R5 checked place preludes | C6 | Phase C checked-C gate |
| R6 bounds-observable optimizer | C4 | Phase C O0–O3 optimizer gate |
| R7 C declaration/name allocation | C5 | Phase C C golden/runtime gate |
| R8 oracle boundary | A5, B6, C10 | every phase regression gate |

## Commit checkpoints

1. Planning commit: design review plus all control/execution/acceptance documents.
2. Phase A commit after its acceptance document is satisfied.
3. Phase B commit after its acceptance document is satisfied.
4. Phase C commit after its acceptance document is satisfied.
5. Final repair/documentation commit only if final acceptance reveals genuine
   issues not already captured by a phase.

Acceptance documents are living evidence: commands and outcomes are recorded in
them after execution. Updating evidence is not a lowering of a threshold.

## Blocker and amendment protocol

A blocker is one of:

- two approved requirements prescribe incompatible observable behavior;
- the current target/toolchain cannot represent a required ABI or runtime result;
- satisfying a requirement would require work expressly excluded by the design;
- a required acceptance command cannot run for an external reason after local
  repository and toolchain causes have been ruled out.

For a blocker:

1. reproduce and record exact evidence;
2. re-check the design and current code path to rule out a plan/implementation
   error;
3. amend the design only when the contract itself is genuinely wrong;
4. update every affected execution and acceptance reference;
5. repeat adversarial review of the amendment before resuming.

Ordinary compiler errors, failing red tests, implementation complexity, and a
need for local refactoring are not blockers.

## Plan self-review record

The decomposition was reviewed against the baseline and approved design before
the planning commit.

| Check | Result |
| --- | --- |
| Every design goal maps to a phase task and acceptance gate | pass |
| R1–R8 each have an explicit owner | pass |
| Phase order matches current AST/MIR/backend dependencies | pass |
| C, WASM, and LLVM promises are not conflated | pass |
| checked bounds remains C-only and status-ABI-wide | pass |
| legacy oracle tests remain regression-only | pass |
| bilingual durable-document updates are scheduled | pass |
| no task requires a subagent or merge to `main` | pass |

Self-review found and corrected four initial plan defects before this version:
the optimizer context now explicitly carries bounds mode; the C checked-place
task requires a prelude/expression lowering API rather than attempting checks in
the existing string-only `c_place` helper; the oracle-risk mapping points to the
actual documentation/fixture tasks; and migration plus WASM interop documentation
is explicitly included in Phase C.

## Execution log

Record completed checkpoints here without deleting failed-command evidence.

| Date | Checkpoint | Commit / evidence | Result |
| --- | --- | --- | --- |
| 2026-08-26 | independent worktree baseline | `0799a4f`; locked build and full tests | pass |
| 2026-08-26 | adversarial design review | review document | pass; no blockers |
