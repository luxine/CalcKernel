import { runTests } from '@vscode/test-electron';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, rmSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { platformTarget } from '../scripts/packageCore.mjs';

const extensionRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const server = process.env.CK_VSCODE_TEST_CKC ?? resolve(extensionRoot, '../../target/debug/ckc');
const profile = mkdtempSync('/tmp/ckv-');
const packaged = process.argv.includes('--vsix');
const workspaceFolder = resolve(profile, 'workspace');
mkdirSync(workspaceFolder);

try {
  let extensionDevelopmentPath = extensionRoot;
  if (packaged) {
    const packageJson = JSON.parse(await readFile(resolve(extensionRoot, 'package.json'), 'utf8'));
    const target = platformTarget(process.platform, process.arch);
    const vsix = resolve(extensionRoot, 'out', `${packageJson.name}-${packageJson.version}-${target}.vsix`);
    const extracted = resolve(profile, 'extracted');
    const unzip = spawnSync('unzip', ['-q', vsix, '-d', extracted], { encoding: 'utf8' });
    if (unzip.error || unzip.status !== 0) {
      throw new Error('Cannot extract VSIX: ' + String(unzip.error ?? unzip.stderr));
    }
    extensionDevelopmentPath = resolve(extracted, 'extension');
  }
  await runTests({
    extensionDevelopmentPath,
    extensionTestsPath: resolve(extensionRoot, 'test/integration/index.cjs'),
    vscodeVersion: process.env.CK_VSCODE_TEST_VERSION ?? 'stable',
    launchArgs: [
      workspaceFolder,
      '--disable-extensions',
      '--disable-workspace-trust',
      '--user-data-dir=' + profile,
      '--extensions-dir=' + resolve(profile, 'extensions')
    ],
    extensionTestsEnv: {
      CK_VSCODE_TEST_CKC: packaged ? '' : server,
      CK_VSCODE_TEST_WORKSPACE: workspaceFolder
    }
  });
} catch (error) {
  console.error(error);
  process.exitCode = 1;
} finally {
  rmSync(profile, { recursive: true, force: true });
}
