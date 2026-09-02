# CalcKernel

[简体中文](README.zh-CN.md)

CalcKernel 0.14.0 ships `native ckc`, a self-contained Rust command-line
compiler for the CK computation-kernel language. Release binaries compile, link,
and run native CK without an external compiler toolchain. The repository also
retains inspectable C and WebAssembly source/binary emitters.

## What ships

- A Rust lexer, parser, type checker, deterministic semantic MIR, and one
  verified fact-driven KIR optimizer shared by every backend.
- Explicit `unsafe fn` entry contracts for affine ranges, alignment, no-alias,
  and memory-effect ceilings, with opt-in contract sanitization.
- `break` / `continue`, return-only `void`, caller-owned `slice<T>`, optional
  checked overflow and slice bounds, and a parameterless `main` entry.
- `ckc run` with an isolated child, deterministic numeric/boolean printing, and
  a secure persistent object cache.
- `ckc build --kind executable|dynamic|static|object` using embedded LLVM
  22.1.8 code generation and in-process LLD.
- One generated-header Native C ABI for object, static, and dynamic libraries.
- A target-profiled, independently checked O3 optimizer with transactional
  specialization, controlled unrolling, SLP, Loop SIMD, runtime alias
  versioning, strict-f64 vectors, and exact modular integer reductions.
- CK-owned `CKPART01`/`CKPROF01` profile generation, merge, inspection, and
  non-proof PGO application through explicit `ckc pgo` / `--pgo-*` workflows.
- Explicit Native `--cpu multiversion` builds with a portable baseline,
  verified bounded variants, baseline-safe one-time dispatch, and stable ABI
  thunks in executable, dynamic, and static artifacts.
- Explicit offline Auto-Tuning through `ckc tune build|inspect` and verified
  `ckc build --tune-use`, with immutable workload snapshots, deterministic
  search, correctness-first measurement, private caches, and journaled output.
- Source-only C output and portable WAT/WASM output.
- Six zero-toolchain release archives for macOS, Linux, and Windows on AArch64
  and x86-64.

Native checked modes support all four overflow/bounds combinations. C emission
supports the same checked status semantics. WebAssembly is unchecked-only.
Native runtime printing is accepted for `run` and executables; reachable print
effects are rejected from library, C, and WebAssembly roots.

## Pipeline

```text
.ck -> frontend -> semantic MIR -> mode/consumer-specific verified KIR v3
                                 -> optional CK workload profile (non-proof)
                                 -> target-profiled transactional optimizer
                                 -> optional verified CPU variants + dispatcher
                                      +-> C source/header
                                      +-> WAT/WASM
                                      +-> structural LLVM -> object
                                                               +-> ORC run
                                                               +-> in-process LLD -> executable/library
```

The product path does not invoke Clang, a system linker, or an archiver. A
pinned Clang 22.1.8 build exists only as the repository's differential and ABI
test oracle.

## Use a release binary

```sh
ckc --version --verbose
ckc check examples/core/scalar.ck
ckc emit-kir examples/core/scalar.ck --print-facts
ckc emit-kir examples/core/scalar.ck --consumer native-library \
  --cpu baseline --explain-optimization
ckc run examples/native/hello.ck
ckc build examples/native/hello.ck --kind executable --out /tmp/hello
ckc build examples/core/scalar.ck --kind dynamic --out /tmp/scalar
ckc pgo build examples/native/hello.ck --out /tmp/hello-pgo \
  --profile-out /tmp/hello.ckprof
ckc build examples/core/scalar.ck --kind static --cpu multiversion \
  --pgo-use /tmp/scalar.ckprof --out /tmp/libscalar.a
ckc tune build examples/native/hello.ck --config workload.cktune.toml \
  --kind executable --cpu native -O3 --out /tmp/hello-tuned
ckc tune inspect /tmp/hello-tuned.cktune
ckc build examples/native/hello.ck --kind executable --cpu native -O3 \
  --tune-use /tmp/hello-tuned.cktune --out /tmp/hello-replayed
ckc emit-c examples/applications/pricing.ck --out /tmp/pricing.c
ckc emit-wasm examples/wasm/scalar.ck --out /tmp/scalar.wasm
ckc licenses
```

`run` and `build` default to O3. PGO and offline tuning are off unless their
explicit commands or flags are present; ordinary development never trains,
measures a runner, or reads a profile/tuning decision. Native
build defaults to the release target's portable CPU baseline; `--cpu native` and
`--cpu multiversion` are explicit. `build-llvm` remains only as a deprecated
alias and has no PGO/multiversion behavior.

## Build from source

The native feature requires the exact LLVM prefix described by
`native/llvm/manifest.toml`; repository scripts bootstrap it into `build/llvm`.
The prefix is a build input, not an end-user runtime dependency.

```sh
rustc_host="$(rustc -vV | sed -n 's/^host: //p')"
llvm_archive=/path/to/llvm-project-22.1.8.src.tar.xz
./scripts/bootstrap-llvm.sh --archive "$llvm_archive" \
  --prefix "$PWD/build/llvm/prefix-$rustc_host-release" \
  --target "$rustc_host" --profile release
export CKC_LLVM_PREFIX="$PWD/build/llvm/prefix-$rustc_host-release"
cargo build --release --features native-toolchain --locked
cargo test --all-features --locked
```

A frontend/C/WASM-only developer build is also available with default features:

```sh
cargo test --locked
cargo build --release --locked
```

## Documentation and verification

Start with the [documentation index](docs/index.md), the
[language reference](docs/reference/language.md), the
[CLI reference](docs/reference/cli.md), and the [Native ABI](docs/abi/llvm.md).
Runnable language examples cover [control flow](examples/core/control_flow.ck),
[void procedures](examples/core/void.ck), and [slices](examples/core/slices.ck).
Every durable user-facing document has a Simplified Chinese mirror under
`docs/zh-CN/`.

The strict native local gate is:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-features --locked
cargo build --release --features native-toolchain --locked
./target/release/ckc --version --verbose
./target/release/ckc licenses
```

Release policy, platform audits, performance gates, archive names, and immutable
GitHub Release publication are defined in [docs/project/release.md](docs/project/release.md).

CalcKernel 0.14.0 keeps the public Native C ABI at version 1 and Runtime ABI at
version 2. The private LLVM bridge is ABI 4, KIR uses the `kir-v3` identity, and
the Native object cache uses `CKCOBJ04` with key/manifest schema 5. Old 0.13
and earlier private cache entries fail closed instead of aliasing a 0.14 artifact.
The accepted 0.13.0, 0.12.0, 0.11.0, and 0.10.0 source boundaries are retained in the
[compatibility policy](docs/project/compatibility.md).

PGO, bounded runtime multiversioning, and explicit offline Auto-Tuning are
implemented. Indirect-call promotion, scalable KIR vectors, and adaptive JIT
PGO remain future work.

## Memory boundary

`slice(data, len)` and `items[start..end]` create non-owning `slice<T>` descriptors. The
caller remains responsible for raw-pointer validity, allocation extent,
alignment, lifetime, and declared length. `--bounds checked` validates only slice
index/range relations; they do not make arbitrary pointer use memory-safe.
