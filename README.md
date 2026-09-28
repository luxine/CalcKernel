# CalcKernel

[Website](https://calckernel.org/) · [Get started](https://calckernel.org/docs/getting-started/) · [Documentation](https://calckernel.org/docs/) · [Download](https://github.com/luxine/CalcKernel/releases/latest) · [VS Code extension](https://marketplace.visualstudio.com/items?itemName=Luxine.calckernel-vscode-plugin) · [Performance measurements](https://calckernel.org/#performance) · [MIT license](LICENSE)

CalcKernel (CK) is a statically typed language for numerical computation kernels: focused calculations that can be called from a larger application. Write a `.ck` source file and use `ckc` to check, run, or build it. CK can produce native programs and libraries, or emit C and WebAssembly for integration with existing systems.

CK is designed to handle the calculation while your application keeps responsibility for the surrounding work, such as user interfaces, input and output, and data storage. Its typed function interfaces and caller-owned data slices make that boundary explicit.

The latest stable release is 0.15.0.

## Why CalcKernel

- **Focused numerical code.** Express calculations with typed functions, structures, integer and floating-point values, conditions, and loops.
- **Native builds.** Compile CK programs and libraries to native machine code, with compiler optimizations for eligible calculations.
- **Flexible integration.** Build a native library with a C ABI, or emit C source or a WebAssembly module.
- **WebAssembly optimization.** O3 can use guarded bulk-memory operations for eligible copies and byte fills. The opt-in `simd128` profile vectorizes supported array maps and integer reductions; unsupported cases keep scalar code.
- **Explicit data ownership.** Applications provide and retain ownership of memory passed to CK. Native and C builds can also enable integer-overflow and slice-bounds checks.

## Start using CK

1. [Download the compiler](https://github.com/luxine/CalcKernel/releases/latest) for your system.
2. Follow the [first-program tutorial](https://calckernel.org/docs/getting-started/) to create and run a CK program.
3. Use any text editor, or install the optional [CalcKernel extension for Visual Studio Code](https://marketplace.visualstudio.com/items?itemName=Luxine.calckernel-vscode-plugin).

The extension provides CK syntax support, live diagnostics, code navigation, and commands to check, run, and build the current file. See the [VS Code guide](https://calckernel.org/docs/vscode/) for setup details.

## Learn more

- [Language introduction](https://calckernel.org/docs/language/)
- [Full documentation](https://calckernel.org/docs/)
- [Performance guide and measurements](https://calckernel.org/docs/performance/)
- [Performance chart](https://calckernel.org/#performance)
- [Source code](https://github.com/luxine/CalcKernel)

Performance depends on the algorithm, implementation, libraries, and target machine. The linked measurements describe specific workloads and are not a general ranking of languages.

## Build from source

The default feature set builds and tests the frontend, C, and WebAssembly components without LLVM:

```sh
cargo test --locked
cargo build --release --locked
```

Native builds require the pinned static LLVM prefix specified in [`native/llvm/manifest.toml`](native/llvm/manifest.toml). The bootstrap script checks the source archive's SHA-256 before building it:

```sh
rustc_host="$(rustc -vV | sed -n 's/^host: //p')"
llvm_archive="$PWD/llvm-project-22.1.8.src.tar.xz"
curl -fL 'https://github.com/llvm/llvm-project/releases/download/llvmorg-22.1.8/llvm-project-22.1.8.src.tar.xz' -o "$llvm_archive"
llvm_prefix="$PWD/build/llvm/prefix-$rustc_host-release"
./scripts/bootstrap-llvm.sh --archive "$llvm_archive" \
  --prefix "$llvm_prefix" --target "$rustc_host" --profile release
CKC_LLVM_PREFIX="$llvm_prefix" cargo build --release --features native-toolchain --locked
```

This prefix is needed to build CalcKernel's Native feature; release binaries do not require it at runtime. Windows MSVC build requirements are described in the [getting-started guide](docs/guides/getting-started.md).

## Community

Contributions are welcome. Please read the [contributing guide](CONTRIBUTING.md), [security policy](SECURITY.md), and [code of conduct](CODE_OF_CONDUCT.md).

## License

CalcKernel is distributed under the [MIT License](LICENSE).
