export const REQUIRED_CKC_MINOR = '0.14';

export function validateServerVersion(versionOutput: string): string | undefined {
  const match = /^ckc (\d+)\.(\d+)\.(\d+)(?:\s|$)/.exec(versionOutput.trim());
  if (!match) {
    return 'Expected ckc 0.14.x; the selected executable did not report a ckc version.';
  }
  if (match[1] + '.' + match[2] !== REQUIRED_CKC_MINOR) {
    return 'Expected ckc ' + REQUIRED_CKC_MINOR + '.x, found ' + match[0].trim() + '.';
  }
  return undefined;
}
