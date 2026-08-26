# CK Control Flow, Void, and Slice Adversarial Design Review

Status: passed with no blocking findings on 2026-08-26

Reviewed design:
`Ai_repository/2026-08-26-control-flow-void-slice-design.md`

Reviewed baseline: `0799a4f34cb1906084da8e558b28122acd1fb933`

## Review standard

This review treats an issue as blocking only when it leaves source semantics,
evaluation order, ABI behavior, error precedence, or a required backend without a
coherent implementable answer. A large implementation surface, a necessary local
refactor, or a choice that can be made without changing promised semantics is not
by itself a blocker.

The review compared the design with the current lexer, parser, checker, typed AST,
MIR lowering and validation, optimizer, C emitter, WASM dispatcher/structured
loop emitter, LLVM emitter, CLI parser, runtime tests, and bilingual documentation
policy. The worktree baseline passed `cargo build --locked` and the complete
locked test suite before the review.

## Result

No blocking design findings remain. The three-phase order is coherent:

1. Phase A establishes structured reachability and loop-exit lowering.
2. Phase B makes call targets and return values optional before aggregate slice
   returns are introduced.
3. Phase C adds semantic descriptors, physical ABI expansion, and checked C
   bounds behavior after the required control-flow and status-return machinery is
   stable.

The design fixes the important externally observable decisions: source grammar,
type restrictions, evaluation order, aliasing, range preconditions, exported
return restriction, flattened parameter ABI, target representation, checked-mode
matrix, status codes, and error precedence. The current implementation does not
contain an architectural constraint that contradicts those decisions.

## Mandatory implementation risks

These are not blockers because the approved design already determines the
required behavior. They are mandatory planning and acceptance constraints.

### R1 — `..` must win over floating-point scanning

The current number scanner treats every dot after digits as the beginning of a
float and reports a malformed float when no digit follows it. Therefore
`items[0..len]` cannot be implemented by adding a `DotDot` token only. Lexing must
recognize `..` before the decimal-fraction path, while preserving existing errors
for `1.` and `.5`. Lexer tests must cover no-space and spaced ranges.

### R2 — reachability analysis and MIR lowering must share the same flow model

The checker currently uses a final-statement `block_definitely_returns` boolean,
while MIR lowering independently fails when a statement follows a terminated
block. Phase A must replace this split with the designed structured flow summary,
diagnose unreachable source statements before MIR, and lower `break`/`continue`
using an explicit innermost-loop target stack. Tests must cover both arms of
conditionals, nested blocks, nested loops, and code following each
non-fallthrough statement.

### R3 — void is a no-value state, not a synthetic value

The current MIR and every optimizer/backend helper assumes calls have targets and
returns have values. Phase B must audit all constructors, printers, validators,
use/def collectors, inlining logic, temporary collectors, terminator walkers, and
backend emitters. `MirType::Void` may describe only a function return; no dummy
constant, local, temp, parameter, place, or result pointer may be introduced.

### R4 — slice values require target-specific compound-value helpers

The C backend can use descriptor structs and LLVM can use aggregates, but the
WASM backend currently assumes every MIR value has one scalar type and one local.
Phase C must add a logical-value abstraction for slice data/length expansion
rather than scattering name suffix rules through emitters. It must cover params,
locals, temps, moves, loads/stores, calls, dispatcher returns, structured paths,
and multi-value internal returns. The MIR must remain semantic and must not be
rewritten into generated scalar locals.

### R5 — checked slice places need instruction-scoped guard preludes

The current C `c_place` helper returns only an expression string. It cannot by
itself emit a bounds guard before an address, load, or store, especially for a
nested place such as `items[i].price`. Checked C lowering must produce an ordered
prelude plus the final place expression. It must use already-lowered MIR values,
emit exactly one logical guard per slice index, and perform no pointer arithmetic
or dereference before the guard. Raw pointer places and `.data`-derived pointer
places must remain unguarded.

### R6 — optimizer context must represent bounds observability

The current pass context carries overflow mode but not bounds mode. Add bounds
mode to the pass context and keep `Subslice` and slice-index-dependent operations
conservative in checked mode. The initial implementation may also remain
conservative in unchecked mode, but no O1–O3 pass may delete, duplicate, or move a
checked bounds observation or change arithmetic/call-versus-bounds error order.

### R7 — generated C declarations and names need one dependency-aware allocator

Stored slice values and slice fields require generated descriptor declarations,
and exported slice parameters require deterministic flattened names. Planning
must include one collision-aware C identifier allocator and declaration ordering
that handles descriptors referencing named struct element types. Golden headers
must cover collisions with user struct, parameter, local, temporary, status, and
return-helper names.

### R8 — new-language tests cannot silently depend on the legacy oracle

The TypeScript oracle does not parse these features. New tests must assert direct
diagnostics, MIR structure, backend output, and cross-backend runtime results.
Existing oracle-backed V0 tests remain regression gates and must not be weakened
or rewritten to avoid failures.

## Design-to-repository traceability

| Concern | Current implementation anchor | Required direction |
| --- | --- | --- |
| Keywords and ranges | `src/lexer/mod.rs` | reserved tokens; `..` before float fraction |
| Statements and types | `src/parser.rs` | explicit AST variants; optional return expression |
| Flow and type rules | `src/typeck.rs` | structured flow summary; void/slice position rules |
| Semantic operations | `src/mir/mod.rs` | optional call/return values; slice ops and places |
| Optimization safety | `src/opt/mod.rs` | exhaustive new variants; bounds-aware context |
| C ABI and checks | `src/backend/mod.rs` | descriptors, flattening, status ABI, ordered guards |
| WASM ABI | `src/backend/mod.rs` | paired locals/params, size 8 alignment 4, multi-return |
| LLVM ABI | `src/backend/mod.rs` | aggregate descriptors and flattened params |
| CLI mode matrix | `src/main.rs` | independent `--overflow` and `--bounds` validation |
| Durable contracts | `docs/`, `docs/zh-CN/` | matching English/Chinese updates per phase |

## Non-blocking choices delegated to the execution plan

The following details may be fixed by the plan without changing the approved
language design:

- concrete Rust names for new AST/MIR helper structs and enums;
- MIR printer spelling, provided it is deterministic and covered by snapshots;
- new diagnostic-code allocation, provided old codes and formatting stay stable;
- the exact deterministic suffix algorithm for generated identifiers;
- whether conservative unchecked-mode optimizer behavior is later relaxed;
- focused helper/module extraction inside the existing compiler layers.

## Exit decision

The design review passes. No semantic or architectural blocker requires a design
revision, so the workflow advances directly to plan decomposition and plan
self-review. Risks R1–R8 must appear as explicit tasks and acceptance evidence;
omitting one is a plan defect.
