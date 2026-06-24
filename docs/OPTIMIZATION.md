# Optimization

[简体中文](zh-CN/OPTIMIZATION.md)

IntKernel exposes a compiler optimization level so MIR passes can be enabled
consistently across C, WASM, LLVM, and MIR debug output.

Phase 14.4 adds option plumbing. Phase 14.5 adds the MIR pass manager framework.
Phase 14.6 adds conservative local MIR passes. Phase 14.7 adds CFG
simplification. Phase 14.8 through Phase 14.10 add local CSE, address CSE,
small-function inlining, and conservative loop analysis. Phase 14.12 adds a
checked C hot-path optimization for proven-safe loop induction increments.
Phase 14.13 adds WASM hot-path lowering for simple while loops and indexed
address reuse. Phase 14.14 adds LLVM build optimization flags and a small
SSA-like LLVM lowering path for simple scalar straight-line functions.

## CLI

All code generation commands accept `--opt-level`:

```sh
ikc emit-mir examples/pricing.ik --opt-level 0
ikc emit-c examples/pricing.ik --out build/pricing.c --header build/pricing.h --opt-level 3
ikc build examples/pricing.ik --out build/libpricing --opt-level 3
ikc emit-wat examples/pricing.ik --out build/pricing.wat --opt-level 3
ikc emit-wasm examples/pricing.ik --out build/pricing.wasm --opt-level 3
ikc emit-llvm examples/pricing.ik --out build/pricing.ll --opt-level 3
ikc build-llvm examples/pricing.ik --out build/libpricing --opt-level 3
```

The `-O` aliases are equivalent:

```sh
ikc emit-c examples/pricing.ik --out build/pricing.c --header build/pricing.h -O3
```

The default is `-O0`.

## Levels

### `-O0`

No MIR optimization.

`-O0` runs validator-only code generation. It keeps compiler output closest to
the lowered MIR and is the default for Phase 14. It is the baseline for
debugging and snapshot review.

### `-O1`

Cheap local optimizations.

`-O1` currently runs:

- constant folding
- copy propagation
- dead code elimination (DCE)
- simple CFG cleanup, limited to unreachable block removal

### `-O2`

Standard optimization pipeline.

`-O2` currently runs `-O1` plus:

- full CFG simplification
- local CSE
- address CSE
- small-function inlining with the O2 threshold
- repeated cleanup passes after inlining and CSE

### `-O3`

Aggressive optimization pipeline.

`-O3` enables the `-O2` pipeline plus:

- more aggressive small-function inlining with the O3 threshold
- conservative basic loop analysis
- conservative loop-invariant code motion
- induction simplification metadata and checked C induction proof support
- C and LLVM native build commands using clang `-O3`
- WASM hot-path lowering for simple while loops

Optional CPU-native and LTO controls are reserved for future work. They are not
enabled by default, and Phase 14 does not add unsafe target-specific flags.
`-O3` must still preserve the selected overflow mode, ABI, and language
semantics.

## Pass Manager

The MIR pass manager is the shared optimization entry point. It operates after
MIR lowering and before backend emission:

```text
CheckedProgram
  -> MIR lowering
  -> MIR pass manager
  -> MIR validator
  -> backend
```

Each pass implements:

```ts
interface MirPass {
  name: string;
  run(module: MirModule, context: MirPassContext): MirPassResult;
}
```

The pass context carries:

- optimization level
- overflow mode
- target backend
- debug flags

Pass results report whether the pass changed MIR and may include diagnostics.
The manager records pass order and changed state, and can run the MIR validator
after every pass.

Current pipelines:

- `-O0`: validator only
- `-O1`: constant folding -> copy propagation -> dead code elimination -> CFG
  simplification
- `-O2`: constant folding -> copy propagation -> local CSE -> copy
  propagation -> address CSE -> dead code elimination -> CFG simplification ->
  dead code elimination
- `-O3`: constant folding -> copy propagation -> small-function inlining ->
  constant folding -> copy propagation -> loop analysis -> loop-invariant code
  motion -> induction simplification -> constant folding -> copy propagation ->
  local CSE -> copy propagation -> address CSE -> dead code elimination -> CFG
  simplification -> dead code elimination

## Passes

### Constant Folding

The constant folding pass folds pure constant expressions inside unchecked MIR.
It supports integer `+`, `-`, `*`, `/`, `%`, comparisons, unary `-`, unary `!`,
and bool constants.

Safety boundaries:

- disabled in checked overflow mode
- does not fold divide or modulo by zero
- does not fold signed min / -1 division or modulo
- does not fold integer operations whose result is outside the target integer
  type range
- does not fold memory access, stores, calls, or control-flow effects

### Copy Propagation

The copy propagation pass replaces simple temp copies inside a basic block.

Safety boundaries:

- does not propagate lvalue places
- does not cross `store`
- does not cross `call`
- only rewrites MIR value uses and index expressions

### Dead Code Elimination

The dead code elimination pass removes unused pure temp definitions.

It may remove unused:

- `const_int`
- `const_bool`
- pure `move` to temp
- pure `binary`
- pure `unary`
- pure `compare`

It does not remove:

- `store`
- `call`
- `load`
- terminators
- branch conditions
- return values
- local assignments

### Local CSE

The local common subexpression elimination pass works inside one basic block.

It can reuse:

- pure binary arithmetic
- pure unary expressions
- comparisons

Safety boundaries:

- does not do global CSE
- does not CSE ordinary loads
- clears its table at `store` and `call`
- invalidates expressions that depend on a reassigned local
- relies on later copy propagation and DCE to clean up replacement moves

### Address CSE

The address CSE pass is currently enabled for the C backend at `-O2` and `-O3`.
It recognizes indexed places in one basic block and hoists the address into a
pointer temp. This includes repeated `ptr<Struct>[i].field` reads and scalar
indexed stores such as `out[i] = value`.

Example generated C shape:

```c
Item* ik_tmp_addr0;

ik_tmp_addr0 = &items[i];
ik_tmp0 = ik_tmp_addr0->price;
ik_tmp1 = ik_tmp_addr0->qty;
```

Scalar store example:

```c
int64_t* ik_tmp_addr1;

ik_tmp_addr1 = &out[i];
(*ik_tmp_addr1) = ik_tmp11;
```

Safety boundaries:

- does not eliminate or cache loads
- does not do alias analysis
- clears address state at `store` and `call`
- invalidates address entries that depend on a reassigned local
- does not specialize for `Item` or `pricing.ik`

### WASM Hot Path Lowering

The WASM backend keeps the original block-dispatcher fallback for arbitrary
MIR CFGs. At `-O3`, it recognizes a simple while-loop shape:

```text
entry -> condition
condition -> body | exit
body -> condition
exit -> return
```

For that shape, WAT is emitted as structured `block` + `loop` instead of the
generic dispatcher with `br_table` and an `$ik_bb` local. This reduces branch
dispatch traffic in kernels such as `examples/pricing.ik`.

The `-O3` WASM pipeline also enables address CSE, so repeated
`ptr<Struct>[i].field` loads reuse the indexed base address:

```wat
local.get $items
local.get $i
i32.const 32
i32.mul
i32.add
local.set $addr0

local.get $addr0
i64.load offset=0 align=8
```

Fallback behavior:

- complex control flow still uses the dispatcher
- short-circuit CFGs keep their RHS blocks isolated
- no bounds check is added
- no checked WASM arithmetic is added

### LLVM Hot Path Lowering

The LLVM backend normally emits conservative alloca/load/store IR so it can
cover MIR control flow, calls, and memory operations uniformly. At `-O2` and
`-O3`, it can bypass stack slots for a very small class of functions:

- single basic block
- no `let` locals
- no branch, jump, or loop
- no function call
- no load/store/address operation
- only constants, moves, scalar arithmetic, comparisons, unary operators, and
  return

For this shape, the LLVM backend emits direct SSA-like operations:

```llvm
define i64 @add_i64(i64 %a, i64 %b) {
entry:
  %v0 = add i64 %a, %b
  ret i64 %v0
}
```

More complex functions, including `examples/pricing.ik`, continue to use
stack lowering. Clang `-O2`/`-O3` can then promote many stack slots and optimize
the resulting native code. The backend still does not emit unsafe `nsw`/`nuw`
flags, does not add bounds checks, and does not support checked LLVM arithmetic.

`build-llvm` now passes the selected IK optimization level through to clang as
`-O0`, `-O1`, `-O2`, or `-O3`.

### Checked C Induction Optimization

Checked arithmetic normally emits overflow checks for every integer `+`, `-`,
and `*`, and emits division checks for `/` and `%`.

At `-O3`, the C backend may emit a plain `i + 1` for a loop induction increment
only when a very conservative proof succeeds:

- `i` is initialized to literal `0`
- the loop condition is exactly `i < len`
- `i` and `len` are both `i32` or both `u32`
- `i` is updated in the loop only as `i = i + 1`
- `len` is not modified in the loop
- the loop body jumps directly back to the condition block

If any condition is not met, the checked backend keeps
`__builtin_add_overflow`.

Checks intentionally kept:

- business arithmetic such as `price * qty`
- discount, tax, and amount addition/subtraction
- division by zero checks
- signed min / `-1` checks
- unary minus overflow checks
- any induction update that is not proven safe by the exact rule above

### CFG Simplification

The CFG simplification pass rewrites basic-block structure without moving
ordinary instructions.

At `-O1`, it only removes unreachable blocks.

At `-O2` and `-O3`, it also:

- rewrites constant branches to direct jumps
- rewrites jumps through empty jump-only blocks to their final target
- removes empty jump-only blocks after their predecessors have been redirected

Safety boundaries:

- does not move instructions across basic-block boundaries
- does not duplicate instructions
- does not evaluate RHS blocks for `&&` or `||` early
- keeps MIR validation enabled after the pass

## Debug Flags

The CLI exposes MIR optimization debug flags:

```sh
ikc emit-mir examples/pricing.ik -O3 --print-pass-pipeline
ikc emit-mir examples/pricing.ik -O3 --print-mir-before-opt
ikc emit-mir examples/pricing.ik -O3 --print-mir-after-opt
```

Debug output is written to stderr so stdout can remain a stable artifact stream
for commands such as `emit-mir`, `emit-wat`, and `emit-llvm`.

## Phase 14 Final Behavior

In Phase 14, `-O0` remains validator-only and keeps generated output closest to
the lowered MIR. `-O1` enables cheap local cleanup. `-O2` enables the standard
optimization pipeline. `-O3` enables the most aggressive currently implemented
pipeline and passes the selected native optimization level through to C and
LLVM build commands.

The pass set is still intentionally conservative. In checked mode,
`examples/pricing.ik` keeps all business overflow and division checks; only the
loop counter increment can be emitted as unchecked arithmetic after the proof
succeeds. WASM and LLVM remain unchecked-only backends and reject
`--overflow checked`.

## Release Guidance

Before releasing optimization changes:

- run `pnpm test`, `pnpm typecheck`, and `pnpm build`
- run `node --test bench/perf/tests/perf-core.test.mjs`
- run `node bench/perf/run.mjs --quick`
- if a local baseline exists, run `node bench/perf/run.mjs --quick --compare`
- run a full performance pass before important tags when machine time allows
- review MIR, C, WAT, LLVM, and benchmark summary diffs for accidental
  benchmark-specific behavior
