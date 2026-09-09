# CK 0.13 Design Blocker Rediagnosis 01

## Decision

All three reported blockers reproduce against the current repository and are
accepted. The review did not mistake an implementation preference for a
blocker:

- `ckc pgo merge` had no information capable of detecting overlapping nested
  aggregates.
- `run_build` and the Native artifact layer deliver exactly one object for
  `--kind object`, with no bundle or partial-link abstraction.
- `ckc_llvm_module_optimize` runs the complete default LLVM pipeline in one
  call, so the old O2 wording did not impose an enforceable metadata boundary.

## Closed contract

### R1: one-layer merge

Schema 1 merge accepts completed `.ckprof-part` shards only. A `.ckprof` is a
terminal aggregate and is rejected as merge input. This preserves deterministic
output without retaining workload/run provenance in the final artifact and
makes duplicate-run rejection implementable.

### R2: explicit artifact legality matrix

`--cpu multiversion --kind object` is rejected in 0.13. Multiversion final
artifacts may be executable, dynamic, or static; baseline/native profile-use
objects remain supported. Profile generation also rejects object output because
an unlinked object has no defined flush owner. No implicit bundle or partial
link is introduced.

### R3: enforceable O2 split

The complete default LLVM O2 IR pipeline runs without any profile-derived LLVM
metadata. A checked post-O2 survivor map is then used to attach weights; no
further CFG-changing or code-copying LLVM IR pass runs. Only artifact ordering
and machine-code layout/scheduling may consume the weights at O2. O3 retains the
broader profile-aware optimization permission.

## Additional clarification retained from the review

The library generation workflow now exposes an instrumentation-only,
identity-namespaced, idempotent flush entry. The host quiesces calls and receives
the write result synchronously; unload hooks perform no profile I/O. The design
also freezes generation-directory resolution, saturation fallback, exact
integer weighted-cost comparison, and tests for these contracts. These changes
close planning ambiguity without weakening any acceptance gate.

## Gate for review 02

The next adversarial review must verify all three closures against both language
mirrors and current source, and must look for new blocking contradictions caused
by the revised CLI/lifecycle/LLVM boundaries. Planning cannot begin until that
review returns no blocker.

