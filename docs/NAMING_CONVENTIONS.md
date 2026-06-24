# Naming Conventions

IntKernel uses one canonical language identity across source files, generated artifacts, documentation, tests, and release packaging.

## Canonical Names

- Language name: IK / IntKernel
- Compiler command: `ikc`
- Source file extension: `.ik`

The project does not support alternate compiler command aliases or alternate source suffix aliases.

## Rules

- Use IK / IntKernel for the language and project name.
- Use `ikc` in every CLI example.
- Use `.ik` for every source file.
- Keep examples under `examples/*.ik`.
- Keep tests and snapshots aligned with `ikc` and `.ik`.
- Do not add compatibility aliases unless a future user request explicitly changes this policy.

## Examples

```sh
ikc check examples/pricing.ik
```

```sh
ikc emit-c examples/pricing.ik \
  --out build/pricing.c \
  --header build/pricing.h
```

```sh
ikc emit-wasm examples/pricing.ik \
  --out build/pricing.wasm
```

```sh
ikc emit-llvm examples/pricing.ik \
  --out build/pricing.ll
```
