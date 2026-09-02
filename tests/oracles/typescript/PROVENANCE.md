# TypeScript Oracle Provenance

This directory is a test-only snapshot of the retired TypeScript CalcKernel
compiler. It is not part of the CK language or ABI contract and is not shipped
in native release archives.

- Origin: `https://github.com/luxine/CalcKernel_retire`
- Commit: `5e989939d89d75056e5f3bea25f3bf7204d5529a`
- Origin tree: `445743ef4d270ba7a26a5402243ce0bb606fb44b`
- Declared license: MIT (`package.json`)

The snapshot contains the original `src/`, `package.json`, `pnpm-lock.yaml`,
`tsconfig.json`, and only the CK example/benchmark fixtures exercised by the
Rust differential suites. Those files were copied byte-for-byte from the
detached origin commit. `SOURCE_MANIFEST.sha256` records every included source,
configuration, lock, and fixture byte sequence.

The quality job verifies the source manifest, installs the lockfile exactly,
builds the oracle locally, and then runs the existing live C/WASM/CLI/fixture
differential gates. Generated `dist/` and dependency directories remain ignored.
Changing the snapshot requires a reviewed origin commit, refreshed tree identity
and source manifest, and a full rerun of the differential gates.
