import { createF64Inputs, usesF64Y } from "./f64-workloads.mjs";

const f64Size = 8;

export function requiredBytesFor(len) {
  const xOffset = 0;
  const yOffset = alignTo(xOffset + len * f64Size, f64Size);

  return {
    xOffset,
    yOffset,
    totalBytes: yOffset + len * f64Size
  };
}

export function byteOffsetToF64Index(byteOffset) {
  if (!Number.isInteger(byteOffset) || byteOffset < 0 || byteOffset % f64Size !== 0) {
    throw new Error(`f64 byte offset must be a non-negative 8-byte aligned integer: ${byteOffset}`);
  }
  return byteOffset / f64Size;
}

export function ensureDataView(memory, requiredBytes) {
  ensureMemory(memory, requiredBytes);
  return new DataView(memory.buffer);
}

export function ensureFloat64Array(memory, requiredBytes) {
  ensureMemory(memory, requiredBytes);
  return new Float64Array(memory.buffer);
}

export function createLowCopyF64Inputs(len, kernel) {
  const inputs = createF64Inputs(len, "typedarray");
  const inputChecksum =
    checksumTypedArray(inputs.x, 0, len) + (usesF64Y(kernel) ? checksumTypedArray(inputs.y, 0, len) : 0.0);

  return { ...inputs, inputChecksum };
}

export function writeInputsFloat64Array(values, layout, inputs, kernel) {
  values.set(inputs.x, byteOffsetToF64Index(layout.xOffset));
  if (usesF64Y(kernel)) {
    values.set(inputs.y, byteOffsetToF64Index(layout.yOffset));
  }
  return inputs.inputChecksum;
}

export function checksumOutputFloat64Array(values, layout, len, kernel, scalarResult = 0.0) {
  switch (kernel) {
    case "axpy":
      return checksumTypedArray(values, byteOffsetToF64Index(layout.yOffset), len);
    case "scale":
      return checksumTypedArray(values, byteOffsetToF64Index(layout.xOffset), len);
    case "dot":
    case "sum":
      return scalarResult;
  }
}

function ensureMemory(memory, requiredBytes) {
  const pageSize = 64 * 1024;
  const currentPages = Math.ceil(memory.buffer.byteLength / pageSize);
  const requiredPages = Math.ceil(requiredBytes / pageSize);

  if (requiredPages > currentPages) {
    memory.grow(requiredPages - currentPages);
  }
}

function alignTo(value, align) {
  return Math.ceil(value / align) * align;
}

function checksumTypedArray(values, start, len) {
  let checksum = 0.0;
  for (let index = 0; index < len; index += 1) {
    checksum += values[start + index];
  }
  return checksum;
}
