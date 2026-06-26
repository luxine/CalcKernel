# Naming Conventions

CalcKernel uses one canonical language identity across source files, generated artifacts, documentation, tests, and release packaging.

## Canonical Names

- Language name: CK / CalcKernel
- Compiler command: `ckc`
- Source file extension: `.ck`
- C ABI prefix: `CK_`

The project does not support alternate compiler command aliases or alternate source suffix aliases.

## Rules

- Use CK / CalcKernel for the language and project name.
- Use `ckc` in every CLI example.
- Use `.ck` for every source file.
- Use `CK_API`, `CK_BUILD_DLL`, `CK_Status`, `CK_OK`, and `CK_ERR_*` in the generated C ABI.
- Keep examples under `examples/*.ck`.
- Keep tests and snapshots aligned with `ckc` and `.ck`.
- Do not add compatibility aliases unless a future user request explicitly changes this policy.

## Examples

```sh
ckc check examples/pricing.ck
```

```sh
ckc emit-c examples/pricing.ck \
  --out build/pricing.c \
  --header build/pricing.h
```

```sh
ckc emit-wasm examples/pricing.ck \
  --out build/pricing.wasm
```

```sh
ckc emit-llvm examples/pricing.ck \
  --out build/pricing.ll
```
