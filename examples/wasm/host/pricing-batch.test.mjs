import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { createCKWasmArena } from './ck-wasm-arena.mjs';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const sourcePath = path.join(repoRoot, 'examples/wasm/pricing_batch.ck');

function compileFixture(outPath, optLevel) {
  const args = [
    'emit-wasm', sourcePath,
    '--out', outPath,
    '--overflow', 'unchecked',
    '--bounds', 'unchecked',
    '--opt-level', String(optLevel),
    '--wasm-features', 'baseline',
  ];
  const compiler = process.env.CKC ?? process.env.CARGO_BIN_EXE_ckc;
  const command = compiler ? [compiler, args] : ['cargo', ['run', '--quiet', '--bin', 'ckc', '--', ...args]];
  const result = spawnSync(command[0], command[1], {
    cwd: repoRoot,
    encoding: 'utf8',
    timeout: 120_000,
  });
  assert.equal(result.status, 0, `ckc emit-wasm failed: ${result.stderr || result.stdout}`);
  return readFileSync(outPath);
}

function expectedTotal(price, quantity, discount, taxRatePpm) {
  const afterDiscount = price * quantity - discount;
  return afterDiscount + (afterDiscount * taxRatePpm) / 1_000_000n;
}

test('one batch export matches per-record calls across repeated arena reuse at O0 and O3', (t) => {
  const directory = mkdtempSync(path.join(os.tmpdir(), 'ck-wasm-pricing-batch-'));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const count = 19;
  for (const optLevel of [0, 3]) {
    const wasmPath = path.join(directory, `pricing_batch_O${optLevel}.wasm`);
    const module = new WebAssembly.Module(compileFixture(wasmPath, optLevel));
    const instance = new WebAssembly.Instance(module);
    const arena = createCKWasmArena(instance);
    const columns = [
      new BigInt64Array(count),
      new BigInt64Array(count),
      new BigInt64Array(count),
      new BigInt64Array(count),
    ];
    const output = arena.allocI64(count);
    const outputView = arena.viewI64(output, count);
    const pointers = columns.map((column) => arena.copyInI64(column).ptr);

    for (let round = 0; round < 3; round += 1) {
      for (let index = 0; index < count; index += 1) {
        columns[0][index] = BigInt(101 + index + round);
        columns[1][index] = BigInt(1 + (index % 4));
        columns[2][index] = BigInt(index % 7);
        columns[3][index] = BigInt(100_000 + (index % 5) * 10_000);
      }

      for (let column = 0; column < columns.length; column += 1) {
        arena.viewI64(pointers[column], count).set(columns[column]);
      }
      const expected = Array.from({ length: count }, (_, index) => expectedTotal(
        columns[0][index], columns[1][index], columns[2][index], columns[3][index],
      ));
      const perRecord = Array.from({ length: count }, (_, index) => instance.exports.pricing_one(
        columns[0][index], columns[1][index], columns[2][index], columns[3][index],
      ));

      assert.deepEqual(perRecord, expected, `pricing_one O${optLevel} round ${round}`);
      assert.equal(instance.exports.pricing_batch(...pointers, output, count), 0);
      assert.deepEqual([...arena.viewI64(output, count)], expected, `pricing_batch O${optLevel} round ${round}`);
      assert.equal(outputView.buffer, instance.exports.memory.buffer);
    }
  }
});
