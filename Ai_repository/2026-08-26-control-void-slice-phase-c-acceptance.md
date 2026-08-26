# Phase C Acceptance — first-class slices and bounds modes

Status: waiting for Phase B

All unchecked and checked promises below are mandatory. Checked bounds are C-only
in this release; explicit rejection by WASM/LLVM is part of acceptance.

## Lexer/parser acceptance

- [ ] `slice` and `..` are reserved/tokenized with exact spans.
- [ ] `items[0..len]`, `items[0 .. len]`, and ordinary `items[0]` parse distinctly.
- [ ] Existing valid floats still lex as one token; `1.` and `.5` retain their
      malformed-float diagnostics.
- [ ] Omitted range endpoints and extra `..` are rejected and recover cleanly.
- [ ] `slice(data, len)` is dedicated syntax and cannot be shadowed.

## Type-system acceptance

- [ ] Slice element whitelist is primitive, raw pointer, or ordinary named
      struct; void and direct slice elements are rejected.
- [ ] Locals, params, call arguments, ordinary struct fields, assignments, and
      internal returns work with exact descriptor types.
- [ ] Exported slice returns are rejected; exported slice params are accepted.
- [ ] Constructor pointer element and length types are exact.
- [ ] Index, start, end, and length use `u32` or a non-negative materializable
      integer literal; other integer types and out-of-range literals fail.
- [ ] `.data` and `.len` are readable with correct types and individually
      non-assignable; whole descriptors remain assignable.
- [ ] Pointer indexing and `.data` escape retain raw unchecked semantics.

## MIR/optimizer acceptance

- [ ] MIR exposes semantic slice type, construction, projection, sub-slice, and a
      distinct slice-index place.
- [ ] Descriptor, then index/start/end operands are each evaluated once in source
      order.
- [ ] Moves, loads/stores, calls, fields, and internal returns preserve exact
      descriptor types.
- [ ] Validator-negative tests cover malformed elements, operands, places,
      calls, returns, and exported returns.
- [ ] Bounds mode is present in every MIR pass context.
- [ ] O1–O3 do not delete, duplicate, CSE, or hoist a checked sub-slice or
      slice-index observation and preserve overflow/call/bounds precedence.

## C ABI acceptance

- [ ] Stored descriptors are deterministic generated structs with typed pointer
      plus `uint32_t`.
- [ ] Descriptor/ordinary struct declarations compile in dependency-safe order.
- [ ] Generated identifiers are deterministic and collision-safe.
- [ ] Every exported and internal slice param is physically flattened.
- [ ] Unchecked internal slice returns use descriptor-by-value.
- [ ] Status internal slice returns use descriptor output pointer.
- [ ] Public header is authoritative and compiles under C11 `-Wall -Wextra
      -Werror`.

## Checked C acceptance

The full status set is emitted whenever overflow or bounds is checked:

- [ ] `CK_OK = 0`
- [ ] `CK_ERR_OVERFLOW = 1`
- [ ] `CK_ERR_DIV_BY_ZERO = 2`
- [ ] `CK_ERR_NULL_POINTER = 3`
- [ ] `CK_ERR_OUT_OF_BOUNDS = 4`

Runtime cases:

| Case | Expected |
| --- | --- |
| index `len - 1` for nonempty slice | success |
| index equal to length | out of bounds |
| very large `u32` index | out of bounds before address |
| reversed `start > end` | out of bounds before subtraction |
| `end > len` | out of bounds |
| empty `start == end <= len` | success, length zero |
| zero-start empty slice | original pointer bits preserved |
| nested struct field access | one logical index guard |
| arithmetic overflow computing index | overflow precedes bounds |
| failing call computing operand | call status precedes bounds |
| nested void/value/slice calls | status propagates unchanged |
| raw `ptr<T>` index | no slice guard |
| `.data` then raw index | no slice guard |

- [ ] Every row passes at O0, O1, O2, and O3.
- [ ] No pointer advancement, address formation, dereference, or range subtraction
      occurs before its required guard.
- [ ] Non-void `ck_return == NULL` remains the first body error; void has no
      generated result pointer.

## WASM acceptance

- [ ] Slice params expand to two collision-safe i32 params.
- [ ] Slice locals/temps use paired physical locals through every CFG path.
- [ ] Stored descriptor layout is size 8, alignment 4, offsets 0/4.
- [ ] Calls expand logical args; internal return is `(i32, i32)` with correct
      result order in direct, dispatcher, and structured paths.
- [ ] Slice of ordinary struct advances by deterministic struct size.
- [ ] Valid construction/index/sub-slice/field/load/store programs run with the
      same results as C and LLVM at O0–O3.
- [ ] `--bounds checked` is rejected before emission; unchecked emits no guards.

## LLVM acceptance

- [ ] Stored descriptor type is `{ ptr, i32 }`.
- [ ] Slice params flatten to `ptr, i32` and reconstruct logical aggregates.
- [ ] Moves, fields, loads/stores, calls, and internal aggregate returns verify
      and compile with clang.
- [ ] Index/sub-slice uses the correct element type in GEP.
- [ ] Zero-start preserves original pointer bits.
- [ ] Valid runtime results match C/WASM at O0–O3.
- [ ] `--bounds checked` is rejected before emission.

## CLI acceptance

| Command family | unchecked bounds | checked bounds |
| --- | --- | --- |
| `emit-c`, `build` | accepted | accepted |
| `emit-wat`, `emit-wasm` | accepted | stable WASM rejection |
| `emit-llvm`, `build-llvm` | accepted | stable LLVM rejection |
| `check`, `emit-mir` | source/mode independent | source/mode independent |

- [ ] Default is unchecked.
- [ ] C accepts all four overflow/bounds combinations.
- [ ] Invalid values are stable command errors where the flag is consumed.
- [ ] Unsupported checked overflow precedes unsupported checked bounds for
      WASM/LLVM, independent of source flag order.
- [ ] Success/help output documents the effective matrix.

## Documentation and regression acceptance

- [ ] `examples/slices.ck` demonstrates construction, forwarding, indexing,
      projections, sub-slicing, fields, and internal return.
- [ ] English/Chinese language, architecture, MIR, ABI, checked arithmetic,
      optimization, WASM, LLVM, roadmap, and output docs agree.
- [ ] Caller ownership, invalid raw descriptor responsibility, aliasing, and raw
      escape are explicit.
- [ ] Existing example signatures and TypeScript fixtures remain unchanged.

## Required commands

```bash
cargo test --locked --test lexer_test --test parser_test --test checker_test --test mir_test
cargo test --locked --test optimizer_test
cargo test --locked --test c_backend_test
cargo test --locked --test wasm_backend_test
cargo test --locked --test llvm_backend_test
cargo test --locked --test cli_test
cargo test --locked --test docs_surface_test
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
CALCKERNEL_TS_ROOT=/Users/lynn/code/CalcKernel cargo test --locked
cargo build --release --locked
cargo run --locked --bin ckc -- --help
git diff --check
```

## Evidence record

| Date | Command / check | Result | Notes |
| --- | --- | --- | --- |
| | | | |

## Exit decision

- [ ] Every checkbox and runtime matrix row is satisfied.
- [ ] Phase C changes and this evidence are ready for the dedicated phase commit.

Accepted by inline execution: pending
