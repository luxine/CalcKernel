# Phase C Execution Plan — first-class `slice<T>` and optional bounds checks

Status: waiting for Phase B acceptance

Acceptance contract:
`Ai_repository/2026-08-26-control-void-slice-phase-c-acceptance.md`

## Phase outcome

Add semantic non-owning slice descriptors, raw construction, read-only
projections, `u32` indexing, half-open sub-slicing, exact descriptor copies,
flattened parameters, internal returns, unchecked C/WASM/LLVM behavior, and
checked C bounds through module-wide status ABI.

## C1 — Lex and parse slice syntax without float regressions

Files:

- Modify `src/lexer/mod.rs`
- Modify `src/parser.rs`
- Modify `tests/lexer_test.rs`
- Modify `tests/parser_test.rs`

Red tests:

- `lex_should_tokenize_slice_and_dotdot_with_exact_spans`
- `lex_should_prefer_dotdot_over_float_fraction_after_integer`
- `lex_should_preserve_supported_and_malformed_float_behavior`
- `parse_should_parse_slice_type_constructor_and_subslice`
- `parse_should_distinguish_index_from_explicit_range`
- `parse_should_reject_omitted_or_extra_range_endpoints`

Implementation:

- Add reserved `Slice` and punctuation `DotDot` tokens.
- In number scanning, enter the fraction path only for a single dot followed by a
  digit; leave `..` for the punctuation scanner. Preserve current `1.` and `.5`
  diagnostics.
- Add `TypeNode::Slice`, `Expression::SliceConstructor`, and
  `Expression::Subslice` with complete spans.
- Parse `slice(data, len)` as dedicated syntax, not a normal/shadowable call.
- In postfix brackets, parse one expression and then either `]` for index or
  `.. expression ]` for a sub-slice.

## C2 — Enforce semantic slice types and exact operations

Files:

- Modify `src/typeck.rs`
- Modify `tests/checker_test.rs`

Red tests must cover:

- allowed primitives, raw pointers, and ordinary named structs as elements;
- rejection of `void`, direct `slice<slice<T>>`, and invalid type positions;
- locals, params, arguments, ordinary struct fields, assignments, and internal
  returns;
- rejection of exported slice returns;
- `slice(ptr, len)` exact pointer element and `u32` length rules;
- exact slice element matching in initialization, assignment, call, and return;
- `.data: ptr<T>` and `.len: u32` reads plus assignment rejection;
- slice index/endpoints accepting `u32` and non-negative materializable literals,
  while rejecting `i32`, `u64`, negative, and out-of-range literals;
- pointer indexing retaining its existing rules;
- field access after indexing a slice of struct;
- source evaluation order represented by the typed AST.

Implementation:

- Add `CalcKernelType::Slice(Box<CalcKernelType>)` and explicit `CK2012` errors.
- Extend type resolution with a location/context check so invalid recursive
  direct-slice and void positions are diagnosed at their source spans.
- Add checker branches for constructor, projection, index, sub-slice, assignment
  target, calls, and exported returns.
- Treat `.data` and `.len` as compiler projections only when the receiver is a
  slice; ordinary struct field rules remain unchanged.
- Preserve exact type equality and the existing integer-literal materialization
  model, specializing slice lengths/indices/endpoints to `u32`.

## C3 — Add explicit slice MIR and validation

Files:

- Modify `src/mir/mod.rs`
- Modify `tests/mir_test.rs`

Red tests:

- `mir_should_print_make_slice_projections_index_and_subslice`
- `mir_should_copy_slice_locals_fields_arguments_and_internal_returns`
- `mir_should_evaluate_slice_then_index_or_range_operands_once_in_order`
- `mir_validator_should_reject_each_malformed_slice_operation`
- `mir_validator_should_reject_void_or_direct_slice_elements_and_exported_returns`

Implementation:

- Add `MirType::Slice(Box<MirType>)`.
- Add `MakeSlice`, `SliceData`, `SliceLen`, and `Subslice` instructions.
- Add `MirPlace::SliceIndex { slice, index, type_node }`; keep raw
  `MirPlace::Index` unchanged.
- Lower arbitrary slice descriptor expressions to one logical `MirValue` before
  lowering the following index/start/end expressions.
- Support exact slice descriptor values in moves, loads/stores, calls, and
  internal returns.
- Extend printer, type helpers, target helpers, use checks, place validation,
  instruction validation, call validation, and module-level exported-return
  validation.
- Keep bounds guards out of MIR.

## C4 — Make optimizer behavior bounds-aware and conservative

Files:

- Modify `src/opt/mod.rs`
- Modify all `MirPassContext` construction sites in `src/main.rs` and `tests/*.rs`
- Modify `tests/optimizer_test.rs`

Red tests:

- `optimizer_should_track_slice_instruction_and_place_uses`
- `optimizer_should_keep_checked_subslice_even_when_result_is_dead`
- `optimizer_should_keep_checked_slice_index_address_observable`
- `optimizer_should_not_cse_or_hoist_checked_slice_guards_at_o1_through_o3`
- `optimizer_should_preserve_slice_internal_calls_and_returns`

Implementation:

- Add `MirPassBoundsMode::{Unchecked, Checked}` to `MirPassContext`.
- Update every type key, instruction target, use/def collector, value replacement,
  place rewrite, candidate classifier, DCE rule, CSE key, inliner, and CFG walker.
- In checked mode, retain `Subslice` and any address/load/store whose place
  contains `SliceIndex`, because backend guard failure is observable.
- Do not apply address CSE, local CSE, LICM, or proof-free elimination across a
  checked slice operation. The first release need not optimize these operations
  in unchecked mode either.
- Preserve operand and call failure order.

## C5 — Build deterministic C descriptor types and flattened ABIs

Files:

- Modify `src/backend/mod.rs`
- Modify `tests/c_backend_test.rs`

Red tests:

- `c_backend_should_emit_dependency_ordered_slice_descriptors`
- `c_backend_should_flatten_exported_and_internal_slice_params`
- `c_backend_should_copy_slice_locals_fields_and_internal_returns`
- `c_backend_should_disambiguate_generated_slice_and_parameter_names`
- `c_backend_should_compile_generated_slice_headers_with_werror`

Implementation:

- Extend `EmitCOptions` with independent bounds mode.
- Collect all reachable slice element types deterministically from MIR types.
- Use one collision-aware identifier allocator for descriptor types, flattened
  param names, internal temporaries, `ck_return`, and status helpers.
- Emit named struct forward declarations, descriptor declarations, and complete
  ordinary struct declarations in dependency-safe order.
- Use descriptor structs for stored values and internal returns; flatten every
  physical slice parameter into typed data pointer plus `uint32_t` length.
- Expand logical slice call arguments and reconstruct logical descriptors at
  function entry without changing MIR.
- Return internal slices by descriptor value in unchecked C and through a final
  descriptor pointer in status C. Keep exported slice returns rejected earlier.

## C6 — Add checked C bounds with ordered place preludes

Files:

- Modify `src/backend/mod.rs`
- Modify `tests/c_backend_test.rs`
- Modify `tests/cli_test.rs`

Red tests:

- `checked_c_backend_should_guard_slice_reads_writes_and_nested_fields`
- `checked_c_backend_should_guard_subslice_before_arithmetic_or_pointer_advance`
- `checked_c_backend_should_return_out_of_bounds_for_edge_cases`
- `checked_c_backend_should_preserve_empty_zero_start_slice_pointer`
- `checked_c_backend_should_propagate_bounds_through_void_value_and_slice_calls`
- `checked_c_backend_should_preserve_overflow_call_and_bounds_error_precedence`
- `checked_c_backend_should_leave_raw_pointer_and_slice_data_access_unchecked`
- `checked_c_backend_should_preserve_guards_at_o0_through_o3`

Implementation:

- Activate module-wide `CK_Status` ABI when overflow or bounds is checked and emit
  all five stable status constants.
- Refactor checked C place emission to return ordered prelude lines plus a final
  lvalue/address expression. It must recurse through `Field(SliceIndex(...))`
  without duplicating the logical check.
- For slice index, compare the already-evaluated `u32` index with descriptor
  length before calculating an element pointer.
- For sub-slice, check `start <= end && end <= len` before subtraction and pointer
  advancement. Use a conditional zero-start path that copies the original data
  pointer exactly.
- A failed guard immediately returns `CK_ERR_OUT_OF_BOUNDS`; checked arithmetic
  and nested call status produced while evaluating operands occurs first.
- Do not guard construction, raw pointer indexing, or indexing through `.data`.

## C7 — Lower logical slices to paired WASM values

Files:

- Modify `src/backend/mod.rs`
- Modify `tests/wasm_backend_test.rs`

Red tests:

- `wasm_backend_should_flatten_slice_params_collision_safely`
- `wasm_backend_should_emit_paired_slice_locals_temps_moves_and_projections`
- `wasm_backend_should_load_store_slice_fields_with_size8_align4`
- `wasm_backend_should_return_internal_slices_as_two_values`
- `wasm_backend_should_run_slice_index_subslice_and_struct_elements_at_all_levels`

Implementation:

- Add backend-local logical-value helpers that map a slice value to deterministic
  data/length physical names; do not alter semantic MIR.
- Flatten slice parameters and represent locals/temps with paired `i32` locals.
- Load/store stored descriptors at offsets 0/4 and use size 8, alignment 4 for
  slice fields.
- Expand calls and use `(result i32 i32)` for internal slice returns, assigning
  stack results to the target pair in correct reverse-pop order.
- Carry paired dispatcher return locals and emit both final result values.
- Implement construction, projection, sub-slice, and slice index address
  calculation using the element’s deterministic WASM size.
- Keep checked bounds rejected at the CLI; no silent guards or traps are added.

## C8 — Lower logical slices to LLVM aggregates

Files:

- Modify `src/backend/mod.rs`
- Modify `tests/llvm_backend_test.rs`

Red tests:

- `llvm_backend_should_flatten_slice_params_and_reconstruct_aggregate_values`
- `llvm_backend_should_emit_slice_moves_fields_and_internal_aggregate_returns`
- `llvm_backend_should_gep_slice_indices_and_subslices`
- `llvm_backend_should_run_slice_programs_at_all_opt_levels`

Implementation:

- Map a stored slice to `{ ptr, i32 }`, including fields, allocas, loads, stores,
  moves, and internal returns.
- Flatten every physical param to `ptr, i32` and reconstruct the logical aggregate
  used by MIR.
- Expand logical call arguments and accept aggregate internal return values.
- Use `extractvalue` / `insertvalue` and target-aware `getelementptr` for
  construction, projection, indexing, and sub-slicing.
- Preserve the exact original pointer for zero start through conditional
  selection rather than relying on null GEP behavior.
- Keep checked bounds unsupported and explicitly rejected by CLI.

## C9 — Implement CLI matrix and precedence

Files:

- Modify `src/main.rs`
- Modify `tests/cli_test.rs`

Red tests:

- `cli_should_default_bounds_to_unchecked_for_all_emitters`
- `cli_should_accept_all_four_c_overflow_bounds_combinations`
- `cli_should_reject_invalid_bounds_values_on_commands_that_use_them`
- `cli_should_reject_checked_bounds_for_wasm_and_llvm_with_stable_help`
- `cli_should_report_checked_overflow_before_checked_bounds_for_wasm_and_llvm`
- `cli_should_keep_check_and_emit_mir_source_semantics_mode_independent`

Implementation:

- Add parsed `bounds`, backend `BoundsMode`, and pass-context bounds mapping.
- C emit/build parse both modes and report both in success output.
- WASM/LLVM commands parse overflow first, return the existing checked-overflow
  error immediately, then parse/reject checked bounds. This fixes flag-order
  independent precedence.
- `check` and `emit-mir` do not validate or apply bounds mode, matching their
  existing semantic-flag deferral behavior.
- Update usage text for every supported/rejected command.

## C10 — Cross-backend fixtures and durable documentation

Files:

- Add `examples/slices.ck`
- Modify `README.md` and `README.zh-CN.md`
- Modify English/Chinese pairs for `LANGUAGE_SPEC.md`,
  `COMPILER_ARCHITECTURE.md`, `MIR.md`, `ABI.md`, `CHECKED_ARITHMETIC.md`,
  `WASM_ABI.md`, `LLVM_BACKEND.md`, `OPTIMIZATION.md`, `ROADMAP.md`, and
  `ckc-outputs.md`
- Modify `docs/MIGRATION.md` and `docs/zh-CN/MIGRATION.md`
- Modify `docs/wasm-interop.md` and `docs/zh-CN/wasm-interop.md`
- Modify `tests/docs_surface_test.rs`
- Modify backend/CLI runtime tests as needed for one shared semantic fixture

Red tests:

- `slice_docs_should_define_ownership_bounds_and_backend_matrix`
- `slice_example_should_run_with_equal_valid_results_across_backends`

Documentation must spell out descriptor aliasing, caller-owned validity, exact
`u32` rules, half-open range precondition, read-only projections, raw escape,
flattened ABI, internal return representation, C status code 4, and explicit
WASM/LLVM rejection. Migration notes must identify all four newly reserved
keywords. Existing published example signatures remain unchanged and the new
file is excluded from legacy oracle lists.

## C11 — Phase acceptance and commit

Execute every Phase C acceptance gate and record evidence. Then:

```bash
git diff --check
git status --short
git add src tests examples README.md README.zh-CN.md docs Ai_repository
git commit -m "feat: add first-class slices and checked C bounds"
```

Proceed to final acceptance, not to a merge.
