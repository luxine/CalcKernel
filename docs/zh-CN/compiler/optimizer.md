# CalcKernel V0.9 Optimizer

[English](../../compiler/optimizer.md)

本文规范 optimization level 与 preservation requirement，并解释 algorithm。
`--opt-level 0|1|2|3` / `-O0`–`-O3` 选择所有 backend 共享的 MIR pipeline。

- O0：仅 validation。
- O1：constant folding、copy propagation、dead-code elimination、CFG simplify。
- O2：加入 small-function inline、local CSE、address CSE 与 cleanup。
- O3：再加入 loop analysis、loop-invariant code motion、induction hook 与 cleanup。

`--print-pass-pipeline` 的稳定 pass name 是 `constant-folding`、
`copy-propagation`、`dead-code-elimination`、`cfg-simplify`、
`inline-small-functions`、`local-cse`、`address-cse`、`loop-analysis`、
`loop-invariant-code-motion`、`induction-simplify`；顺序由所选 pipeline 决定。
每个 pass 后都 validation MIR。

Constant folding 排除 overflowing integer、divide/modulo by zero，以及会改变
NaN、infinity、signed zero 或 operand order 的 `f64` algebra。Call 与 targetless
void call 的 side effect 必须保留；CFG pass 保留 valueless return 与最内层
`break`/`continue` target。

`slice<T>` descriptor 按 value copy，data 仍 alias。Checked `SliceIndex`/`Subslice`
guard 与 address calculation 可观察，并且只存在于 C `--bounds checked` context；
除非在 active context 中证明安全，否则不可
删除，也不可跨可能失败的 call/arithmetic CSE、hoist 或 reorder。

`--print-mir-before-opt`、`--print-mir-after-opt`、`--print-pass-pipeline`
向 stderr 写确定信息且不改变 artifact。
