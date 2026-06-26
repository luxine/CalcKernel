export interface CKWasmArenaOptions {
  heapBase?: number;
}

export interface CKWasmMemory {
  buffer: ArrayBuffer;
  grow(delta: number): number;
}

export interface CKWasmGlobal {
  value: number | bigint;
}

export interface CKWasmArenaCopy<T extends ArrayBufferView> {
  ptr: number;
  view: T;
}

const wasmPageBytes = 64 * 1024;

export class CKWasmArena {
  readonly memory: CKWasmMemory;

  private buffer: ArrayBuffer;
  private nextOffset: number | undefined;

  constructor(memory: CKWasmMemory, options: CKWasmArenaOptions = {}) {
    if (!isWasmMemory(memory)) {
      throw new Error("CKWasmArena requires a WebAssembly.Memory instance.");
    }

    this.memory = memory;
    this.buffer = memory.buffer;
    this.nextOffset = options.heapBase === undefined ? undefined : checkedNonNegativeInteger(options.heapBase, "heapBase");
  }

  static fromExports(exports: Record<string, unknown>, options: CKWasmArenaOptions = {}): CKWasmArena {
    const memory = exports.memory;
    if (!isWasmMemory(memory)) {
      throw new Error("CKWasmArena.fromExports requires exports.memory to be a WebAssembly.Memory.");
    }

    return new CKWasmArena(memory, {
      heapBase: options.heapBase ?? CKWasmArena.heapBaseFromExports(exports)
    });
  }

  static heapBaseFromExports(exports: Record<string, unknown>): number | undefined {
    return exportedHeapBase(exports.__ck_heap_base) ?? exportedHeapBase(exports.__heap_base);
  }

  ensureBytes(bytes: number): void {
    const requiredBytes = checkedNonNegativeInteger(bytes, "bytes");
    this.refreshViewsIfNeeded();

    if (requiredBytes <= this.buffer.byteLength) {
      return;
    }

    const currentPages = Math.ceil(this.buffer.byteLength / wasmPageBytes);
    const requiredPages = Math.ceil(requiredBytes / wasmPageBytes);
    this.memory.grow(requiredPages - currentPages);
    this.refreshViewsIfNeeded();
  }

  refreshViewsIfNeeded(): void {
    if (this.buffer !== this.memory.buffer) {
      this.buffer = this.memory.buffer;
    }
  }

  allocBytes(bytes: number, align: number): number {
    if (this.nextOffset === undefined) {
      throw new Error("CKWasmArena allocation requires a heapBase option or exported __ck_heap_base / __heap_base.");
    }

    const byteLength = checkedNonNegativeInteger(bytes, "bytes");
    const alignment = checkedPositiveInteger(align, "align");
    const ptr = alignTo(this.nextOffset, alignment);
    const end = checkedByteEnd(ptr, byteLength);
    this.ensureBytes(end);
    this.nextOffset = end;
    return ptr;
  }

  allocF64(length: number): number {
    return this.allocTyped(length, Float64Array.BYTES_PER_ELEMENT);
  }

  allocI32(length: number): number {
    return this.allocTyped(length, Int32Array.BYTES_PER_ELEMENT);
  }

  allocU32(length: number): number {
    return this.allocTyped(length, Uint32Array.BYTES_PER_ELEMENT);
  }

  allocI64(length: number): number {
    return this.allocTyped(length, BigInt64Array.BYTES_PER_ELEMENT);
  }

  allocU64(length: number): number {
    return this.allocTyped(length, BigUint64Array.BYTES_PER_ELEMENT);
  }

  viewF64(ptr: number, length: number): Float64Array {
    return this.viewTyped(Float64Array, ptr, length, Float64Array.BYTES_PER_ELEMENT);
  }

  viewI32(ptr: number, length: number): Int32Array {
    return this.viewTyped(Int32Array, ptr, length, Int32Array.BYTES_PER_ELEMENT);
  }

  viewU32(ptr: number, length: number): Uint32Array {
    return this.viewTyped(Uint32Array, ptr, length, Uint32Array.BYTES_PER_ELEMENT);
  }

  viewI64(ptr: number, length: number): BigInt64Array {
    return this.viewTyped(BigInt64Array, ptr, length, BigInt64Array.BYTES_PER_ELEMENT);
  }

  viewU64(ptr: number, length: number): BigUint64Array {
    return this.viewTyped(BigUint64Array, ptr, length, BigUint64Array.BYTES_PER_ELEMENT);
  }

  copyInF64(src: Float64Array): CKWasmArenaCopy<Float64Array> {
    const ptr = this.allocF64(src.length);
    const view = this.viewF64(ptr, src.length);
    view.set(src);
    return { ptr, view };
  }

  copyInI32(src: Int32Array): CKWasmArenaCopy<Int32Array> {
    const ptr = this.allocI32(src.length);
    const view = this.viewI32(ptr, src.length);
    view.set(src);
    return { ptr, view };
  }

  copyInU32(src: Uint32Array): CKWasmArenaCopy<Uint32Array> {
    const ptr = this.allocU32(src.length);
    const view = this.viewU32(ptr, src.length);
    view.set(src);
    return { ptr, view };
  }

  copyOutF64(ptr: number, length: number): Float64Array {
    return new Float64Array(this.viewF64(ptr, length));
  }

  private allocTyped(length: number, bytesPerElement: number): number {
    const elementCount = checkedNonNegativeInteger(length, "length");
    return this.allocBytes(checkedByteLength(elementCount, bytesPerElement), bytesPerElement);
  }

  private viewTyped<T extends ArrayBufferView>(
    ctor: { new (buffer: ArrayBuffer, byteOffset: number, length: number): T },
    ptr: number,
    length: number,
    bytesPerElement: number
  ): T {
    const byteOffset = checkedNonNegativeInteger(ptr, "ptr");
    const elementCount = checkedNonNegativeInteger(length, "length");
    if (byteOffset % bytesPerElement !== 0) {
      throw new Error(`ptr must be ${bytesPerElement}-byte aligned: ${ptr}`);
    }

    this.ensureBytes(checkedByteEnd(byteOffset, checkedByteLength(elementCount, bytesPerElement)));
    return new ctor(this.buffer, byteOffset, elementCount);
  }
}

function alignTo(value: number, align: number): number {
  return Math.ceil(value / align) * align;
}

function checkedPositiveInteger(value: number, name: string): number {
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new Error(`${name} must be a positive safe integer.`);
  }
  return value;
}

function checkedNonNegativeInteger(value: number, name: string): number {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new Error(`${name} must be a non-negative safe integer.`);
  }
  return value;
}

function checkedByteLength(length: number, bytesPerElement: number): number {
  const byteLength = length * bytesPerElement;
  if (!Number.isSafeInteger(byteLength)) {
    throw new Error("TypedArray byte length exceeds JavaScript safe integer range.");
  }
  return byteLength;
}

function checkedByteEnd(ptr: number, byteLength: number): number {
  const end = ptr + byteLength;
  if (!Number.isSafeInteger(end)) {
    throw new Error("WASM memory range exceeds JavaScript safe integer range.");
  }
  return end;
}

function isWasmMemory(value: unknown): value is CKWasmMemory {
  return (
    typeof value === "object" &&
    value !== null &&
    (value as { buffer?: unknown }).buffer instanceof ArrayBuffer &&
    typeof (value as { grow?: unknown }).grow === "function"
  );
}

function exportedHeapBase(value: unknown): number | undefined {
  if (value === undefined) {
    return undefined;
  }

  if (typeof value === "number") {
    return checkedNonNegativeInteger(value, "heapBase");
  }

  if (isWasmGlobal(value)) {
    const globalValue = value.value;
    if (typeof globalValue === "number") {
      return checkedNonNegativeInteger(globalValue, "heapBase");
    }
    if (typeof globalValue === "bigint") {
      if (globalValue < 0n || globalValue > BigInt(Number.MAX_SAFE_INTEGER)) {
        throw new Error("heapBase global must fit in a JavaScript safe integer.");
      }
      return Number(globalValue);
    }
  }

  throw new Error("heapBase export must be a number or WebAssembly.Global.");
}

function isWasmGlobal(value: unknown): value is CKWasmGlobal {
  return (
    typeof value === "object" &&
    value !== null &&
    (typeof (value as { value?: unknown }).value === "number" || typeof (value as { value?: unknown }).value === "bigint")
  );
}
