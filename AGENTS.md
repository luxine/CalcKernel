# Repository Conventions

## Canonical naming

- Language: CK / CalcKernel
- Source extension: `.ck`
- Compiler command: `ckc`
- Do not introduce TK, `tkc`, `.tk`, or other compatibility aliases unless the
  user explicitly changes the language contract.
- Examples, tests, snapshots, diagnostics, and CLI documentation must use the
  canonical names.

## Source and test placement

- Keep compiler responsibilities under `src/frontend/`, `src/ir/`,
  `src/optimizer/`, `src/backend/`, and `src/cli/`.
- Keep the binary entry point thin at `src/bin/ckc.rs`.
- Add runnable examples under the matching
  `examples/{core,applications,checked,wasm,llvm}/` directory.
- Put integration tests under the responsibility layout documented in
  `tests/README.md`; shared helpers belong under `tests/support/`.

## Durable documentation

- `docs/` contains only current, user-facing or maintainer-facing project
  documentation. Start at `docs/index.md` or `docs/zh-CN/index.md`.
- New or materially changed formal documentation must have matching English and
  Simplified Chinese files at identical relative paths under `docs/` and
  `docs/zh-CN/`.
- Keep the V0.9 language, CLI, MIR, ABI, compatibility, and release contracts
  synchronized with implementation and tests.
- Do not commit phase plans, dated reviews, readiness reports, migration
  narratives, AI working notes, or release-history snapshots as formal docs.
- Local agent planning may use the ignored `Ai_repository/` directory, but its
  contents must never be committed or shipped. Rewrite any durable conclusion
  into the current bilingual documentation tree instead of preserving history.
