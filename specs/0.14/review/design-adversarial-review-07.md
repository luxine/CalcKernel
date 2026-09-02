# CK 0.14 Design Adversarial Review 07

Review target: commit 4a9f132d8485805a42fad9d69acd6ef2d81695cd

Reviewer: fresh read-only gpt-5.6-sol, ultra reasoning

Verdict: BLOCKED

Blockers: 3

## Blocking findings

### B1. Relative runner-path base is undefined

The manifest declares an explicit runner path, but unlike `input_root` it does not
say whether a relative value resolves from the manifest, source, or caller working
directory. The frozen performance path is therefore not unique.

Minimum correction: resolve relative runner paths from the canonical manifest
parent and explicitly decide whether absolute paths are accepted.

### B2. Expansion ordinal has no base or continuity rule

The algorithm increments its counter before recording, while the wire schema only
sorts by ordinal. The same attempts can be encoded as 0,1,2 or 1,2,3, changing
frontier/session/rotation identity without violating current prose.

Minimum correction: freeze zero-based contiguous ordinals and align pseudocode.

### B3. Trials and finalists are not bound to deterministic search output

The wire schema does not require `Candidates.trials` to be exactly the computed
compile selection, nor derive size rejection and measured finalists from the full
postcompile ranking. An implementation can omit the best plan, measure a weaker
one, and still build internally consistent streams and selection records.

Minimum correction: replay Frontier/Contract to derive the complete compile set,
require one trial per selected plan, and derive every size/finalist outcome and
required stream set from actual bytes and the closed postcompile rank.

## Confirmed closures

The input-map ABI, sole `--config` CLI, and common Linux `wait4` high-water protocol
are now closed. The reviewer also reconfirmed Windows argv, secret-free environment
identity, timeout states, validation derivation, schema-8 split, external work and
correctness, build/output/cache/archive provenance, publication recovery, and
general English/Chinese alignment.

Final verification found the requested HEAD and a clean index/worktree. The
reviewer made no edits.
