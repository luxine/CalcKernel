# Phase B Execution Plan — explicit `void`

Status: waiting for Phase A acceptance

Acceptance contract:
`Ai_repository/2026-08-26-control-flow-void-slice-phase-b-acceptance.md`

## Phase outcome

Add return-only `void`, `return;`, natural void fallthrough, and standalone void
call statements. Represent absence of a value explicitly through MIR,
optimization, C/status C, WASM, and LLVM without synthetic return values.

## B1 — Parse void returns and call statements

Files:

- Modify `src/lexer/mod.rs`
- Modify `src/parser.rs`
- Modify `tests/lexer_test.rs`
- Modify `tests/parser_test.rs`

Red tests:

- `lex_should_tokenize_void_as_a_reserved_keyword`
- `parse_should_allow_void_only_in_type_syntax_for_later_checking`
- `parse_should_parse_return_with_and_without_value`
- `parse_should_distinguish_assignment_from_call_statement`
- `parse_should_recover_from_non_call_expression_statement`

Implementation:

- Add `TokenKind::Void` and a void `TypeNode` representation.
- Change `ReturnStatement.value` to `Option<Expression>`.
- Add `CallStatement` containing the parsed call expression and span.
- Parse a leading expression once, then choose assignment on `=`, call statement
  on `;` plus `Expression::Call`, or a stable parser error otherwise.
- Preserve assignment target parsing and current precedence.

## B2 — Enforce return-only void and call-position rules

Files:

- Modify `src/diagnostics.rs`
- Modify `src/typeck.rs`
- Modify `tests/checker_test.rs`

Red tests:

- `check_should_accept_void_fallthrough_and_empty_return`
- `check_should_reject_void_parameter_local_field_pointer_and_argument_with_ck2011`
- `check_should_reject_value_return_from_void_and_empty_return_from_value_function`
- `check_should_accept_only_void_calls_as_statements`
- `check_should_reject_void_call_in_every_value_context`
- `check_should_preserve_unreachable_analysis_after_empty_return`

Implementation:

- Add `CalcKernelType::Void` and diagnostic `CK2011`.
- Resolve void with a type-use context so only a function return may contain it;
  do not turn invalid positions into later MIR failures.
- Teach call checking to distinguish value-required and statement contexts.
- Apply missing-return only to non-void functions.
- Give natural void fallthrough a valid flow summary and keep unreachable checks
  from Phase A intact.
- Keep general expression statements and discarding non-void results illegal.

## B3 — Make MIR calls and returns explicitly optional

Files:

- Modify `src/mir/mod.rs`
- Modify `tests/mir_test.rs`

Red tests:

- `mir_should_print_targetless_void_calls_and_valueless_returns`
- `mir_should_insert_return_none_for_void_fallthrough`
- `mir_validator_should_reject_void_values_and_call_return_mismatches`
- `mir_validator_should_accept_void_control_flow_with_all_blocks_terminated`

Implementation:

- Add `MirType::Void` for function return signatures only.
- Change `MirInstruction::Call.target` and `MirTerminator::Return.value` to
  `Option<MirValue>`.
- Lower call statements to targetless calls and both explicit/natural void exits
  to valueless returns.
- Reject void params, locals, temps, constants, places, operands, and slice
  positions in validation.
- Update deterministic printing, type conversion, helper APIs, and validation of
  callee/return combinations.

## B4 — Audit optimizer use-def and inlining behavior

Files:

- Modify `src/opt/mod.rs`
- Modify `tests/optimizer_test.rs`

Red tests:

- `optimizer_should_keep_targetless_calls_as_side_effects_at_all_levels`
- `optimizer_should_handle_valueless_returns_in_cfg_passes`
- `optimizer_should_not_require_a_synthetic_void_temp`
- `optimizer_should_inline_or_conservatively_keep_void_helpers_without_panicking`

Implementation:

- Update every instruction-target, use collection, renaming, substitution,
  inlining, DCE, CFG, and terminator match for optional values.
- A targetless call is always effectful and may not be removed.
- It is acceptable to keep void callees non-inlineable in this release, provided
  the decision is explicit, deterministic, and validated at O0–O3.

## B5 — Emit correct C, WASM, and LLVM void ABIs

Files:

- Modify `src/backend/mod.rs`
- Modify `tests/c_backend_test.rs`
- Modify `tests/wasm_backend_test.rs`
- Modify `tests/llvm_backend_test.rs`
- Modify `tests/cli_test.rs`

Red tests:

- `c_backend_should_emit_and_run_unchecked_void_functions`
- `c_backend_should_emit_status_void_without_ck_return`
- `c_backend_should_propagate_checked_void_call_failures`
- `wasm_backend_should_emit_and_run_void_functions_without_results`
- `llvm_backend_should_emit_call_and_ret_void`
- `cli_should_build_void_buffer_mutation_fixture`

Implementation:

- C unchecked: `void`, plain call statements, and `return;`.
- C status ABI: `CK_Status` return, no `ck_return` for source void, `CK_OK` on
  explicit/natural returns, and status propagation for targetless calls.
- WASM: omit result declaration and return local; emit call without local set;
  handle void returns in single-block, dispatcher, and structured paths.
- LLVM: use `void`, `call void`, and `ret void`; never emit a zero value for void.
- Update helper functions that collect temps, generate signatures, and determine
  whether a function needs status temporaries.

## B6 — Document and demonstrate Phase B

Files:

- Add `examples/void.ck`
- Modify `README.md`
- Modify `README.zh-CN.md`
- Modify English and Chinese pairs for `LANGUAGE_SPEC.md`,
  `COMPILER_ARCHITECTURE.md`, `MIR.md`, `ABI.md`, `CHECKED_ARITHMETIC.md`,
  `WASM_ABI.md`, `LLVM_BACKEND.md`, and `ROADMAP.md`
- Modify `tests/docs_surface_test.rs`

Red test:

- `void_docs_should_cover_return_only_type_and_backend_abis`

The example mutates caller-owned memory and demonstrates early `return;`, natural
fallthrough, and a void call. Do not migrate existing public examples or add the
new example to oracle parity lists.

## B7 — Phase acceptance and commit

Execute and record the Phase B acceptance document. Then:

```bash
git diff --check
git status --short
git add src tests examples README.md README.zh-CN.md docs Ai_repository
git commit -m "feat: add explicit void procedures"
```

Do not start Phase C until Phase B is committed and fully accepted.
