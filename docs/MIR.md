# CalcKernel MIR

[简体中文](zh-CN/MIR.md)

MIR is CalcKernel's middle-level intermediate representation. It sits after type
checking and before backend-specific code generation. Its job is to lower the
Typed AST into a typed, normalized structure that is easier for C, WASM, LLVM,
and future checked code generation to consume.

## Goal

The default C codegen pipeline is:

```text
.ck source
  -> lexer
  -> parser
  -> AST
  -> type checker
  -> Typed Program
  -> MIR
  -> C backend
  -> .c / .h
```

Backends use the same MIR rather than each reinterpreting the AST:

```text
MIR -> MIR pass manager at selected optimization levels -> C
MIR -> MIR pass manager at selected optimization levels -> WASM
MIR -> MIR pass manager at selected optimization levels -> LLVM
```

MIR v1 must preserve the current source language semantics. It is an
architecture layer, not a language feature.

As of Phase 11.15, `ckc emit-c` and `ckc build` use this MIR pipeline by
default for both unchecked and checked C generation. The legacy AST-to-C backend
remains available internally for regression comparison and fallback.

## Non-Goals

MIR v1 deliberately does not add:

- SSA
- register allocation
- bounds check
- runtime
- new language features

Phase 14 adds a conservative MIR pass manager with documented O0/O1/O2/O3
pipelines. Those passes operate on MIR v1, preserve the selected overflow mode
and backend ABI, and are documented in [Optimization](OPTIMIZATION.md).

MIR v1 should make later work easier, but it must not change V0 behavior,
unchecked ABI, checked ABI, or diagnostics semantics.

## MIR v1 Scope

MIR v1 covers the current V0 language surface:

- scalar integer and boolean expressions
- strict `f64` expressions
- exact explicit `i32_to_f64` / `u32_to_f64` casts
- `let`, assignment, and `return`
- `if` / `else`
- `while`
- function calls
- short-circuit logical operators
- pointer indexing
- struct field access
- load and store through typed places

It is intentionally a normalization layer. It does not introduce new syntax,
new types, new runtime behavior, or new safety checks.

## Core Structure

MIR v1 is typed, three-address, and basic-block based.

```text
MirModule
  structs: MirStruct[]
  functions: MirFunction[]

MirFunction
  name: string
  exported: bool
  params: MirParam[]
  returnType: MirType
  locals: MirLocal[]
  blocks: MirBlock[]

MirBlock
  label: string
  instructions: MirInstruction[]
  terminator: MirTerminator
```

### Instructions

MIR v1 instructions should describe simple operations with explicit result
values or explicit places:

- `ConstInt`
- `ConstFloat`
- `ConstBool`
- `Move`
- `Binary`
- `Unary`
- `Compare`
- `Cast`
- `Load`
- `Store`
- `Call`

Arithmetic and comparison operations should be three-address operations. For
example:

```text
%t0: i64 = add a, b
%t1: bool = lt %t0, c
%t2: f64 = cast i32_to_f64 i
```

The only MIR cast operations today are `i32_to_f64` and `u32_to_f64`. MIR
validator rejects all other cast kinds, wrong input types, and wrong result
types.

### Terminators

Every block ends with one terminator:

- `Return`
- `Jump`
- `Branch`

Terminators own control flow. Ordinary instructions should not implicitly jump
to another block.

### Places

`MirPlace` represents a readable or writable storage location:

- `Local`
- `Param`
- `Index`
- `Field`

Examples:

```text
local sum
param items
index items, i
field (index items, i), price
```

## Types

Every MIR value must have a resolved CalcKernel type:

- `i32`
- `i64`
- `u32`
- `u64`
- `f64`
- `bool`
- `ptr<T>`
- `struct`

MIR should not contain parser-only types such as `NamedTypeNode` or unresolved
type nodes. The type checker must resolve names before lowering.

Integer literals should be materialized to the concrete type chosen by the type
checker. MIR should not need the AST-only `integerLiteral` pseudo-type.

## Control Flow

MIR v1 lowers structured control flow into basic blocks.

An `if / else` lowers to a condition value and a `Branch`:

```text
bb0:
  %cond: bool = lt a, b
  branch %cond, bb_then, bb_else

bb_then:
  return a

bb_else:
  return b
```

A `while` lowers into a condition block, body block, and exit block:

```text
bb0:
  jump bb_cond

bb_cond:
  %cond: bool = lt i, len
  branch %cond, bb_body, bb_exit

bb_body:
  ...
  jump bb_cond

bb_exit:
  return 0
```

The builder should preserve source-level evaluation order.

## Function Call Lowering

Function calls lower to explicit `Call` instructions with a result value:

```text
%t0: i64 = call add_i64(a, b)
```

Nested calls are lowered inside out while preserving source argument order:

```ck
double_i64(add_i64(a, b))
```

becomes conceptually:

```text
%t0: i64 = call add_i64(a, b)
%t1: i64 = call double_i64(%t0)
```

Argument expressions are evaluated left to right before the call instruction is
emitted. Calls used only as statements still produce a temporary result so the
call is represented explicitly; MIR v1 does not delete unused calls.

The MIR validator checks that the callee exists, argument count and types match,
and the call result type equals the callee return type.

## Short-Circuit Logic

`&&` and `||` must not lower into ordinary `Binary` instructions. They must
lower into control flow so the right-hand side is evaluated only when required.

For `a && b`:

- evaluate `a`
- if `a` is false, result is false
- otherwise evaluate `b`

For `a || b`:

- evaluate `a`
- if `a` is true, result is true
- otherwise evaluate `b`

This matters in checked mode because a skipped right-hand side must not trigger
overflow or division-by-zero checks.

## Lvalues and Rvalues

MIR v1 distinguishes places from values.

Reading:

```ck
items[i].price
```

is represented as a `Load` from a field place:

```text
%idx: i32 = move i
%tmp: i64 = load field(index(items, %idx), price)
```

Writing:

```ck
out[i] = value;
```

is represented as a `Store` to an index place:

```text
%idx: i32 = move i
store index(out, %idx), value
```

Index expressions can contain arithmetic. The index expression itself lowers to
MIR values before the final `Index` place is formed.

## Pointer, Index, and Field Lowering

Pointer, index, and field access is represented through places.

Reading:

```ck
items[i].price
```

lowers by evaluating the index expression first, then loading from the field
place:

```text
%idx: i32 = move i
%value: i64 = load field(index(items, %idx), price)
```

Writing:

```ck
out[i] = value;
```

lowers to a store:

```text
%idx: i32 = move i
store index(out, %idx), value
```

For a compound index:

```ck
items[i + 1].price
```

the arithmetic is lowered before the place is formed:

```text
%idx: i32 = add i, 1
%value: i64 = load field(index(items, %idx), price)
```

MIR v1 still does not add bounds checks, pointer validity checks, or buffer
length checks. Those require future language-level metadata such as `slice<T>`.

## Checked Mode Relationship

MIR v1 expresses ordinary CalcKernel arithmetic semantics. It does not directly
encode overflow checks.

The backend decides how to emit arithmetic based on the requested overflow mode:

- `unchecked`: generate ordinary C operations
- `checked`: generate overflow guards, division checks, `CK_Status`
  propagation, and checked return-pointer handling

This keeps MIR independent from one backend's checked C implementation while
still making checked lowering easier to implement consistently.

Short-circuit operators are already represented as MIR control flow, so checked
C emission does not need a separate logical-operator special case that could
accidentally evaluate the right-hand side too early. Function calls are explicit
MIR `Call` instructions, so checked C emission can insert `CK_Status` propagation
at each call site.

Pointer/index/field access in checked mode uses the same MIR places. Arithmetic
inside index expressions is checked because it is represented as ordinary MIR
arithmetic before the final place is used. MIR does not imply bounds checks.

## Why MIR v1 Is Not SSA

MIR v1 deliberately avoids SSA so the project can share code generation across
backends without also introducing phi nodes and dominance rules. The current
language has mutable locals and structured loops; representing them as basic
blocks with explicit moves and stores is enough for readable C emission,
WASM/LLVM lowering, and backend parity testing.

This keeps MIR snapshots easy to read. Phase 14 optimizations remain
conservative MIR-to-MIR passes rather than an SSA rewrite.

## Future SSA and Optimizer Work

Future phases can introduce an SSA-based IR or broader optimizer work above MIR
v1. Candidate future work includes:

- range analysis for future checked or bounds-safe features
- lowering to backend-specific SSA for LLVM or WASM
- broader f64 optimization only after strict-safe floating point optimization
  rules are explicitly designed
- optional CPU-native or LTO experiments outside default builds

Those passes should be added only after MIR v1 remains stable as the default C
pipeline and after their effect on diagnostics, snapshots, and generated ABI is
well understood.

## Text Format

MIR should have a stable text format for snapshots. The format should avoid
absolute paths, timestamps, random IDs, or platform-specific line endings.

Example:

```text
fn add_i64(a: i64, b: i64) -> i64 {
bb0:
  %t0: i64 = add a, b
  return %t0
}
```

Example with a branch:

```text
export fn max_i32(a: i32, b: i32) -> i32 {
bb0:
  %t0: bool = gt a, b
  branch %t0, bb1, bb2

bb1:
  return a

bb2:
  return b
}
```

The printer should make temporary names stable, for example `%t0`, `%t1`, and
block labels such as `bb0`, `bb1`, `bb2`.

## Phase 11 Migration Strategy

Phase 11 is incremental. The AST backend stays available as a legacy/internal
path while MIR code generation is validated by snapshots and e2e tests.

1. Add MIR types, printer, and validator.
2. Add Typed AST to MIR lowering.
3. Add MIR to C unchecked backend.
4. Add MIR to C checked backend.
5. Switch the default C backend to MIR after generated C snapshots and e2e
   tests match the intended output. Completed in Phase 11.15.
6. Keep or remove the old AST backend only after the MIR backend covers the full
   current V0 surface and has had a release cycle as the default.

Each step should preserve existing CLI behavior, generated ABI, diagnostics, and
test expectations unless a deliberate snapshot update is reviewed.
