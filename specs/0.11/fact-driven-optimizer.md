# CK 0.11 Fact-Driven Optimizer Specification

[简体中文](zh-CN/fact-driven-optimizer.md)

This is the pre-release specification for the CK 0.11 optimizer foundation. It
does not alter the released 0.10 language, CLI, MIR, or ABI contracts. When 0.11
is implemented and released, its durable requirements move into the current
bilingual documentation tree and this pre-release specification is removed.

## Objective

CK will not treat LLVM as its only source of optimization knowledge. The
compiler will derive and preserve facts that ordinary C and Rust compilation
often cannot recover reliably: value ranges, memory regions, aliasing, access
effects, alignment, slice bounds, loop iteration ranges, and call effects. CK
will prove transformations in its own target-neutral optimizer and pass only
verified facts to backend toolchains.

The long-term performance target is evaluated for the same algorithm, safety
semantics, observable behavior, and hardware. Eligible kernels should approach
fixed, audited hand-written C/Rust plus SIMD implementations. Kernels whose CK
contracts expose domain constraints unavailable to a generic compiler should
beat the fixed generic Clang/Rust O3 references. No target is met by weakening
strict floating-point behavior, checked first-error order, print order, ABI, or
the declared contract domain.

## Selected architecture

The compiler uses an evidence-first, unified optimization IR:

```text
AST and checked source contracts
              |
              v
       semantic MIR
              |
              v
 KIR builder and verifier
              |
              v
KIR: scalar SSA + region Memory SSA + facts + effects + proofs
              |
              v
       KIR optimizer
        /      |      \
       C     WebAssembly  Native LLVM
```

Semantic MIR continues to own source evaluation order, checked first-error
order, runtime-print order, frontend-independent meaning, and the stable
`emit-mir` text. KIR is the single internal representation for target-neutral
analysis and optimization. The C, WebAssembly, and Native LLVM backends all
consume verified KIR before 0.11 can ship. A development-only shadow pipeline
may compare KIR with the 0.10 path during migration, but the release cannot
retain two permanent optimization pipelines.

KIR text is deterministic and available for inspection, but is an internal
compiler format without cross-version compatibility guarantees.

## Trusted source contracts

### Syntax and unsafe boundary

Trusted facts are attached only to an `unsafe fn` entry:

```ck
export unsafe fn saxpy(
    x: slice<f64>,
    y: slice<f64>,
    n: u32
) -> void
contract {
    requires n <= x.len && n <= y.len;
    requires noalias(x, y);
    requires aligned(x.data, 32);
    effects read(x), write(y);
}
{
    // function body
}
```

Every call to an unsafe function must appear inside an explicit unsafe statement
block, including calls made from another unsafe function:

```ck
unsafe {
    saxpy(x, y, n);
}
```

An unsafe function must have at least one `requires` clause. A contract and an
`effects` clause are invalid on a safe function. Unsafe blocks do not suppress
unrelated type, control-flow, bounds-mode, or ABI diagnostics.

The contract is required to hold when control enters the function. A false
`requires` clause makes that execution immediately undefined, independently of
optimization level, backend, or whether a pass happens to exploit the fact.
Normal O0 through O3 compilation does not insert contract checks.

This boundary governs new trusted optimization facts. It does not retroactively
make CK a memory-safe language: the existing caller responsibility for raw
pointers and `slice(data, len)` memory validity remains in force. Conversely,
ordinary CK execution does not acquire new undefined behavior merely because an
optimizer guesses a fact; every non-contract fact used by a transformation must
be proven.

`unsafe` does not change a function's C ABI. A generated header for an exported
unsafe function includes normalized contract comments. Foreign callers carry
the same entry obligation. Strengthening an exported precondition is a breaking
source-contract change; weakening one is compatible.

### Closed contract language

Contract expressions are side-effect-free compiler facts, not executable CK
code. They may contain:

- integer parameters, integer constants, `slice.len`, and `slice.data` where a
  pointer predicate requires it;
- affine integer expressions using addition, subtraction, and multiplication
  by an integer constant;
- `==`, `!=`, `<`, `<=`, `>`, `>=`, and conjunction;
- `multiple_of(value, positive_constant)`;
- `noalias(slice_a, slice_b)`;
- `aligned(pointer, power_of_two)`; and
- one optional effect ceiling: `effects none` or a comma-separated set of
  `read(slice)`, `write(slice)`, and `readwrite(slice)`.

Function calls, disjunction, negation, memory loads, stores, mutable state, and
target-specific cache, vector-width, or prefetch hints are not contract syntax
in 0.11. Contract integer expressions are interpreted over unbounded
mathematical integers, so their evaluation cannot itself overflow. Ordinary CK
type rules are checked before that mathematical lift; the contract language does
not create implicit signed/unsigned conversions.

`noalias(a, b)` means that the complete valid byte ranges denoted by the two
slice descriptors do not overlap for the dynamic extent of the call. The ranges
are mathematical allocation ranges and do not wrap at the target address width.
A zero-length slice denotes an empty range. `aligned(p, n)` requires `n` to be a
positive `u32` power of two no greater than `2^31` and the pointer address to be
divisible by `n`; a null pointer is aligned by this predicate, while its
dereferenceability continues to follow the existing slice-validity rules.
Effect targets in 0.11 are named slice parameters.

An effect clause is an upper bound, not an unchecked promise. The compiler
checks it against the function body and transitive callee summaries. Omitting
the clause requests inferred effects. An incomplete clause is diagnostic
`CK2016`, not undefined behavior.

Local `assume`, loop contracts, and arbitrary contract expressions are not part
of 0.11. Loop facts must be derived from entry contracts, SSA values, branch
conditions, and induction analysis.

## KIR model

KIR contains typed functions, basic blocks, scalar SSA definitions, phi nodes,
explicit control-flow, region Memory SSA, and explicit operations that may fail
or produce runtime effects. Bounds and overflow guards remain explicit until a
proof-carrying transformation removes them. Possible checked failure and
runtime print are ordered effects; a transformation cannot move another
possibly failing or observable operation across them without proving the move
unobservable.

Each pointer or slice origin has a stable `MemoryRegion`. A sub-slice retains
its parent region plus a symbolic byte interval. Proven `noalias` relations
partition regions. Loads consume a partition version, stores and effectful calls
produce new versions, control-flow joins produce memory phi nodes, and unknown
aliasing merges affected regions into a conservative partition. Failure to
prove separation loses an optimization opportunity but never correctness.

Facts have stable in-compilation identifiers and one of two origins:

- `Proven`: derived by validated compiler analysis; or
- `TrustedContract`: imported from a dominating unsafe function entry.

Proofs form a dependency DAG over facts, instructions, blocks, and effect
summaries. Transformations and diagnostic output retain the origin distinction.

## Analyses

### Scalar and path facts

The scalar domain combines signed and unsigned intervals, affine relations,
congruence, and internal known bits. Branch edges refine path-sensitive facts.
Natural-loop phi nodes use deterministic widening to guarantee termination and
a fixed number of narrowing iterations to regain precision.

Checked arithmetic carries a possible-failure effect until analysis proves the
operation cannot fail. Unchecked integer arithmetic uses the specified modular
semantics and cannot inherit mathematical-integer conclusions that wrapping
invalidates.

Analysis limits are functions of KIR size and fixed configuration, never wall
clock time. Exceeding a limit yields `unknown` or a conservative summary.

### Alias and memory facts

Region identity, symbolic sub-slice intervals, `noalias`, access width, and
alignment feed one shared alias query service. Memory SSA, load forwarding,
dead-store elimination, LICM, and backend metadata all use this service rather
than implementing pass-local alias rules.

### Interprocedural effects

The compiler solves effect summaries bottom-up over call-graph strongly
connected components. A summary records parameter-mapped reads and writes,
runtime print, possible checked failure, and unsafe calls. Recursive components
use monotone iteration to a fixed point. An unknown or over-budget function
becomes `readwrite all + may_fail + runtime_effect`.

## Proof-carrying transformation

A pass cannot silently erase or weaken a bounds or overflow guard. It submits a
transformation with a `ProofId` identifying the dominating range, control,
slice-length, alias, alignment, effect, and contract facts used. The independent
KIR verifier checks the proof against the current CFG, scalar SSA, Memory SSA,
and effect order after every pass.

CFG edits, inlining, and loop edits invalidate facts and proofs through explicit
analysis-preservation declarations. A stale or invalid proof is a compiler
internal error: compilation stops and no artifact is committed. The compiler
does not recover by emitting unverified machine code.

## Optimization levels and 0.11 scope

O0 constructs and verifies KIR but performs no optional optimization. O1 adds
CFG canonicalization, sparse conditional constant and range propagation,
proof-carrying redundant-check elimination, dead-code elimination, and cleanup.
O2 adds effect-aware inlining, global value numbering, Memory-SSA load
forwarding, dead-store elimination, another propagation/check-elimination
cycle, and cleanup. O3 adds natural-loop and induction analysis,
effect/alias-aware loop-invariant code motion, induction simplification, a
final propagation/check-elimination cycle, and cleanup.

Every named pass is deterministic and followed by KIR verification. The one
selected level controls the shared KIR pipeline for all backends and the
subsequent Native LLVM optimization level.

CK 0.11 expressly excludes automatic SIMD, loop unrolling, function
specialization, PGO, Auto-Tuning, fast-math, local assumptions, and permanent
dual pipelines. Existing strict floating-point, checked-error, runtime-effect,
and ABI behavior remains unchanged.

## Backend fact mapping

Only verified KIR facts may strengthen backend IR.

The Native LLVM lowering uses a reviewed whitelist that includes `noalias`,
`readonly`, `writeonly`, applicable `memory(...)` effects, alignment,
`alias.scope`/`noalias` metadata, integer ranges, and proven `nuw`/`nsw` flags.
It emits `llvm.assume` only when a verified fact has no more direct
representation and the assumption is useful. Every strengthening retains a
`FactId` or `ProofId` in the compiler's audit map. A post-lowering audit rejects
an LLVM attribute or flag without an admissible KIR source.

The C backend consumes the same optimized KIR and may express equivalent facts
through standard `restrict` and conditionally defined compiler alignment hints;
the portable fallback remains valid C. The WebAssembly backend consumes KIR
check elimination and proven access alignment but does not invent a checked ABI
or unsupported alias metadata. No backend may infer a stronger source contract
than KIR provides.

LLVM remains responsible for machine-level canonicalization, instruction
selection, register allocation, scheduling, target legalization, and its own
legal downstream optimizations. It is no longer CK's sole fact-discovery layer.

## Inspection, diagnostics, and sanitizer

The 0.11 CLI adds deterministic inspection surfaces:

- `ckc emit-kir`;
- `--print-facts`;
- `--print-effect-summaries`;
- `--explain-optimization`; and
- `--sanitize-contracts`.

Optimization explanations identify each removed or retained check, its facts
and proof, whether a trusted contract participated, and the conservative reason
when no transformation was legal. Output contains no absolute paths, addresses,
timestamps, or unordered-map iteration.

Contract sanitization is accepted only by `run` and executable builds. It
instruments dynamically checkable `requires` clauses at unsafe call and exported
entry boundaries. Effect ceilings remain compile-time checks. A violation emits
exactly `CKR0007: unsafe contract violation` plus LF on stderr and exits with
status 246. Sanitized behavior is a debugging facility, not normal language
semantics, a production-library ABI, or benchmark evidence. Status 246 is
reserved for this sanitizer failure in 0.11; production runtime statuses remain
those selected by the normal overflow, bounds, output, and child-process
contracts.

The source diagnostics added by this specification are:

| Code | Meaning |
| --- | --- |
| `CK2014` | Invalid unsafe-function, contract-placement, unsafe-block, or unsafe-call boundary. |
| `CK2015` | Invalid, ill-typed, unsupported, or non-decidable contract expression or predicate. |
| `CK2016` | A declared effect ceiling does not cover the statically inferred function effects. |

KIR verifier and backend fact-audit failures are compiler errors, not CK source
diagnostics.

## Acceptance contract

CK 0.11 is not complete until all of the following hold:

1. Every existing 0.10 semantic, ABI, checked first-error, runtime-print, CLI,
   artifact, and distribution test remains green without ignored tests or
   lowered thresholds.
2. All official and compatibility fixtures pass differential C, WebAssembly,
   and Native testing at O0 through O3 in every supported checked/unchecked
   combination.
3. Contract tests cover valid syntax, every invalid boundary and predicate,
   foreign-export comments, unsafe calls, immediate-UB model, and positive and
   negative sanitizer executions.
4. KIR mutation tests prove that the verifier rejects missing dominance,
   malformed scalar or memory phi nodes, incorrect alias partitions, stale
   facts, invalid ProofIds, and reordered possible failures or runtime effects.
5. Fixed-seed generated kernels compare unoptimized and optimized observable
   behavior; contract-generated cases are run only with inputs satisfying the
   declared domain.
6. LLVM fact audits pass for all six release targets and reject intentionally
   injected attributes without a KIR source.
7. Canonical provable loops contain no redundant hot-loop bounds guard at O2 or
   O3, as verified structurally in KIR and backend IR.
8. The existing Native-versus-pinned-Clang O3 gate remains at a minimum 95%
   geometric-mean throughput with no individual kernel more than 10% slower.
9. Against the pinned accepted 0.10 compiler baseline on controlled workers,
   0.11 loses no more than 3% geometric-mean runtime throughput and no more than
   8% on an individual kernel. The benchmark manifest records the exact 0.10
   source digest and compiler identity rather than relying on a moving branch.
10. For the accepted suite of loops whose checks are fully proven redundant,
    checked geometric-mean throughput is at least 97% of the corresponding
    unchecked execution.
11. The median KIR analysis-and-optimization time is at most 2 times the pinned
    0.10 MIR optimization time, and no accepted case exceeds 3 times. A budget
    fallback must preserve semantics and report the conservative reason.

Runtime comparisons pin source, compiler identity, target, CPU policy, safety
modes, strict floating-point behavior, harness, warm-up, repetition count, and
statistical rule. Threshold or corpus changes are reviewed contract changes.

## Performance program after 0.11

The following work requires separate versioned specifications and cannot expand
the 0.11 implementation plan:

- 0.12: loop canonicalization and dependence legality, profitability models,
  loop and SLP SIMD, controlled unrolling/versioning, and fact-driven function
  specialization;
- 0.13: baseline-plus-feature CPU multiversioning, a stable profile schema,
  profile instrumentation/use, and PGO; and
- 0.14: bounded, reproducible, cached offline Auto-Tuning whose cache key covers
  compiler and ABI identity, kernel and contract digests, target CPU, profile,
  candidate space, and measurement policy.

For eligible vector kernels, the eventual gate fixes audited hand-written
C/Rust plus SIMD sources and requires CK geometric-mean throughput of at least
95% of that oracle, with every individual kernel at least 90%. A separate suite
whose CK contracts expose domain constraints unavailable to the fixed generic
source must beat the pinned generic Clang/Rust O3 geometric mean by at least 5%
under the same observable behavior, valid input domain, and safety semantics.
The hand-written SIMD oracle receives every equivalent precondition its source
language can express. The generic-default oracle is independent audited source,
not C emitted from optimized KIR; it contains no hidden undefined behavior and
encodes only the facts present in that source. Auto-Tuning must select a legal
candidate within its fixed budget, preserve a baseline fallback, and never make
runtime exploration an implicit production dependency.
