# Implementation blocker 26: compact multiversion helper inlining

Date: 2026-09-07

## Finding

Exact V0.13 run `34041456107`, AArch64 performance job `101509032163`,
passed the unchanged cumulative schema-7 gate but failed schema 8 because
dispatch geometric improvement was `1.01545`, below the required `1.08`.
The eligible ordinary-to-multiversion median ratios were `1.00109` for
call-constant-length, `1.00117` for trip-unroll-simd, `1.00261` for
memory-bound, and `1.05810` for compute-bound. All sample distributions were
stable, the resolver ran exactly once, and the selected direct member was the
actual SVE compatibility companion. Rerunning the same bytes could not pass.

V0.14 run `34041903921`, AArch64 performance job `101510076457`, reproduced
the same exact V0.13 failure while preparing its historical schema-8 replay.
It did not expose an independent V0.14 defect. No threshold, workload, sample
count, statistic, platform, or required job may change.

## Rediagnosis

The GitHub-hosted Neoverse N2 worker has 128-bit SVE2. Same-width memory and
recurrence workloads cannot by themselves guarantee a large target-only SVE
advantage. The fixed call-constant-length workload supplied the intended
independent source of benefit, but ordinary and multiversion O3 both fully
inlined its small hot helper and medium cold helper. LLVM then if-converted the
two arms, so the held-out all-hot input still executed the cold arithmetic.

KIR had no inline policy reflecting multiversion clone amplification. Retaining
the medium helper at KIR alone was insufficient because LLVM could inline it
again independently in every baseline and enhanced member.

## Resolution

Ordinary static O3 retains its 32-instruction pure-helper inline budget.
Unprofiled multiversion compilation uses a compact eight-instruction budget:
the small hot helper remains inline, while a medium branch helper remains a
call. Native lowering deterministically marks a still-called, non-exported,
memory-free helper of 9 through 32 supported KIR instructions `noinline`, so
the target optimizer cannot undo the checked KIR clone decision. PGO-proven hot
inlining retains its existing 48-instruction budget.

The rule is based only on instruction count, effects, visibility, and call
reachability. It does not inspect function names, fixture paths, profile
contents, inputs, or host identity. The object-affecting multiversion cache
identity adds `compact-multiversion-inline-v2`.

A RED/GREEN optimizer regression proves ordinary O3 still inlines both helpers
while multiversion inlines the compact helper and preserves the medium helper.
A Native regression isolates the object cache, builds a real dynamic library,
and verifies the medium symbol remains while the compact symbol disappears.
On the local AArch64 Darwin diagnostic, the unchanged all-hot held-out input
improved from an ordinary median of 611,542 ns to a multiversion median of
540,625 ns, a ratio of `1.13118`. The exact Linux AArch64 CI gate remains the
authority.

No CK language or public ABI, strict floating or safety rule, target ISA,
candidate frontier, profitability floor, code-growth budget, performance or
stability threshold, timed work, sample count, corpus, platform, or required
CI job changed.

## Verdict

Accepted implementation blocker. Focused and complete local verification must
pass before pushing a new exact candidate SHA. The real AArch64 performance job
remains authoritative, and every other required job remains mandatory.
