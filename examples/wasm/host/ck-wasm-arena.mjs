const WASM_PAGE_BYTES = 64 * 1024;
const WASM32_LIMIT = 0x1_0000_0000;
const WASM32_MAX_ADDRESS = WASM32_LIMIT - 1;

/**
 * Create a monotonic allocator for a CK module's exported linear memory.
 *
 * The arena owns only the byte range starting at heapBase. It does not free or
 * validate pointers owned by other code. Typed views returned by this class
 * are snapshots of the current memory buffer; request a new view after growth.
 */
export function createCKWasmArena(instanceOrExports, options = {}) {
  if (typeof options !== 'object' || options === null) {
    throw new TypeError('options must be an object');
  }
  const exports = exportsFrom(instanceOrExports);
  const memory = exports.memory;
  const heapBase = options.heapBase ?? heapBaseFromExports(exports);
  if (heapBase === undefined) {
    throw new TypeError(
      'CKWasmArena requires heapBase or an exported __ck_heap_base/__heap_base value',
    );
  }
  return new CKWasmArena(memory, { heapBase });
}

export class CKWasmArena {
  #memory;
  #buffer;
  #heapBase;
  #nextOffset;

  constructor(memory, { heapBase } = {}) {
    if (!isWasmMemory(memory)) {
      throw new TypeError('memory must be a WebAssembly.Memory instance');
    }
    if (heapBase === undefined) {
      throw new TypeError('heapBase must be provided when constructing CKWasmArena directly');
    }
    this.#memory = memory;
    this.#buffer = memory.buffer;
    this.#heapBase = checkedWasmAddress(heapBase, 'heapBase');
    this.#nextOffset = this.#heapBase;
    this.#checkMemorySize(this.#buffer.byteLength);
  }

  get memory() {
    return this.#memory;
  }

  get heapBase() {
    return this.#heapBase;
  }

  get buffer() {
    return this.#refreshBuffer();
  }

  /** The next byte address that will be considered for allocation. */
  get nextOffset() {
    return this.#nextOffset;
  }

  reserve(bytes) {
    const required = checkedWasmEnd(bytes, 'bytes');
    this.#ensureBytes(required);
  }

  ensureBytes(bytes) {
    this.reserve(bytes);
  }

  refreshViewsIfNeeded() {
    return this.#refreshBuffer();
  }

  allocBytes(bytes, alignment = 1) {
    const byteLength = checkedWasmEnd(bytes, 'bytes');
    const align = checkedPositiveInteger(alignment, 'alignment');
    const start = alignUp(this.#nextOffset, align);
    if (start > WASM32_MAX_ADDRESS) {
      throw new RangeError('aligned allocation start is outside the Wasm32 address range');
    }
    const end = checkedWasmEndFrom(start, byteLength, 'allocation');

    // Keep the cursor unchanged if memory.grow throws or fails to provide the
    // requested size.
    this.#ensureBytes(end);
    this.#nextOffset = end;
    return start;
  }

  allocI32(length) {
    return this.#allocTyped(length, Int32Array.BYTES_PER_ELEMENT, 'i32');
  }

  allocU32(length) {
    return this.#allocTyped(length, Uint32Array.BYTES_PER_ELEMENT, 'u32');
  }

  allocI64(length) {
    return this.#allocTyped(length, BigInt64Array.BYTES_PER_ELEMENT, 'i64');
  }

  allocU64(length) {
    return this.#allocTyped(length, BigUint64Array.BYTES_PER_ELEMENT, 'u64');
  }

  allocF64(length) {
    return this.#allocTyped(length, Float64Array.BYTES_PER_ELEMENT, 'f64');
  }

  viewI32(ptr, length) {
    return this.#viewTyped(Int32Array, ptr, length, 'i32');
  }

  viewU32(ptr, length) {
    return this.#viewTyped(Uint32Array, ptr, length, 'u32');
  }

  viewI64(ptr, length) {
    return this.#viewTyped(BigInt64Array, ptr, length, 'i64');
  }

  viewU64(ptr, length) {
    return this.#viewTyped(BigUint64Array, ptr, length, 'u64');
  }

  viewF64(ptr, length) {
    return this.#viewTyped(Float64Array, ptr, length, 'f64');
  }

  viewData(ptr, bytes) {
    const start = checkedWasmAddress(ptr, 'ptr');
    const byteLength = checkedWasmEnd(bytes, 'bytes');
    const end = checkedWasmEndFrom(start, byteLength, 'DataView');
    this.#ensureBytes(end);
    return new DataView(this.#refreshBuffer(), start, byteLength);
  }

  copyInI32(source) {
    return this.#copyIn(source, Int32Array, 'i32');
  }

  copyInU32(source) {
    return this.#copyIn(source, Uint32Array, 'u32');
  }

  copyInI64(source) {
    return this.#copyIn(source, BigInt64Array, 'i64');
  }

  copyInU64(source) {
    return this.#copyIn(source, BigUint64Array, 'u64');
  }

  copyInF64(source) {
    return this.#copyIn(source, Float64Array, 'f64');
  }

  copyOutI32(ptr, length) {
    return this.viewI32(ptr, length).slice();
  }

  copyOutU32(ptr, length) {
    return this.viewU32(ptr, length).slice();
  }

  copyOutI64(ptr, length) {
    return this.viewI64(ptr, length).slice();
  }

  copyOutU64(ptr, length) {
    return this.viewU64(ptr, length).slice();
  }

  copyOutF64(ptr, length) {
    return this.viewF64(ptr, length).slice();
  }

  #allocTyped(length, bytesPerElement, label) {
    const count = checkedNonNegativeSafeInteger(length, 'length');
    const byteLength = checkedByteLength(count, bytesPerElement, label);
    return this.allocBytes(byteLength, bytesPerElement);
  }

  #viewTyped(Constructor, ptr, length, label) {
    const start = checkedWasmAddress(ptr, 'ptr');
    const count = checkedNonNegativeSafeInteger(length, 'length');
    const bytesPerElement = Constructor.BYTES_PER_ELEMENT;
    if (start % bytesPerElement !== 0) {
      throw new RangeError(`ptr must be aligned to ${bytesPerElement} bytes for ${label}`);
    }
    const byteLength = checkedByteLength(count, bytesPerElement, label);
    const end = checkedWasmEndFrom(start, byteLength, label);
    this.#ensureBytes(end);
    return new Constructor(this.#refreshBuffer(), start, count);
  }

  #copyIn(source, Constructor, label) {
    if (!(source instanceof Constructor)) {
      throw new TypeError(`copyIn${label.toUpperCase()} expects a ${Constructor.name}`);
    }
    // Allocation may grow this memory and detach a source view backed by it.
    // Take a stable copy before reserving destination space in that case.
    const stableSource = source.buffer === this.#memory.buffer ? new Constructor(source) : source;
    const ptr = this.#allocTyped(stableSource.length, Constructor.BYTES_PER_ELEMENT, label);
    const view = this.#viewTyped(Constructor, ptr, stableSource.length, label);
    view.set(stableSource);
    return { ptr, view };
  }

  #ensureBytes(requiredBytes) {
    this.#checkMemorySize(requiredBytes);
    let currentBuffer = this.#refreshBuffer();
    if (requiredBytes <= currentBuffer.byteLength) return;

    const currentPages = currentBuffer.byteLength / WASM_PAGE_BYTES;
    const requiredPages = Math.ceil(requiredBytes / WASM_PAGE_BYTES);
    const pagesToGrow = requiredPages - currentPages;
    try {
      this.#memory.grow(pagesToGrow);
    } catch (error) {
      throw new RangeError(
        `memory.grow failed while reserving ${requiredBytes} bytes (${currentPages} to ${requiredPages} pages): ${errorMessage(error)}`,
        { cause: error },
      );
    }

    currentBuffer = this.#refreshBuffer();
    if (currentBuffer.byteLength < requiredBytes) {
      throw new RangeError(`memory.grow completed but memory is still smaller than ${requiredBytes} bytes`);
    }
  }

  #refreshBuffer() {
    const current = this.#memory.buffer;
    this.#checkMemorySize(current.byteLength);
    if (this.#buffer !== current) this.#buffer = current;
    return this.#buffer;
  }

  #checkMemorySize(byteLength) {
    if (!Number.isSafeInteger(byteLength) || byteLength < 0 || byteLength > WASM32_LIMIT) {
      throw new RangeError(`memory size ${byteLength} exceeds the Wasm32 address range`);
    }
  }
}

function exportsFrom(instanceOrExports) {
  if (typeof instanceOrExports !== 'object' || instanceOrExports === null) {
    throw new TypeError('expected a WebAssembly.Instance or its exports object');
  }
  const exports = isWasmMemory(instanceOrExports.memory)
    ? instanceOrExports
    : 'exports' in instanceOrExports
      ? instanceOrExports.exports
      : instanceOrExports;
  if (typeof exports !== 'object' || exports === null) {
    throw new TypeError('instance.exports must be an object');
  }
  return exports;
}

function heapBaseFromExports(exports) {
  const exported = exports.__ck_heap_base ?? exports.__heap_base;
  if (exported === undefined) return undefined;
  if (isWasmGlobal(exported)) {
    const raw = exported.value;
    // The compiler exports an i32 global. Its JavaScript numeric value is
    // signed, while a Wasm32 byte address uses the corresponding u32 bits.
    if (typeof raw === 'number' && Number.isInteger(raw)) return raw >>> 0;
    if (typeof raw === 'bigint' && raw >= 0n && raw <= BigInt(WASM32_MAX_ADDRESS)) return Number(raw);
    throw new RangeError('exported heap base global is not a Wasm32 address');
  }
  return checkedWasmAddress(exported, '__ck_heap_base/__heap_base');
}

function isWasmMemory(value) {
  return typeof WebAssembly !== 'undefined' &&
    typeof WebAssembly.Memory === 'function' && value instanceof WebAssembly.Memory;
}

function isWasmGlobal(value) {
  return typeof WebAssembly !== 'undefined' &&
    typeof WebAssembly.Global === 'function' && value instanceof WebAssembly.Global;
}

function checkedNonNegativeSafeInteger(value, name) {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new RangeError(`${name} must be a non-negative safe integer`);
  }
  return value;
}

function checkedPositiveInteger(value, name) {
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new RangeError(`${name} must be a positive safe integer`);
  }
  return value;
}

function checkedWasmAddress(value, name) {
  const address = checkedNonNegativeSafeInteger(value, name);
  if (address > WASM32_MAX_ADDRESS) {
    throw new RangeError(`${name} is outside the Wasm32 address range`);
  }
  return address;
}

function checkedWasmEnd(value, name) {
  const byteLength = checkedNonNegativeSafeInteger(value, name);
  if (byteLength > WASM32_LIMIT) {
    throw new RangeError(`${name} exceeds the Wasm32 address range`);
  }
  return byteLength;
}

function checkedWasmEndFrom(start, length, name) {
  const end = start + length;
  if (!Number.isSafeInteger(end) || end > WASM32_LIMIT) {
    throw new RangeError(`${name} exceeds the Wasm32 address range`);
  }
  return end;
}

function checkedByteLength(length, bytesPerElement, label) {
  const byteLength = length * bytesPerElement;
  if (!Number.isSafeInteger(byteLength) || byteLength > WASM32_LIMIT) {
    throw new RangeError(`${label} byte length exceeds the Wasm32 address range`);
  }
  return byteLength;
}

function alignUp(value, alignment) {
  const remainder = value % alignment;
  const aligned = remainder === 0 ? value : value + (alignment - remainder);
  if (!Number.isSafeInteger(aligned) || aligned > WASM32_LIMIT) {
    throw new RangeError('aligned allocation start exceeds the Wasm32 address range');
  }
  return aligned;
}

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}
