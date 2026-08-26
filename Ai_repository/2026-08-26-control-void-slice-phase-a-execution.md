# Phase A Execution Plan — `break` and `continue`

Status: completed and accepted on 2026-08-26

Prerequisite: planning commit only

Acceptance contract:
`Ai_repository/2026-08-26-control-flow-void-slice-phase-a-acceptance.md`

## Phase outcome

Add unlabeled `break;` and `continue;` for the innermost `while`, replace the
checker’s return-only reachability shortcut with structured flow outcomes, reject
unreachable source statements, and preserve correct behavior across C, WASM,
LLVM, and O0–O3.

## A1 — Reserve and parse loop-control statements

Files:

- Modify `src/lexer/mod.rs`
- Modify `src/parser.rs`
- Modify `tests/lexer_test.rs`
- Modify `tests/parser_test.rs`

Red tests:

- `lex_should_tokenize_break_and_continue_as_reserved_keywords`
- `parse_should_parse_break_and_continue_statements_with_spans`
- `parse_should_require_semicolons_after_loop_control`

Implementation:

- Add `TokenKind::Break` and `TokenKind::Continue` and keyword mappings.
- Add spanned `BreakStatement` / `ContinueStatement` AST structs and statement
  variants.
- Parse only the exact `break;` and `continue;` forms.
- Include the new variants in statement span access and parser recovery.

Focused green command:

```bash
cargo test --locked --test lexer_test --test parser_test
```

## A2 — Introduce explicit control-flow diagnostics and flow summaries

Files:

- Modify `src/diagnostics.rs`
- Modify `src/typeck.rs`
- Modify `tests/checker_test.rs`

Red tests:

- `check_should_reject_break_and_continue_outside_while_with_ck2009`
- `check_should_bind_loop_control_to_innermost_lexical_while`
- `check_should_report_unreachable_after_return_break_and_continue_with_ck2010`
- `check_should_combine_if_branch_flow_without_false_unreachable_errors`
- `check_should_keep_while_conservatively_fallthrough_for_missing_return`

Implementation:

- Add diagnostic variants `Ck2009` and `Ck2010` and explicit coded error
  reporting in the checker.
- Carry `loop_depth` while checking nested blocks/loops.
- Replace `block_definitely_returns` with a `FlowSummary` that records
  fallthrough, return, break, and continue outcomes.
- Fold statements sequentially only while fallthrough remains possible; report
  every subsequent source statement as unreachable with its own span.
- Combine `if` arms by union; an absent `else` contributes fallthrough.
- Consume break/continue at the enclosing while, preserve return, and always
  retain conservative loop fallthrough.
- Keep missing-return behavior unchanged for every non-void function.

Focused green command:

```bash
cargo test --locked --test checker_test
```

## A3 — Lower loop control to MIR jump targets

Files:

- Modify `src/mir/mod.rs`
- Modify `tests/mir_test.rs`

Red tests:

- `mir_should_lower_break_to_innermost_loop_exit`
- `mir_should_lower_continue_to_innermost_loop_condition`
- `mir_should_lower_nested_loop_control_without_dangling_blocks`
- `mir_should_validate_break_continue_cfg_after_checker_acceptance`

Implementation:

- Add a `LoopTargets` stack to function lowering.
- Push condition/exit labels before lowering a while body and pop them on every
  successful return path.
- Lower `break` / `continue` to `MirTerminator::Jump` and clear the current source
  block without manufacturing a fallthrough block.
- Let enclosing `if` / block lowering join only branches that still fall through.
- Treat a loop-control statement reaching MIR without a loop target as an
  invariant violation, not a backend behavior.
- Preserve deterministic labels and current MIR printer format.

Focused green command:

```bash
cargo test --locked --test mir_test
```

## A4 — Prove optimizer and backend correctness at every level

Files:

- Modify `tests/optimizer_test.rs`
- Modify `tests/c_backend_test.rs`
- Modify `tests/wasm_backend_test.rs`
- Modify `tests/llvm_backend_test.rs`

Red tests:

- `optimizer_should_preserve_break_continue_targets_at_all_opt_levels`
- `c_backend_should_run_nested_break_continue_at_all_opt_levels`
- `wasm_backend_should_run_break_continue_dispatcher_fallback_at_all_opt_levels`
- `llvm_backend_should_run_nested_break_continue_at_all_opt_levels`

Runtime fixture behavior must cover:

- early exit;
- skipped iterations;
- `continue` inside an `if`;
- nested loops proving innermost selection;
- a function return inside the same loop;
- O3 WASM falling back to the dispatcher when the simple structured recognizer no
  longer matches.

Implementation changes should be unnecessary in C/LLVM because they already
consume arbitrary MIR jumps. Update WASM structured recognition only if needed
for correctness; dispatcher fallback is the intended general path.

Focused green commands:

```bash
cargo test --locked --test optimizer_test
cargo test --locked --test c_backend_test
cargo test --locked --test wasm_backend_test
cargo test --locked --test llvm_backend_test
```

## A5 — Document and demonstrate Phase A

Files:

- Add `examples/control_flow.ck`
- Modify `README.md`
- Modify `README.zh-CN.md`
- Modify `docs/LANGUAGE_SPEC.md`
- Modify `docs/zh-CN/LANGUAGE_SPEC.md`
- Modify `docs/COMPILER_ARCHITECTURE.md`
- Modify `docs/zh-CN/COMPILER_ARCHITECTURE.md`
- Modify `docs/MIR.md`
- Modify `docs/zh-CN/MIR.md`
- Modify `docs/ROADMAP.md`
- Modify `docs/zh-CN/ROADMAP.md`
- Modify `tests/docs_surface_test.rs`

Red test:

- `control_flow_docs_should_cover_break_continue_and_unreachable_rules`

Document exact syntax, innermost-loop behavior, conservative while flow,
unreachable diagnostics, MIR jump lowering, and backend fallback. The new example
must compile through all three promised backends but must not be added to the
TypeScript-oracle fixture list.

## A6 — Phase acceptance and commit

Run every command and record evidence in the Phase A acceptance document. If a
gate fails, repair production behavior or a genuinely incorrect test; do not
relax the contract. After all gates pass:

```bash
git diff --check
git status --short
git add src tests examples README.md README.zh-CN.md docs Ai_repository
git commit -m "feat: add break and continue control flow"
```

Do not start Phase B until the commit exists and Phase A acceptance is complete.
