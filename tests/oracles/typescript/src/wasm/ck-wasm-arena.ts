export interface CKWasmArenaOptions {
  heapBase?: number;
}

export interface CKWasmMemory {
  readonly buffer: ArrayBuffer;
  grow(delta: number): number;
}

export interface CKWasmGlobal {
  value: number | bigint;
}

export interface CKWasmArenaCopy<T extends ArrayBufferView> {
  ptr: number;
  view: T;
}

export interface CKWasmInstanceLike {
  exports: Record<string, unknown>;
}

const WASM_PAGE_BYTES = 64 * 1024;

type TypedArrayConstructor<T extends ArrayBufferView> = {
  readonly BYTES_PER_ELEMENT: number;
  new (buffer: ArrayBuffer, byteOffset: number, length: number): T;
};

export function createCKWasmArena(
  instanceOrExports: CKWasmInstanceLike | Record<string, unknown>,
  options: CKWasmArenaOptions = {},
): CKWasmArena {
  const api = "createCKWasmArena";
  const exports = exportsFromInstanceOrExports(instanceOrExports, api);
  const memory = exports.memory;
  if (!isWasmMemory(memory)) {
    throw arenaError(
      api,
      "exports.memory must be a WebAssembly.Memory instance",
      "Pass a WebAssembly.Instance, instance.exports, or an exports object containing the exported memory.",
    );
  }

  const heapBase = options.heapBase ?? CKWasmArena.heapBaseFromExports(exports);
  if (heapBase === undefined) {
    throw arenaError(
      api,
      "heapBase is missing; no options.heapBase, __ck_heap_base, or __heap_base value was found",
      "Pass { heapBase } explicitly or use CK / CalcKernel WASM output that exports __ck_heap_base.",
    );
  }

  return new CKWasmArena(memory, { heapBase });
}

export class CKWasmArena {
  readonly memory: CKWasmMemory;

  private buffer: ArrayBuffer;
  private nextOffset: number | undefined;

  constructor(memory: CKWasmMemory, options: CKWasmArenaOptions = {}) {
    const api = "CKWasmArena.constructor";
    if (!isWasmMemory(memory)) {
      throw arenaError(
        api,
        "memory must be a WebAssembly.Memory instance",
        "Pass instance.exports.memory or create a new WebAssembly.Memory({ initial }).",
      );
    }

    this.memory = memory;
    this.buffer = memory.buffer;
    this.nextOffset =
      options.heapBase === undefined
        ? undefined
        : checkedNonNegativeInteger(
            options.heapBase,
            "heapBase",
            api,
            "Pass a non-negative safe integer heapBase option, or omit it and export __ck_heap_base / __heap_base.",
          );
  }

  static fromExports(exports: Record<string, unknown>, options: CKWasmArenaOptions = {}): CKWasmArena {
    const api = "CKWasmArena.fromExports";
    const memory = exports.memory;
    if (!isWasmMemory(memory)) {
      throw arenaError(
        api,
        "exports.memory must be a WebAssembly.Memory instance",
        "Export memory from the WASM module or pass a WebAssembly.Memory directly to the constructor.",
      );
    }

    const heapBase = options.heapBase ?? CKWasmArena.heapBaseFromExports(exports);
    return new CKWasmArena(memory, { heapBase });
  }

  static heapBaseFromExports(exports: Record<string, unknown>): number | undefined {
    const api = "CKWasmArena.heapBaseFromExports";
    const candidate = exports.__ck_heap_base ?? exports.__heap_base;
    if (candidate === undefined) {
      return undefined;
    }

    return checkedNonNegativeInteger(
      exportedHeapBaseValue(candidate, api),
      "__ck_heap_base/__heap_base",
      api,
      "Export a non-negative safe integer heap base, or pass { heapBase } explicitly.",
    );
  }

  ensureBytes(bytes: number): void {
    this.ensureBytesForApi(bytes, "CKWasmArena.ensureBytes");
  }

  refreshViewsIfNeeded(): void {
    if (this.buffer !== this.memory.buffer) {
      this.buffer = this.memory.buffer;
    }
  }

  allocBytes(bytes: number, align: number): number {
    return this.allocBytesForApi(bytes, align, "CKWasmArena.allocBytes");
  }

  allocF64(length: number): number {
    return this.allocTyped(length, Float64Array.BYTES_PER_ELEMENT, "CKWasmArena.allocF64");
  }

  allocI32(length: number): number {
    return this.allocTyped(length, Int32Array.BYTES_PER_ELEMENT, "CKWasmArena.allocI32");
  }

  allocU32(length: number): number {
    return this.allocTyped(length, Uint32Array.BYTES_PER_ELEMENT, "CKWasmArena.allocU32");
  }

  allocI64(length: number): number {
    return this.allocTyped(length, BigInt64Array.BYTES_PER_ELEMENT, "CKWasmArena.allocI64");
  }

  allocU64(length: number): number {
    return this.allocTyped(length, BigUint64Array.BYTES_PER_ELEMENT, "CKWasmArena.allocU64");
  }

  viewF64(ptr: number, length: number): Float64Array {
    return this.viewTyped(Float64Array, ptr, length, Float64Array.BYTES_PER_ELEMENT, "CKWasmArena.viewF64");
  }

  viewI32(ptr: number, length: number): Int32Array {
    return this.viewTyped(Int32Array, ptr, length, Int32Array.BYTES_PER_ELEMENT, "CKWasmArena.viewI32");
  }

  viewU32(ptr: number, length: number): Uint32Array {
    return this.viewTyped(Uint32Array, ptr, length, Uint32Array.BYTES_PER_ELEMENT, "CKWasmArena.viewU32");
  }

  viewI64(ptr: number, length: number): BigInt64Array {
    return this.viewTyped(BigInt64Array, ptr, length, BigInt64Array.BYTES_PER_ELEMENT, "CKWasmArena.viewI64");
  }

  viewU64(ptr: number, length: number): BigUint64Array {
    return this.viewTyped(BigUint64Array, ptr, length, BigUint64Array.BYTES_PER_ELEMENT, "CKWasmArena.viewU64");
  }

  copyInF64(src: Float64Array): CKWasmArenaCopy<Float64Array> {
    assertTypedArray(src, Float64Array, "Float64Array", "CKWasmArena.copyInF64");
    const ptr = this.allocTyped(src.length, Float64Array.BYTES_PER_ELEMENT, "CKWasmArena.copyInF64");
    const view = this.viewTyped(Float64Array, ptr, src.length, Float64Array.BYTES_PER_ELEMENT, "CKWasmArena.copyInF64");
    view.set(src);
    return { ptr, view };
  }

  copyInI32(src: Int32Array): CKWasmArenaCopy<Int32Array> {
    assertTypedArray(src, Int32Array, "Int32Array", "CKWasmArena.copyInI32");
    const ptr = this.allocTyped(src.length, Int32Array.BYTES_PER_ELEMENT, "CKWasmArena.copyInI32");
    const view = this.viewTyped(Int32Array, ptr, src.length, Int32Array.BYTES_PER_ELEMENT, "CKWasmArena.copyInI32");
    view.set(src);
    return { ptr, view };
  }

  copyInU32(src: Uint32Array): CKWasmArenaCopy<Uint32Array> {
    assertTypedArray(src, Uint32Array, "Uint32Array", "CKWasmArena.copyInU32");
    const ptr = this.allocTyped(src.length, Uint32Array.BYTES_PER_ELEMENT, "CKWasmArena.copyInU32");
    const view = this.viewTyped(Uint32Array, ptr, src.length, Uint32Array.BYTES_PER_ELEMENT, "CKWasmArena.copyInU32");
    view.set(src);
    return { ptr, view };
  }

  copyOutF64(ptr: number, length: number): Float64Array {
    return new Float64Array(
      this.viewTyped(Float64Array, ptr, length, Float64Array.BYTES_PER_ELEMENT, "CKWasmArena.copyOutF64"),
    );
  }

  private allocTyped(length: number, bytesPerElement: number, api: string): number {
    const itemCount = checkedNonNegativeInteger(
      length,
      "length",
      api,
      "Pass a non-negative safe integer element count.",
    );
    const bytes = checkedByteLength(itemCount, bytesPerElement, api);
    return this.allocBytesForApi(bytes, bytesPerElement, api);
  }

  private allocBytesForApi(bytes: number, align: number, api: string): number {
    if (this.nextOffset === undefined) {
      throw arenaError(
        api,
        "allocation requires a heapBase option or exported __ck_heap_base / __heap_base",
        "Pass { heapBase } when constructing the arena, or export a heap base from the WASM module.",
      );
    }

    const byteCount = checkedNonNegativeInteger(
      bytes,
      "bytes",
      api,
      "Pass a non-negative safe integer byte count.",
    );
    const alignment = checkedPositiveInteger(
      align,
      "align",
      api,
      "Pass a positive safe integer alignment such as 4 for i32/u32 or 8 for f64/i64/u64.",
    );
    const ptr = alignTo(this.nextOffset, alignment);
    const end = checkedByteEnd(ptr, byteCount, api);
    this.ensureBytesForApi(end, api);
    this.nextOffset = end;
    return ptr;
  }

  private viewTyped<T extends ArrayBufferView>(
    ctor: TypedArrayConstructor<T>,
    ptr: number,
    length: number,
    bytesPerElement: number,
    api: string,
  ): T {
    const byteOffset = checkedNonNegativeInteger(
      ptr,
      "ptr",
      api,
      "Pass the byte offset returned by alloc*/copyIn*, or another non-negative WASM memory byte offset.",
    );
    if (byteOffset % bytesPerElement !== 0) {
      throw arenaError(
        api,
        `ptr must be ${bytesPerElement}-byte aligned; got ${byteOffset}`,
        "Use the matching alloc*/copyIn* method or align the pointer before creating the view.",
      );
    }

    const itemCount = checkedNonNegativeInteger(
      length,
      "length",
      api,
      "Pass a non-negative safe integer element count.",
    );
    const byteLength = checkedByteLength(itemCount, bytesPerElement, api);
    const requiredBytes = checkedByteEnd(byteOffset, byteLength, api);

    this.ensureBytesForApi(requiredBytes, api);
    this.refreshViewsIfNeeded();
    try {
      return new ctor(this.buffer, byteOffset, itemCount);
    } catch (error) {
      throw arenaError(
        api,
        `typed array view is out of bounds for ptr=${byteOffset}, length=${itemCount}`,
        `Ensure memory is large enough or call ensureBytes(${requiredBytes}) before creating the view. Cause: ${errorMessage(error)}`,
      );
    }
  }

  private ensureBytesForApi(bytes: number, api: string): void {
    const requiredBytes = checkedNonNegativeInteger(
      bytes,
      "bytes",
      api,
      "Pass a non-negative safe integer byte count.",
    );
    this.refreshViewsIfNeeded();
    if (requiredBytes <= this.buffer.byteLength) {
      return;
    }

    const currentPages = Math.ceil(this.buffer.byteLength / WASM_PAGE_BYTES);
    const requiredPages = Math.ceil(requiredBytes / WASM_PAGE_BYTES);
    const pagesToGrow = requiredPages - currentPages;
    try {
      this.memory.grow(pagesToGrow);
    } catch (error) {
      throw arenaError(
        api,
        `memory.grow failed while growing from ${currentPages} to ${requiredPages} WASM pages for ${requiredBytes} bytes`,
        `Pre-grow memory, increase the memory maximum, or pass smaller buffers. Cause: ${errorMessage(error)}`,
      );
    }

    this.refreshViewsIfNeeded();
    if (requiredBytes > this.buffer.byteLength) {
      throw arenaError(
        api,
        `memory.grow completed but memory is still too small for ${requiredBytes} bytes`,
        "Pre-grow memory with enough pages or increase the WebAssembly.Memory maximum.",
      );
    }
  }
}

function isWasmMemory(value: unknown): value is CKWasmMemory {
  const MemoryCtor = (globalThis as { WebAssembly?: { Memory?: Function } }).WebAssembly?.Memory;
  return typeof MemoryCtor === "function" && value instanceof MemoryCtor;
}

function exportsFromInstanceOrExports(value: CKWasmInstanceLike | Record<string, unknown>, api: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null) {
    throw arenaError(
      api,
      "instanceOrExports must be a WebAssembly.Instance-like object or an exports object",
      "Pass a WebAssembly.Instance, instance.exports, or an exports object containing memory.",
    );
  }

  const maybeExports = (value as { exports?: unknown }).exports;
  if (maybeExports !== undefined) {
    if (typeof maybeExports !== "object" || maybeExports === null) {
      throw arenaError(
        api,
        "instance.exports must be an object",
        "Pass a WebAssembly.Instance or its instance.exports object.",
      );
    }
    return maybeExports as Record<string, unknown>;
  }

  return value as Record<string, unknown>;
}

function isWasmGlobal(value: unknown): value is CKWasmGlobal {
  return typeof value === "object" && value !== null && "value" in value;
}

function exportedHeapBaseValue(value: unknown, api: string): number {
  const raw = isWasmGlobal(value) ? value.value : value;
  if (typeof raw === "bigint") {
    if (raw > BigInt(Number.MAX_SAFE_INTEGER)) {
      throw arenaError(
        api,
        "__ck_heap_base/__heap_base is larger than Number.MAX_SAFE_INTEGER",
        "Export a smaller heap base or pass a safe integer { heapBase } explicitly.",
      );
    }
    return Number(raw);
  }

  if (typeof raw !== "number") {
    throw arenaError(
      api,
      "__ck_heap_base/__heap_base must be a number or WebAssembly.Global value",
      "Export a numeric heap base or pass { heapBase } explicitly.",
    );
  }

  return raw;
}

function checkedNonNegativeInteger(value: number, name: string, api: string, fix: string): number {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw arenaError(api, `${name} must be a non-negative safe integer; got ${String(value)}`, fix);
  }
  return value;
}

function checkedPositiveInteger(value: number, name: string, api: string, fix: string): number {
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw arenaError(api, `${name} must be a positive safe integer; got ${String(value)}`, fix);
  }
  return value;
}

function checkedByteLength(length: number, bytesPerElement: number, api: string): number {
  const bytes = length * bytesPerElement;
  if (!Number.isSafeInteger(bytes)) {
    throw arenaError(
      api,
      `byte length must be a safe integer; length=${length}, bytesPerElement=${bytesPerElement}`,
      "Pass a smaller element count or split the buffer into multiple allocations.",
    );
  }
  return bytes;
}

function checkedByteEnd(ptr: number, byteLength: number, api: string): number {
  const end = ptr + byteLength;
  if (!Number.isSafeInteger(end)) {
    throw arenaError(
      api,
      `ptr + byte length must be a safe integer; ptr=${ptr}, byteLength=${byteLength}`,
      "Pass a smaller pointer/length pair or split the buffer into multiple views.",
    );
  }
  return end;
}

function alignTo(value: number, align: number): number {
  return Math.ceil(value / align) * align;
}

function assertTypedArray<T extends ArrayBufferView>(
  value: unknown,
  ctor: { new (...args: never[]): T },
  expected: string,
  api: string,
): asserts value is T {
  if (!(value instanceof ctor)) {
    const article = expected === "Int32Array" ? "an" : "a";
    throw arenaError(
      api,
      `src must be ${article} ${expected}`,
      `Pass ${article} ${expected} backed by the data you want to copy.`,
    );
  }
}

function arenaError(api: string, reason: string, fix: string): Error {
  return new Error(`${api}: ${reason}. ${fix}`);
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
