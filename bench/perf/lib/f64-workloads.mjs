export const f64Kernels = ["axpy", "dot", "sum", "scale"];
export const f64Factor = 1.000001;

export function requireF64Kernel(value) {
  if (f64Kernels.includes(value)) {
    return value;
  }
  throw new Error(`--kernel must be one of: ${f64Kernels.join(", ")}`);
}

export function positiveInteger(value, flag) {
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed <= 0) {
    throw new Error(`${flag} must be a positive integer`);
  }
  return parsed;
}

export function initialF64X(index) {
  return 1.0 + (index % 251) * 0.125;
}

export function initialF64Y(index) {
  return -2.0 + (index % 199) * 0.0625;
}

export function createF64Inputs(len, kind) {
  const x = kind === "typedarray" ? new Float64Array(len) : new Array(len);
  const y = kind === "typedarray" ? new Float64Array(len) : new Array(len);

  for (let index = 0; index < len; index += 1) {
    x[index] = initialF64X(index);
    y[index] = initialF64Y(index);
  }

  return { x, y };
}

export function f64InputSums(len) {
  let xSum = 0.0;
  let ySum = 0.0;
  let xySum = 0.0;

  for (let index = 0; index < len; index += 1) {
    const x = initialF64X(index);
    const y = initialF64Y(index);
    xSum += x;
    ySum += y;
    xySum += x * y;
  }

  return { xSum, ySum, xySum };
}

export function expectedF64ComputeSink(kernel, len, iterations, factor = f64Factor) {
  const { xSum, ySum, xySum } = f64InputSums(len);

  switch (kernel) {
    case "axpy":
      return iterations * ySum + factor * xSum * (iterations * (iterations + 1)) / 2;
    case "dot":
      return iterations * xySum;
    case "sum":
      return iterations * xSum;
    case "scale": {
      let power = 1.0;
      let expected = 0.0;
      for (let iteration = 0; iteration < iterations; iteration += 1) {
        power *= factor;
        expected += xSum * power;
      }
      return expected;
    }
  }
}

export function expectedF64ResetSink(kernel, len, iterations, factor = f64Factor) {
  const { xSum, ySum, xySum } = f64InputSums(len);

  switch (kernel) {
    case "axpy":
      return iterations * (ySum + factor * xSum);
    case "dot":
      return iterations * xySum;
    case "sum":
      return iterations * xSum;
    case "scale":
      return iterations * factor * xSum;
  }
}

export function expectedF64MemorySink(kernel, len, iterations) {
  const { xSum, ySum } = f64InputSums(len);
  const perIteration = usesF64Y(kernel) ? xSum + ySum : xSum;
  return iterations * perIteration;
}

export function expectedF64OutputReadbackSink(kernel, len, iterations, factor = f64Factor) {
  const { xSum, ySum } = f64InputSums(len);

  switch (kernel) {
    case "axpy":
      return iterations * (xSum + ySum + factor * xSum);
    case "dot":
      return iterations * (xSum + ySum);
    case "sum":
      return iterations * xSum;
    case "scale":
      return iterations * factor * xSum;
  }
}

export function usesF64Y(kernel) {
  return kernel === "axpy" || kernel === "dot";
}

export function runF64Kernel(kernel, x, y, iterations, factor = f64Factor) {
  let sink = 0.0;

  for (let iteration = 0; iteration < iterations; iteration += 1) {
    switch (kernel) {
      case "axpy":
        sink += runAxpy(factor, x, y);
        break;
      case "dot":
        sink += runDot(x, y);
        break;
      case "sum":
        sink += runSum(x);
        break;
      case "scale":
        sink += runScale(factor, x);
        break;
    }
  }

  return sink;
}

function runAxpy(factor, x, y) {
  let checksum = 0.0;
  for (let index = 0; index < x.length; index += 1) {
    const value = factor * x[index] + y[index];
    y[index] = value;
    checksum += value;
  }
  return checksum;
}

function runDot(x, y) {
  let checksum = 0.0;
  for (let index = 0; index < x.length; index += 1) {
    checksum += x[index] * y[index];
  }
  return checksum;
}

function runSum(x) {
  let checksum = 0.0;
  for (let index = 0; index < x.length; index += 1) {
    checksum += x[index];
  }
  return checksum;
}

function runScale(factor, x) {
  let checksum = 0.0;
  for (let index = 0; index < x.length; index += 1) {
    const value = factor * x[index];
    x[index] = value;
    checksum += value;
  }
  return checksum;
}
