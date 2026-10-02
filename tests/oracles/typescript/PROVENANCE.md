# TypeScript Oracle Provenance

This directory is a test-only snapshot of the TypeScript CalcKernel compiler.
It is not part of the CK language or ABI contract and is not shipped in native
release archives.

- Source commit: `5e989939d89d75056e5f3bea25f3bf7204d5529a`
- Source tree: `445743ef4d270ba7a26a5402243ce0bb606fb44b`
- Declared license: MIT (`package.json`)

The original `src/`, fixtures, and `tsconfig.json` were copied byte-for-byte
from the detached origin commit. `package.json` and `pnpm-lock.yaml` have since
received dependency-only security maintenance: Vitest was updated from 3.2.6 to
4.1.11 in reviewed public PR #5 (`cd5cb14b7362ea6b1de3b0de979ec58e4a8ff242`),
and esbuild was pinned to 0.28.1 to address GHSA-g7r4-m6w7-qqqr. These changes
do not alter oracle behavior. `SOURCE_MANIFEST.sha256` records every included
source, configuration, lock, and fixture byte sequence.

The quality job verifies the source manifest, installs the lockfile exactly,
builds the oracle locally, and then runs the existing live C/WASM/CLI/fixture
differential gates. Generated `dist/` and dependency directories remain ignored.
Dependency maintenance requires a reviewed maintenance commit, refreshed
source manifest, complete diff review, and a full rerun of the differential
gates. Replacing the source or fixture snapshot requires a reviewed origin
commit, refreshed source tree identity and source manifest, and a full rerun of
the differential gates.
