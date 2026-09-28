import assert from 'node:assert/strict';
import test from 'node:test';
import { createCKWasmArena } from './ck-wasm-arena.mjs';

function makeArena(memory, heapBase = 32) {
  return createCKWasmArena({
    memory,
    __ck_heap_base: new WebAssembly.Global({ value: 'i32' }, heapBase),
  });
}

test('allocates aligned typed regions and copies i64 input', () => {
  const memory = new WebAssembly.Memory({ initial: 1, maximum: 2 });
  const arena = makeArena(memory, 48);

  const first = arena.allocBytes(3);
  const values = arena.copyInI64(new BigInt64Array([11n, -7n]));

  assert.equal(first, 48);
  assert.equal(values.ptr, 56);
  assert.deepEqual([...arena.viewI64(values.ptr, 2)], [11n, -7n]);
  assert.deepEqual([...values.view], [11n, -7n]);
});

test('refreshes views after arena growth and after external memory growth', () => {
  const memory = new WebAssembly.Memory({ initial: 1, maximum: 3 });
  const arena = makeArena(memory, 65_528);
  const beforeGrow = arena.viewI32(0, 4);

  const ptr = arena.allocI64(2);
  assert.equal(ptr, 65_528);
  assert.equal(memory.buffer.byteLength, 2 * 65_536);
  assert.equal(beforeGrow.byteLength, 0);

  const current = arena.viewI64(ptr, 2);
  assert.equal(current.buffer, memory.buffer);
  current.set([3n, 5n]);

  memory.grow(1);
  const afterExternalGrow = arena.viewI64(ptr, 2);
  assert.equal(afterExternalGrow.buffer, memory.buffer);
  assert.deepEqual([...afterExternalGrow], [3n, 5n]);
});

test('copies a source view from the same memory before growth detaches it', () => {
  const memory = new WebAssembly.Memory({ initial: 1, maximum: 2 });
  const arena = makeArena(memory, 65_520);
  const source = arena.viewI64(0, 4);
  source.set([13n, -2n, 0x1234n, 99n]);

  const copy = arena.copyInI64(source);

  assert.equal(memory.buffer.byteLength, 2 * 65_536);
  assert.equal(source.byteLength, 0);
  assert.deepEqual([...copy.view], [13n, -2n, 0x1234n, 99n]);
});

test('does not advance the allocation cursor when memory growth fails', () => {
  const memory = new WebAssembly.Memory({ initial: 1, maximum: 1 });
  const arena = makeArena(memory, 65_528);

  assert.throws(() => arena.allocBytes(16), /memory\.grow failed/i);
  assert.equal(arena.allocBytes(8), 65_528);
});

test('rejects invalid lengths, alignment, and Wasm32 range overflow', () => {
  const memory = new WebAssembly.Memory({ initial: 1, maximum: 1 });
  const arena = makeArena(memory);

  assert.throws(() => arena.allocI64(-1), /length/i);
  assert.throws(() => arena.allocI64(Number.MAX_SAFE_INTEGER), /overflow|Wasm32/i);
  assert.throws(() => arena.allocBytes(1, 0), /align/i);
  assert.throws(() => arena.viewI64(33, 1), /aligned/i);

  const nearEnd = makeArena(memory, 0xffff_fffc);
  assert.throws(() => nearEnd.allocBytes(8), /Wasm32|range/i);
});
