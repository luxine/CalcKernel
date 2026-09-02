# CK 0.14 Design Adversarial Review 08

Review target: commit a62465bac3940ba6a8024d57f914c9f76eb78e6d

Reviewer: fresh read-only gpt-5.6-sol, ultra reasoning

Verdict: PASS

Blockers: 0

## Confirmed closures

- Relative `runner.path` values resolve from the canonical manifest parent while
  absolute paths are accepted unchanged. Every path component rejects
  symlink/reparse traversal, the final file is opened without following links,
  and only the captured runner snapshot is executed. The spelling is excluded
  from identity while the snapshot length and digest are included.
- Expansion ordinals are zero-based and contiguous. The source-aware checker
  replays every attempt through the normative loop, so missing, inserted,
  reordered, or misclassified attempts are rejected.
- Frontier, compile selection, trials, size filtering, finalists, outcomes, and
  required streams form a complete recomputable chain. Independent rebuilds,
  exact size decisions, the candidate-state matrix, validation entrants, raw
  streams, and derived summaries are all checker-bound.
- The wall budget covers baseline, compilation, runner setup, search, both
  validation rounds, final replay, and staging. Work cannot start unless its
  complete timeout plus fixed margin fits, and partial expansion, compilation,
  or validation cannot produce a successful decision.
- The English and Chinese designs, all four normative attachments, and prior
  review corrections remain aligned. No earlier blocker regressed.
- The v0.13 CLI, cache, checker, artifact ABI, and KIR pipeline provide adequate
  implementation seams. The absence of an accepted v0.13 tag remains an explicit
  release gate, not a design blocker.

Final verification found the requested HEAD and a clean index/worktree. The
reviewer made no edits.
