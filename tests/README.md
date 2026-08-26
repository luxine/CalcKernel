# Integration Test Layout

Cargo compiles seven responsibility-based integration drivers: `frontend`,
`ir`, `optimizer`, `backend`, `cli`, `contracts`, and `performance`. Shared test
infrastructure lives under `support/`; responsibility files are not standalone
Cargo test crates.

The V0.9 reorganization preserved all 222 test functions. Every former test
name has the exact replacement `<module>::<former-name>` according to this
mapping; no leaf test function was renamed.

| Former driver | Current driver and module |
| --- | --- |
| `backend_surface_test.rs` | `backend.rs` → `surface` |
| `c_backend_test.rs` | `backend.rs` → `c` |
| `control_void_slice_e2e_test.rs` | `backend.rs` → `control_void_slice` |
| `llvm_backend_test.rs` | `backend.rs` → `llvm` |
| `wasm_backend_test.rs` | `backend.rs` → `wasm` |
| `cli_test.rs` | `cli.rs` → `commands` |
| `typescript_oracle_portability_test.rs` | `cli.rs` → `oracle_portability` |
| `typescript_oracle_readiness_test.rs` | `cli.rs` → `oracle_readiness` |
| `ci_surface_test.rs` | `contracts.rs` → `ci` |
| `docs_surface_test.rs` | `contracts.rs` → `docs` |
| `git_repository_test.rs` | `contracts.rs` → `git` |
| `release_surface_test.rs` | `contracts.rs` → `release` |
| `repository_contract_test.rs` | `contracts.rs` → `repository` |
| `checker_test.rs` | `frontend.rs` → `checker` |
| `lexer_test.rs` | `frontend.rs` → `lexer` |
| `parser_test.rs` | `frontend.rs` → `parser` |
| `frontend_surface_test.rs` | `frontend.rs` → `surface` |
| `mir_test.rs` | `ir.rs` → `mir` |
| `optimizer_test.rs` | `optimizer.rs` → `passes` |
| `bench_surface_test.rs` | `performance.rs` → `bench` |
| `typescript_oracle_fixture_coverage_test.rs` | `performance.rs` → `oracle_fixtures` |
