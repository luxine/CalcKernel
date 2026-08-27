# CalcKernel 0.10 Optimizer

[简体中文](../zh-CN/compiler/optimizer.md)

This document normatively fixes optimization-level selection and preservation
requirements; algorithm descriptions are explanatory. `--opt-level 0|1|2|3`
and `-O0`–`-O3` select one MIR pipeline shared by all backends.

## Pipelines

- O0: validation only.
- O1: constant folding, copy propagation, dead-code elimination, CFG simplify.
- O2: O1 foundations plus small-function inlining, local CSE, address CSE, and
  cleanup passes.
- O3: O2 capabilities plus loop analysis, loop-invariant code motion, induction
  analysis/simplification hooks, and final cleanup.

The exact pass names printed by `--print-pass-pipeline` are
`constant-folding`, `copy-propagation`, `dead-code-elimination`, `cfg-simplify`,
`inline-small-functions`, `local-cse`, `address-cse`, `loop-analysis`,
`loop-invariant-code-motion`, and `induction-simplify`, in the order constructed
by the selected pipeline. MIR is validated after each pass.

Constant folding excludes overflowing integers, integer division/modulo by zero,
and strict `f64` algebra that could change NaN, infinity, signed zero, or operand
order. Calls and targetless void calls retain side effects. CFG passes preserve
valueless returns and innermost-loop `break`/`continue` targets.

`slice<T>` descriptors are copied as values while their data remains aliased.
Checked `SliceIndex`/`Subslice` guards and address computation are observable in
C and Native `--bounds checked` contexts. Passes may remove a guard only when
safety is proven under the active context,
and may not CSE, hoist, or reorder it across a possibly failing call/arithmetic
operation. Index/place analysis tracks slice data and length uses together.

Native print calls are runtime effects. Optimization may remove only unreachable
ones and preserves the count and source order of reachable effects through
calls, loops, and inlining. One selected level controls MIR and the subsequent
LLVM default pipeline; no level enables fast-math.

`--print-mir-before-opt`, `--print-mir-after-opt`, and
`--print-pass-pipeline` write deterministic diagnostics to stderr and do not
alter artifacts.
