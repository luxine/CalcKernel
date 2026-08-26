# CK Control Flow, Void, and Slice Evolution Design

Status: proposed; semantic decisions approved in conversation on 2026-08-26

## Context

CK / CalcKernel V0 deliberately has a small language surface: typed functions,
`let`, assignment, value-returning `return`, `if` / `else`, `while`, raw
`ptr<T>`, structs, and scalar expressions. That surface is sufficient for the
current C, WASM, and LLVM examples, but it has three closely related usability
gaps:

- A loop can only reach its next iteration through the end of the body, and can
  only exit early by returning from the whole function or by maintaining a
  manual flag.
- Every function must return a value, so procedures that only write through an
  output pointer return an artificial integer such as `0`.
- Buffers are represented as an unrelated `ptr<T>` and length parameter. The
  compiler cannot associate the length with an indexed access and therefore
  cannot offer optional bounds checks.

The current compiler already has the right broad structure for these features:
a typed AST, typed basic-block MIR, a MIR validator and optimizer, and separate
C, WASM, and LLVM emitters. This design extends those layers directly rather
than adding another IR or hiding language semantics in early desugaring.

## Goals

- Add unlabeled `break;` and `continue;` for the innermost `while` loop.
- Add an explicit return-only `void` type, `return;`, natural void fallthrough,
  and standalone void call statements.
- Add a first-class, non-owning `slice<T>` descriptor with raw construction,
  indexing, `.data`, `.len`, sub-slicing, assignment, locals, parameters,
  ordinary struct fields, and internal function returns.
- Use `u32` for every slice length, index, and sub-slice endpoint.
- Flatten slice parameters into data and length at every physical function ABI.
- Add `--bounds unchecked|checked`, defaulting to `unchecked`.
- Support unchecked slice semantics in C, WASM, and LLVM.
- Support checked slice bounds in the C backend first, using the existing
  `CK_Status` error-propagation model.
- Preserve existing `ptr<T>` behavior as an explicit unchecked escape hatch.
- Keep every phase independently releasable and fully tested before the next
  phase starts.

## Non-Goals

- `for`, `loop`, labeled loops, labeled break, or break-with-value.
- A first-class `unit` value, tuple syntax, or general expression statements.
- Discarding the result of a non-void function call.
- Direct `slice<slice<T>>` nesting.
- Owned arrays, allocation, deallocation, resizing, or lifetime management.
- Borrow checking, uniqueness, mutable/immutable reference types, or alias
  analysis at the source-language level.
- Omitted sub-slice endpoints such as `items[..end]` or `items[start..]`.
- Null-pointer validation or proof that a raw pointer covers its declared
  length.
- Bounds checks for raw `ptr<T>` indexing.
- Checked bounds for WASM or LLVM in the first slice release.
- Exported functions that return a slice descriptor.
- Automatic migration of existing public functions from `ptr<T>, len` to
  `slice<T>` or from artificial integer returns to `void`.
- Bounds-check elimination in the first checked-bounds implementation.

## Decisions and Compatibility

`break`, `continue`, `void`, and `slice` become reserved keywords. There is no
feature flag, contextual-keyword period, or compatibility alias. Existing CK
programs retain their behavior and ABI unless they use one of those words as an
identifier.

The work ships in three ordered phases:

1. `break` / `continue` and control-flow analysis.
2. `void`, void returns, and standalone void calls.
3. First-class slice descriptors and optional C bounds checks.

Existing public examples and benchmarks keep their current signatures unless a
separate migration is approved. New fixtures demonstrate the new language
surface without silently changing a published ABI.

## Approaches Considered

### Extend the typed AST and MIR

Keep `void` and `slice<T>` explicit through parsing, type checking, MIR,
validation, and optimization. Lower `break` and `continue` to ordinary MIR
jumps. Let each backend choose the physical representation of a semantic slice
value and flatten slice parameters at its ABI boundary.

This is the selected approach. It makes type and safety rules inspectable,
gives the MIR validator enough information to reject malformed programs, and
allows later optimizations to reason about slice operations without relying on
generated variable names.

### Desugar before MIR

Rewrite slices into compiler-generated pointer and length variables before MIR,
and rewrite the new control-flow and void forms into the existing language
surface. This would initially touch fewer MIR variants, but it would erase the
relationship between a data pointer and its length. Bounds checks, descriptor
assignment, struct fields, and internal slice returns would then depend on
fragile conventions and require later reconstruction.

### Add an HIR layer

Introduce a typed HIR between the checked AST and existing scalar MIR. This is a
reasonable long-term architecture for a much larger language, but it adds a new
compiler layer, printer, validator, and test surface before the current project
needs them. Direct typed AST and MIR extensions are smaller and remain clear for
the selected features.

## Compiler Architecture

The pipeline remains:

```text
Source
  -> lexer / parser
  -> typed AST + control-flow analysis
  -> explicit typed MIR
  -> MIR validation and optimization
  -> C / WASM / LLVM ABI lowering
```

Source-level slice values stay semantic descriptors until backend emission.
They are not rewritten into specially named scalar locals in the type checker.
The MIR printer exposes slice types and operations, while backend snapshots
expose each target's physical representation.

The new phases may make focused refactors inside the parser, checker, MIR, and
backend modules when the existing single-purpose functions become unwieldy.
They must not introduce unrelated module-system, ownership, allocator, or
general aggregate refactors.

## Phase A: `break` and `continue`

### Syntax and static semantics

The only new statements are:

```ck
break;
continue;
```

They operate on the innermost lexically enclosing `while`:

- `break;` transfers control to that loop's exit.
- `continue;` transfers control to that loop's condition.
- Either statement outside a `while` is a checker error.
- Neither statement accepts a label or value.

Nested blocks and `if` branches do not change which loop is selected. A nested
`while` establishes a new innermost loop until its body ends.

### Control-flow analysis

The current `block_definitely_returns` boolean is replaced by a small structured
flow summary. A statement or block can expose these outcomes:

- falls through to the following statement
- returns from the function
- breaks from the current loop
- continues the current loop

Sequential block analysis only visits a following statement if the preceding
summary can fall through. A statement after an unconditional `return`, `break`,
or `continue` is diagnosed as unreachable instead of reaching MIR lowering and
causing an internal "statement after return" error.

An `if` combines the outcomes of its branches. A `while` consumes break and
continue outcomes from its body, preserves possible function returns, and is
conservatively treated as able to fall through. Even `while true` does not count
as a definite return in this phase. A non-void function therefore still needs
an explicit return on every path after a loop.

### AST and MIR lowering

The AST gains spanned `BreakStatement` and `ContinueStatement` variants. The
checker carries loop depth for placement diagnostics.

MIR does not need break- or continue-specific terminators. Function lowering
maintains a stack of loop targets:

```text
LoopTargets {
  continue_label: condition block
  break_label: exit block
}
```

`break;` emits `MirTerminator::Jump` to `break_label`; `continue;` emits a jump
to `continue_label`. The current source block then has no fallthrough block.
Nested loops push and pop independent targets.

All three backends already understand arbitrary MIR jumps. The optimized WASM
structured-while recognizer may use its simple path when the new CFG still
matches; otherwise the existing dispatcher path is the correctness fallback.
Structured recognition can be broadened later without delaying semantic
support.

## Phase B: `void`

### Syntax

Void is written explicitly as a function return type:

```ck
fn clear(out: ptr<i64>, len: u32) -> void {
  let i: u32 = 0;
  while i < len {
    out[i] = 0;
    i = i + 1;
  }
}

fn maybe_clear(run: bool, out: ptr<i64>, len: u32) -> void {
  if !run {
    return;
  }
  clear(out, len);
}
```

A void function may reach the end of its body naturally. `return;` exits early.
`return value;` in a void function is an error, and `return;` in a non-void
function is an error.

`void` is a return-only type. It is rejected as a parameter type, local type,
ordinary struct field, pointer or slice element, and function-call argument.
There is no void value or void literal.

### Call statements

The grammar accepts a call followed by a semicolon as a statement:

```ck
clear(out, len);
```

The callee must return void. Calling a value-returning function as a statement
is an error because it silently discards a result. A void call cannot appear in
an initializer, return value, argument, arithmetic expression, comparison, or
other value-required position. No other expression becomes a valid standalone
statement.

After parsing the leading expression of a statement, the parser distinguishes:

- `target = value;` as assignment
- `callee(args...);` as a call statement
- every other expression statement as a parser or checker error

### Typed AST and MIR

The type system gains `CalcKernelType::Void`; the AST return statement stores an
optional expression. The missing-return rule applies only to non-void
functions.

MIR gains `MirType::Void`, but void is never the type of a `MirValue`, local,
parameter, constant, or place. Calls and returns change shape:

```text
Call {
  target: Option<MirValue>,
  function_name,
  args
}

Return {
  value: Option<MirValue>
}
```

A void call has no target. A void function returns `None`; a value-returning
function must return `Some(value)`. MIR validation rejects every mismatch.
Natural fallthrough in a void source function lowers to an explicit void return
terminator so every MIR block remains terminated.

### Backend ABI

Unchecked backend mappings are:

| Backend | CK `-> void` |
| --- | --- |
| C | C `void` return |
| WASM | function with no `(result ...)` |
| LLVM | LLVM `void` return and `ret void` |

When the C module uses status ABI because overflow or bounds checks are enabled,
a source void function returns `CK_Status` and does not receive `ck_return`:

```c
CK_API CK_Status clear(int64_t* out, uint32_t len);
```

Success, explicit `return;`, and natural fallthrough all return `CK_OK`.
Checked calls propagate a non-`CK_OK` status exactly like existing value calls,
but do not allocate a result temporary.

## Phase C: First-Class `slice<T>`

### Type and value semantics

A slice is a non-owning descriptor with this semantic shape:

```text
slice<T> = { data: ptr<T>, len: u32 }
```

Slice assignment, parameter passing, and return copy only the descriptor. Every
copy aliases the same caller-owned memory. CK does not allocate, retain, resize,
or free that memory.

The two descriptor fields are readable but not individually assignable:

- `items.data` has type `ptr<T>`.
- `items.len` has type `u32`.
- `items.data = ...` and `items.len = ...` are invalid assignment targets.
- Assigning a whole slice variable or a whole slice-valued ordinary struct field
  is allowed when the element types match exactly.

Access through `.data` is an explicit unchecked escape hatch. A later raw
pointer index is governed by the existing `ptr<T>` rules and never receives a
slice bounds check.

### Allowed element types and positions

`T` may be a scalar primitive, a raw pointer, or an ordinary named struct. Void
and direct slice types are not valid elements, so `slice<void>` and
`slice<slice<T>>` are rejected. A named struct may itself contain a slice field;
that does not create a directly nested slice element type.

A slice value may be used as:

- a local
- a function parameter
- an argument to another slice parameter
- an ordinary struct field
- an internal function return type
- an assignment source or destination

An exported function may accept slice parameters, but its declared return type
may not be `slice<T>`. A named struct that contains a slice field is still an
ordinary named struct; this design does not broaden or otherwise change the
existing backend rules for exported struct-by-value returns. An internal
function can return a slice and can be called from other CK functions.

### Raw construction

The dedicated expression:

```ck
slice(data, len)
```

requires `data: ptr<T>` and `len: u32`, and produces `slice<T>`. `slice` is
compiler syntax, not a shadowable function or a generic runtime helper.

This expression is the raw-memory trust boundary. Even under
`--bounds checked`, the compiler does not prove or check that `data` is valid,
non-null, aligned, or backed by `len` elements. The code supplying the pointer
and declared length owns that contract. A null data pointer may only be used in
ways the host/backend already permits; CK does not add a portable null model.

### Indexing

Slice indexing keeps the existing syntax:

```ck
let value: i64 = items[i];
items[i] = value;
```

The index must be `u32` or a non-negative integer literal materializable as
`u32`. Unlike raw `ptr<T>`, `i32` is not accepted for slice indexing. The result
is an assignable place of type `T`.

The descriptor expression is evaluated once, followed by the index expression.
An access such as `items[i].price` performs one logical slice index operation,
then an ordinary field operation on the selected element.

### Sub-slicing

The only first-phase range form is an explicit half-open range:

```ck
let middle: slice<i64> = items[start..end];
```

`start` and `end` must both be `u32` or materializable non-negative literals.
Omitted endpoints and first-class range values are not supported. The semantic
precondition is:

```text
start <= end <= items.len
```

The resulting descriptor is:

```text
data = items.data advanced by start elements
len  = end - start
```

The source slice is evaluated once, then `start`, then `end`. In checked mode,
validation occurs before pointer advancement or length subtraction. In
unchecked mode, violating the precondition is undefined behavior and remains
the CK program's responsibility.

For a zero start, the backend preserves the original data pointer instead of
requiring pointer arithmetic on a possibly null empty-slice pointer.

### Unsupported slice operations

Slices do not support arithmetic, ordering, equality, implicit pointer
conversion, implicit construction from `ptr<T>`, implicit element conversion,
concatenation, resizing, or iteration syntax. `.data` is the only conversion to
the existing raw pointer surface.

## Slice MIR

MIR gains `MirType::Slice(Box<MirType>)`. Slice-typed params, locals, temps, and
internal return values remain logical descriptor values in MIR even when a
backend later represents them as two physical values.

MIR adds explicit operations equivalent to:

```text
MakeSlice   target, data, len
SliceData   target, slice
SliceLen    target, slice
Subslice    target, slice, start, end
```

Raw pointer indexing keeps the existing `MirPlace::Index`. Slice indexing uses
a distinct `MirPlace::SliceIndex` that retains the slice descriptor and index,
so the backend still has access to both data and length. A field of a selected
struct element remains an ordinary `Field(SliceIndex(...), field)` place.

`Move`, slice-valued `Load` / `Store`, call arguments, and internal returns must
support exact-type descriptor copies. The MIR validator checks:

- slice elements are permitted storage types and are not direct slices
- `MakeSlice` receives `ptr<T>` and `u32` and produces `slice<T>`
- data and length projections match their descriptor
- slice indices and sub-slice endpoints are `u32`
- sub-slice input and result have the same slice type
- assignments and calls use exact slice element types
- only internal functions return a slice
- void never appears as a value or slice element

Bounds guards are not inserted into MIR. As with current checked arithmetic,
MIR preserves the semantic operation and the selected backend decides whether
to emit a guard. This keeps `emit-mir` independent of C's status ABI while still
making every checkable operation explicit.

## Physical Slice ABI

### Common parameter rule

Every semantic slice parameter is flattened, including parameters of internal
functions. Source:

```ck
export fn sum(values: slice<i64>) -> i64
```

has the physical parameter sequence:

```text
values_data: ptr<i64>, values_len: u32
```

A CK call still passes one logical argument. Each backend expands it to two
physical arguments. Generated names are deterministic and collision-safe;
backend name allocation disambiguates them from user parameter names.

### C

C uses a generated descriptor type for stored slice values and internal slice
returns:

```c
typedef struct CK_Slice_i64 {
  int64_t* data;
  uint32_t len;
} CK_Slice_i64;
```

Descriptor names are deterministically derived from the element type and
suffix-disambiguated against every user and generated C identifier. Generated
headers are the authoritative spelling for FFI consumers.

An exported slice parameter is flattened:

```c
CK_API int64_t sum(int64_t* values_data, uint32_t values_len);
```

An unchecked internal function returning a slice returns the descriptor struct
by value. Under status ABI, it returns `CK_Status` and receives a final
descriptor output pointer. Slice fields in public structs use the generated
descriptor type and therefore follow the target C compiler's natural pointer,
`uint32_t`, padding, and alignment rules. Required struct forward declarations
and descriptor declarations are emitted in dependency-safe order.

### WASM

WASM represents a descriptor as two `i32` values:

- data is an `i32` linear-memory byte offset
- length is an `i32` carrying the source `u32` bits

Slice parameters become two WASM parameters. Slice locals and temporaries are
represented by paired internal locals. An internal slice return uses a WASM
multi-value result `(i32, i32)`; exported slice returns remain forbidden.

In linear-memory structs, a slice field has deterministic size 8 and alignment
4: data at offset 0 and length at offset 4 relative to the field. A slice of
ordinary structs uses the existing deterministic WASM size and alignment of its
element type for pointer advancement.

### LLVM

LLVM represents a stored descriptor as `{ ptr, i32 }`. Slice parameters are
flattened to `ptr, i32`; internal slice returns use the aggregate descriptor
type. Construction and projection use ordinary aggregate operations, while
slice element access uses the existing target-aware `getelementptr` path with
the selected element type.

As with existing structs, the C ABI follows the host C compiler while WASM has
its documented deterministic layout. The source-level descriptor semantics are
the same even though physical layout is backend-specific.

## Bounds Mode and Status ABI

### CLI

The C-producing commands gain:

```text
--bounds <unchecked|checked>
```

The default is `unchecked`. Bounds and overflow are independent choices:

| Overflow | Bounds | C behavior |
| --- | --- | --- |
| unchecked | unchecked | current source-value ABI, no guards |
| checked | unchecked | status ABI and arithmetic guards |
| unchecked | checked | status ABI and slice bounds guards |
| checked | checked | status ABI with both guard classes |

`emit-wat`, `emit-wasm`, `emit-llvm`, and `build-llvm` accept only
`--bounds unchecked` in the first release. Passing `--bounds checked` produces
a stable error that directs users to `emit-c` or `build`. `check` and
`emit-mir` do not generate guards; source typing is independent of the selected
bounds mode. If a WASM or LLVM command requests both unsupported checked
overflow and checked bounds, the existing checked-overflow error is reported
first; flag order does not change that result.

### Module-wide C status ABI

If either overflow or bounds mode is checked, every function in the generated C
module uses `CK_Status`. This is intentionally module-wide, matching current
checked arithmetic and preventing function signatures from changing when a
function body gains or loses an indexed access.

The status constants are stable and emitted together whenever status ABI is
active:

```c
#define CK_OK                    ((CK_Status)0)
#define CK_ERR_OVERFLOW          ((CK_Status)1)
#define CK_ERR_DIV_BY_ZERO       ((CK_Status)2)
#define CK_ERR_NULL_POINTER      ((CK_Status)3)
#define CK_ERR_OUT_OF_BOUNDS     ((CK_Status)4)
```

For a non-void source return, status ABI appends the existing `ck_return`
pointer. For a void source return, it appends no result pointer. An internal
slice return uses a generated descriptor pointer as `ck_return`. A non-void
function validates `ck_return` at entry, before evaluating its source body, so
`CK_ERR_NULL_POINTER` takes precedence over later arithmetic or bounds errors.
Void functions have no generated return pointer to validate.

### Guard coverage

With `--bounds checked`, C emits a guard for:

- every logical read or write through `slice[index]`
- every sub-slice creation

An index is valid exactly when `index < slice.len`. A sub-slice is valid exactly
when `start <= end && end <= slice.len`. Failure immediately returns
`CK_ERR_OUT_OF_BOUNDS` through the normal status propagation path.

`slice(data, len)` never emits a pointer-validity or backing-length check. Raw
`ptr<T>` indexing and indexing through `slice_value.data` never emit a slice
guard. The compiler does not promise that bounds-checked code is safe when the
descriptor itself lies about its memory.

Each source operand is evaluated once. Arithmetic used to calculate an index,
start, or end executes before its bounds guard. If both overflow and bounds are
checked, an arithmetic failure observed while calculating the operand takes
precedence over the later bounds result. Call failures similarly propagate at
the point of evaluation.

The first implementation emits checks conservatively. `-O1` through `-O3` must
not delete or move a bounds guard without a proof that preserves error order and
all checked semantics.

## Diagnostics and Error Recovery

New source errors include:

- `break` or `continue` outside a loop
- unreachable statements after a non-fallthrough statement
- void used outside a function return position
- missing or unexpected return values
- a void call used as a value
- a non-void call used as a standalone statement
- invalid slice element types or direct nested slices
- non-`u32` slice indices, lengths, or endpoints
- assignment to `.data` or `.len`
- invalid `slice(data, len)` operands
- invalid sub-slice operands
- an exported slice return

These are checker diagnostics with source spans, not MIR-lowering failures or
backend panics. The checker gains explicit diagnostic-code entry points for the
new control-flow, void, and slice categories instead of extending the existing
message-prefix mapping indefinitely. Existing diagnostic codes and formatting
remain stable.

Invalid CLI bounds combinations are command errors and do not receive source
diagnostic codes.

## Optimizer Requirements

Every MIR pass must understand `MirType::Void`, `MirType::Slice`, optional call
targets, optional return values, and the use-def edges of slice instructions.

- Void produces no value and never participates in constant folding or CSE.
- A slice descriptor move is a value copy, not a copy of underlying elements.
- Slice construction and projection may only be simplified when data and length
  evaluation order is preserved.
- Sub-slice operations remain explicit through backend emission in the first
  checked implementation.
- Raw and slice index places remain distinct.
- Control-flow simplification may redirect break/continue-generated jumps just
  like existing jumps, but must preserve their targets and reachability.
- Loop optimizations must treat bounds errors as observable in checked mode.

The first slice release does not require bounds-check elimination. A later
design may add proof-based elimination or loop hoisting with dedicated checked
mode tests.

## Testing Strategy

### Phase A

- Lexer tests for reserved keywords and exact spans.
- Parser tests for both statements and required semicolons.
- Checker tests for loop placement, nested loops, branch placement, and
  unreachable statements.
- MIR snapshots proving innermost condition/exit jump targets.
- C, WASM, and LLVM runtime tests for early exit, skipped iterations, nesting,
  and combinations with `return`.
- O0 through O3 coverage, including WASM dispatcher fallback.

### Phase B

- Parser and checker tests for `-> void`, `return;`, natural fallthrough, and
  invalid void positions.
- Tests proving that only void calls are legal statements.
- MIR printer and validator tests for targetless calls and valueless returns.
- C header/source golden tests for unchecked void and status-ABI void.
- WASM tests proving no result declaration and correct internal calls.
- LLVM tests proving `void`, `call void`, and `ret void`.
- Runtime tests for void functions that mutate caller-owned buffers.

### Phase C

- Lexer/parser tests for `slice<T>`, `slice(data, len)`, and `start..end`.
- Checker tests for every allowed position, exact element matching, read-only
  projections, `u32` rules, direct nesting rejection, and exported-return
  rejection.
- MIR snapshots and validator-negative tests for every slice operation.
- C golden tests for descriptor declarations, flattened parameters, internal
  returns, void/status combinations, and collision-safe names.
- WASM layout, multi-value internal return, call, field, load/store, and runtime
  tests.
- LLVM aggregate, flattened-parameter, internal-return, GEP, and runtime tests.
- Cross-backend runtime parity for valid unchecked indexing and sub-slicing.
- Checked C runtime tests for index equal to length, very large `u32` indices,
  reversed ranges, end beyond length, valid empty ranges, nested calls, void
  callers, and combined overflow/bounds error ordering.
- Tests proving `.data` and raw `ptr<T>` remain unchecked.
- CLI tests for defaults, invalid values, supported C combinations, and explicit
  WASM/LLVM rejection.

The legacy TypeScript oracle does not understand the new syntax. New language
features therefore use direct semantic expectations and C/WASM/LLVM runtime
parity rather than pretending to have TypeScript oracle coverage. Existing V0
oracle fixtures remain unchanged and must continue to pass.

## Documentation and Migration

Each phase updates the durable English documents and matching Simplified
Chinese documents required by repository policy:

- `LANGUAGE_SPEC.md`
- `COMPILER_ARCHITECTURE.md`
- `MIR.md`
- `ABI.md`
- `CHECKED_ARITHMETIC.md`
- `WASM_ABI.md`
- `LLVM_BACKEND.md`
- `ROADMAP.md`
- relevant README links and examples

Migration notes call out the four newly reserved words. Existing artificial
integer-return APIs and pointer-plus-length APIs are not rewritten silently.
Projects can adopt void and slice signatures deliberately and treat each
exported signature change as an ABI change.

## Delivery Gates

Each phase follows red-green development through lexer, parser, checker, MIR,
backend, CLI, runtime, and documentation contract tests. A phase is complete
only when:

- focused feature tests pass
- `cargo fmt --check` passes
- strict Clippy passes for all targets and features
- the complete locked test suite passes
- release build and CLI smoke tests pass
- C, WASM, and LLVM support the phase's promised mode matrix
- English and Chinese formal documents agree
- existing V0 fixtures and generated ABI tests remain green

Phase C does not start until the control-flow analysis from Phase A and the void
MIR/ABI from Phase B are stable, because checked slice calls depend on both.

## Success Criteria

- CK authors can leave or skip an iteration without returning from the function
  or maintaining a manual flag.
- Procedures can use `-> void` without artificial source or ABI return values.
- Void calls are explicit statements and value results cannot be discarded by
  accident.
- A slice descriptor can be constructed, copied, stored, forwarded, indexed,
  sub-sliced, projected, and internally returned with exact types.
- Slice parameters have the documented flattened pointer-plus-`u32` ABI.
- Valid unchecked slice programs behave consistently across C, WASM, and LLVM.
- C checked bounds returns `CK_ERR_OUT_OF_BOUNDS` without trapping and propagates
  through void, value, and internal slice-returning calls.
- Raw pointer construction and `.data` remain explicit trust boundaries.
- WASM and LLVM reject checked bounds clearly rather than silently omitting
  guards.
- The MIR printer, validator, optimizer, and every backend understand the new
  types and operations without naming conventions or backend panics.
