# Cross-language numerical benchmark

This directory contains a small, reproducible comparison for two CPU-bound
numerical kernels. It compares CalcKernel (CK), C++, Rust, JavaScript on Node.js,
Java on OpenJDK 21, and Python using NumPy. It compares these checked-in
implementations on one host, not the languages in general.

## Run it

Use the release `ckc` executable built with the Native toolchain, a C++ compiler,
`rustc`, Node.js, matching OpenJDK 21 `java` and `javac` commands, and a Python
interpreter with NumPy installed. Pass `--python` to select a specific Python
interpreter when the default does not have NumPy.

```sh
python3 benches/cross-language/run.py \
  --ckc /path/to/release/ckc \
  --python /path/to/python-with-numpy \
  --output target/cross-language/benchmark.json
```

`CKC` may be set instead of `--ckc`. Compiler executables default to `clang++`
(or `g++`/`c++`), `rustc`, `node`, and `javac` from `PATH`; the Java runtime
defaults to `java`. Use `--cxx`, `--rustc`, `--node`, `--java`, or `--javac` to
select installed toolchains. Java and Javac must come from the same OpenJDK 21
release. Run from the repository root. The runner supports macOS and Linux
shared libraries.

The default is seven rounds with roughly 75 ms per timed batch. `--rounds` may
increase the number of measured samples, but cannot be less than seven.
`--target-ms` changes the calibration target. Each round shuffles all twelve
language/mode variants for each kernel with a fixed seed, so the JSON records
both the exact sample order and every measured nanoseconds-per-call sample.
The default output name includes the UTC date and host name. `--output` chooses
a stable path when results need to feed a published chart.

## Workloads and inputs

Both kernels use contiguous, row-major IEEE 754 `f64` values. The runner,
JavaScript, Java, and NumPy workers use the same deterministic inputs:

- Matrix multiplication: two 256 × 256 matrices, with
  `A[row,col] = (((row*17 + col*13) % 31) - 15) / 32` and
  `B[row,col] = (((row*11 + col*7) % 29) - 14) / 32`.
- Gaussian convolution: a 1024 × 1024 input, with
  `input[index] = (((index*17) % 37) - 18) / 32`. The 3 × 3 stencil weights
  are `1,2,1 / 2,4,2 / 1,2,1`, divided by 16. The one-cell output border is
  zero.

Before timing, the runner computes an independent NumPy reference and compares
the SHA-256 hash of the full output buffer from every implementation. These
inputs and weights are dyadic fractions, so their sums are exactly representable
as `f64`; all implementations must produce the same output bits. A mismatch
stops the run before a result file is written.

## What “ordinary” and “tuned” mean here

These labels describe only the paired implementations in this benchmark; they
are not a universal optimization scale:

- **Ordinary** uses the direct `row, column, inner` matrix loop. Native CK,
  C++, and Rust still compile with O3, but target a portable baseline. NumPy's
  ordinary matrix path uses `@` and copies its allocated result to the supplied
  output; its ordinary stencil uses vectorized expressions with intermediate
  arrays.
- **Tuned** uses a row-contiguous `row, inner, column` matrix loop, which reuses
  each input value across adjacent output columns. NumPy's tuned matrix path
  writes to a preallocated output through `out=`; its stencil reuses scratch
  arrays. Native CK, C++, and Rust compile with O3 and the host's native CPU
  features.
- **JavaScript** runs both checked-in functions in one long-lived Node.js
  process. Each function is run before calibration and again while V8 warms up.
  Node/V8 version is recorded; ordinary and tuned refer to the two source
  variants, not separate compiler flags.
- **Java** compiles the checked-in worker with OpenJDK 21 `javac` and runs both
  functions in one long-lived JVM process. Initialization warms both variants
  for each workload before calibration; the JVM and compiler versions, compile
  command, and worker-reported warm-up duration are recorded. Ordinary and
  tuned select the two source variants under the same JDK and JVM settings.

The convolution variants apply the same stencil and zero border, but their
source-level changes differ. CK and JavaScript ordinary variants use a general
stencil path, while tuned variants write the interior directly. C++ and Rust
already traverse only interior cells in both variants; tuned caches row bases.
This is why the chart should label the bars by workload and mode, and should
not imply that every language receives identical source-level tuning.

C++ builds use O3, strict floating-point flags (`-fno-fast-math` and
`-ffp-contract=off`), and the portable host architecture for ordinary or native
host tuning for tuned. Rust uses `opt-level=3`, `target-cpu=x86-64` or `generic`
for ordinary depending on the host, and `target-cpu=native` for tuned. CK uses
`ckc build --kind object -O3 --cpu baseline|native`; the resulting object is
linked as a dynamic library with Clang. This two-step CK link also works around
the current macOS LLVM linker's handling of an O3 zero-fill loop without
changing the kernel or lowering optimization. The result JSON stores each
compiler command with its flags, repository-relative source path, temporary
output placeholder, and exit status. Compiler versions are stored separately;
build stdout and stderr are not retained.

Java and NumPy run in persistent workers. Before importing NumPy, the runner sets
`VECLIB_MAXIMUM_THREADS`, `OMP_NUM_THREADS`, `OPENBLAS_NUM_THREADS`,
`MKL_NUM_THREADS`, `BLIS_NUM_THREADS`, and `NUMEXPR_NUM_THREADS` to `1`. The
NumPy version and its `numpy.__config__.show()` report are recorded, with
absolute paths redacted from both JSON and text formats; on Apple Silicon, the
current bundled NumPy reports Apple's Accelerate backend.

## Timing and reading results

Inputs, output buffers, compilation, process startup, initialization, and SHA-256
calculation are outside the measured interval. Native CK/C++/Rust functions are
called through a small C batch shim, so Python does not cross the FFI boundary
for each operation. JavaScript, Java, and NumPy each use a persistent worker
process; their timed batches include the worker-language repeat loop and
per-call function dispatch (JavaScript in Node/V8, Java in the JVM, Python in
the NumPy worker). The checksum is calculated after timing. Calibration chooses
a repeat count that targets the requested batch length, then samples each
implementation in shuffled, interleaved rounds. These loop and dispatch costs are part of the
measurements and should be considered when interpreting cross-language
differences, especially for smaller workloads.

The JSON includes compiler/runtime versions (including Java and Javac), host
and hardware details, build commands with exact flags and repo-relative source
names (temporary build paths use a `<build-dir>` placeholder), source SHA-256
hashes, fixed input formulas, reference/output hashes, per-call samples,
medians, round order, thread limits, and batch repeat counts.
Use the individual samples to understand run-to-run spread; do not treat the
median as a universal language speed ratio. Matrix multiplication and stencil
convolution exercise different memory and arithmetic patterns, and results can
change with CPU model, compiler version, NumPy/BLAS backend, OS, or workload size.
