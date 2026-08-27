# Integration Test Layout

Cargo compiles eight responsibility-based integration drivers. Files below each
driver directory are modules, not standalone Cargo test crates.

| Driver | Responsibility |
| --- | --- |
| `frontend.rs` | Lexer, parser, type checker, source model, entry and builtin rules. |
| `ir.rs` | MIR lowering, validation, effects, and deterministic printing. |
| `optimizer.rs` | O0–O3 transformations and semantic preservation. |
| `backend.rs` | C, WAT/WASM, portable output, and non-Native backend contracts. |
| `cli.rs` | Argument precedence, command behavior, output transactions, and optional oracle parity. |
| `native.rs` | LLVM bridge/IR, ABI, object/link artifacts, runtime, ORC, run, and cache. |
| `contracts.rs` | Repository, documentation, CI, provenance, release, and version invariants. |
| `performance.rs` | Benchmark/report contracts and oracle fixture coverage. |

Shared filesystem, process, temporary-path, source, and oracle helpers live under
`support/`. Stable test inputs live under `fixtures/`; a test should prefer a
fixture when bytes, paths, compatibility, ABI, or multi-stage reuse matter.
Compatibility changes released in 0.10 are indexed by
`fixtures/compatibility/v0_10/manifest.toml` and must resolve to executable test
evidence.

Run the feature-disabled suite with:

```sh
cargo test --locked
```

Native tests require the pinned release LLVM prefix and, for differential tests,
the separate pinned Clang oracle:

```sh
export CKC_LLVM_PREFIX=/absolute/path/to/release-prefix
export CKC_CLANG_ORACLE=/absolute/path/to/oracle-prefix/bin/clang
cargo test --all-features --locked
```

Generated objects, libraries, executables, caches, and benchmark reports belong
under ignored `target/` or `build/` paths and must not be committed.
