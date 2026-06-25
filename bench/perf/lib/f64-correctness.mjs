export const defaultF64Tolerance = {
  absTol: 1e-6,
  relTol: 1e-8
};

export function withinTolerance(actual, expected, tolerance = defaultF64Tolerance) {
  if (!Number.isFinite(actual) || !Number.isFinite(expected)) {
    return false;
  }

  const absTol = tolerance.absTol ?? defaultF64Tolerance.absTol;
  const relTol = tolerance.relTol ?? defaultF64Tolerance.relTol;
  const diff = Math.abs(actual - expected);
  const scale = Math.max(1, Math.abs(actual), Math.abs(expected));

  return diff <= absTol || diff <= relTol * scale;
}

export function assertWithinTolerance(actual, expected, label, tolerance = defaultF64Tolerance) {
  if (!withinTolerance(actual, expected, tolerance)) {
    throw new Error(`${label} mismatch: expected ${expected}, got ${actual}`);
  }
}
