import { runTests } from '@vscode/test-electron';
import { mkdtempSync, rmSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const extensionRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const server = process.env.CK_VSCODE_TEST_CKC ?? resolve(extensionRoot, '../../target/debug/ckc');
const profile = mkdtempSync('/tmp/ckv-');

try {
  await runTests({
    extensionDevelopmentPath: extensionRoot,
    extensionTestsPath: resolve(extensionRoot, 'test/integration/index.cjs'),
    vscodeVersion: process.env.CK_VSCODE_TEST_VERSION ?? 'stable',
    launchArgs: [
      '--disable-extensions',
      '--disable-workspace-trust',
      '--user-data-dir=' + profile,
      '--extensions-dir=' + resolve(profile, 'extensions')
    ],
    extensionTestsEnv: { CK_VSCODE_TEST_CKC: server }
  });
} catch (error) {
  console.error(error);
  process.exitCode = 1;
} finally {
  rmSync(profile, { recursive: true, force: true });
}
