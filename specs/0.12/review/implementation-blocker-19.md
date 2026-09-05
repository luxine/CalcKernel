# Implementation blocker 19: x86 UF chunks hide independent memory parallelism

Date: 2026-09-05

## Finding

Exact V0.12 run `33966418774`, job `101307347417` (`performance
(x86-64)`), failed the unchanged domain-fact gate:

`domainFactSuites does not exceed generic oracles by 5%`.

The retained unchecked medians were:

- `contract_noalias`: CK 4,833,050 ns, generic C 4,395,200 ns, generic
  Rust 4,833,330 ns;
- `contract_fixed_length`: CK 3,953,465 ns, generic C 4,396,953 ns,
  generic Rust 4,834,713 ns.

The geometric mean of the two faster-oracle/CK ratios was approximately
1.0057, below the unchanged 1.05 requirement. The checked-domain geometric
mean was approximately 1.37 and passed, so this is isolated to the unchecked
x86 code shape rather than a report, corpus, or checker defect.

## Rediagnosis

The exact artifact selected `VF4/UF2` for the 16-element noalias kernel. Its
machine loop preserved materializer order separately for each UF chunk:

`load[0] -> add[0] -> store[0] -> load[1] -> add[1] -> store[1]`.

The generic C and Rust loops expose two independent vector loads before their
arithmetic and stores. CK therefore serializes a read/compute/write chain that
the noalias contract has already proved independent across the two chunks.
The prior UF-scaled admission repair correctly admits the vector candidate;
this failure is a distinct scheduling defect after admission, not evidence for
restoring the old scalar fallback.

## Approved repair

For x86-64 candidates with `UF > 1`, apply a deterministic local list schedule
to the already materialized vector body:

1. an instruction is eligible only after every locally defined SSA operand and
   MemorySSA input has been scheduled;
2. among eligible instructions, prefer vector loads, then scalar address/setup
   work, then vector computation, then vector stores;
3. preserve original instruction order within each priority class;
4. preserve same-partition memory ordering through MemorySSA dependencies and
   renumber ordered effects monotonically after scheduling.

This exposes independent UF loads before stores for the contract-proved
noalias kernel while retaining dependencies for same-partition memory,
reductions, and scalar recurrences. Non-x86 targets and `UF == 1` retain their
existing materialization order. The independent transaction checker and the
structural KIR verifier remain mandatory after materialization.

## Acceptance and propagation

The repair is test-first. A structural regression must fail on the old
`VF4/UF2` order and require both independent vector loads to precede either
store. Focused Loop SIMD and Native LLVM tests must pass before full local
gates. The authoritative x86 performance job must then show the generated
schedule through the unchanged domain-fact gate.

After V0.12 is pushed, V0.13 must import the exact implementation and repin its
V0.12 replay identity. V0.14 must import the resulting V0.13 state and repin
its V0.13 replay identity. Replacement CI runs must be bound to those exact
remote SHAs.

No language or ABI rule changes. No performance threshold, timed work, batch
size, sample count, statistic, corpus, CPU policy, compiler identity, platform
matrix, or required CI job may be reduced or skipped.
