import { describe, expect, it } from 'vitest';
import { validateServerVersion } from '../src/serverVersion';

describe('validateServerVersion', () => {
  it('accepts the matching CK minor version', () => {
    expect(validateServerVersion('ckc 0.14.0\n')).toBeUndefined();
    expect(validateServerVersion('ckc 0.14.8\n')).toBeUndefined();
  });

  it('rejects an older compiler', () => {
    expect(validateServerVersion('ckc 0.8.0\n')).toContain('0.14');
  });

  it('rejects output from an unrelated executable', () => {
    expect(validateServerVersion('hello world\n')).toContain('ckc');
  });
});
