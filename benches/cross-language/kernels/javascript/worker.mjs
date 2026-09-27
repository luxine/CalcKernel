import { createHash } from 'node:crypto';
import { createInterface } from 'node:readline';

const weights = [1, 2, 1, 2, 4, 2, 1, 2, 1];
let activeCase = null;
let size = 0;
let left = null;
let right = null;
let output = null;

function matmulOrdinary(a, b, out, n) {
  for (let row = 0; row < n; row += 1) {
    const rowOffset = row * n;
    for (let col = 0; col < n; col += 1) {
      let sum = 0;
      for (let k = 0; k < n; k += 1) {
        sum += a[rowOffset + k] * b[k * n + col];
      }
      out[rowOffset + col] = sum;
    }
  }
}

function matmulTuned(a, b, out, n) {
  out.fill(0);
  for (let row = 0; row < n; row += 1) {
    const rowOffset = row * n;
    for (let k = 0; k < n; k += 1) {
      const aValue = a[rowOffset + k];
      const bRowOffset = k * n;
      for (let col = 0; col < n; col += 1) {
        const index = rowOffset + col;
        out[index] += aValue * b[bRowOffset + col];
      }
    }
  }
}

function convolveOrdinary(input, out, n) {
  for (let row = 0; row < n; row += 1) {
    for (let col = 0; col < n; col += 1) {
      const outputIndex = row * n + col;
      if (row === 0 || col === 0 || row === n - 1 || col === n - 1) {
        out[outputIndex] = 0;
        continue;
      }
      let sum = 0;
      let weightIndex = 0;
      for (let rowDelta = -1; rowDelta <= 1; rowDelta += 1) {
        const sourceRow = row + rowDelta;
        for (let colDelta = -1; colDelta <= 1; colDelta += 1) {
          const sourceCol = col + colDelta;
          sum += input[sourceRow * n + sourceCol] * weights[weightIndex];
          weightIndex += 1;
        }
      }
      out[outputIndex] = sum / 16;
    }
  }
}

function convolveTuned(input, out, n) {
  out.fill(0);
  if (n < 3) return;

  for (let row = 1; row < n - 1; row += 1) {
    const top = (row - 1) * n;
    const middle = row * n;
    const bottom = (row + 1) * n;
    for (let col = 1; col < n - 1; col += 1) {
      const leftCol = col - 1;
      const rightCol = col + 1;
      let sum = input[top + leftCol] + 2 * input[top + col] + input[top + rightCol];
      sum += 2 * input[middle + leftCol];
      sum += 4 * input[middle + col];
      sum += 2 * input[middle + rightCol];
      sum += input[bottom + leftCol];
      sum += 2 * input[bottom + col];
      sum += input[bottom + rightCol];
      out[middle + col] = sum / 16;
    }
  }
}

const kernels = {
  matmul: {
    ordinary: matmulOrdinary,
    tuned: matmulTuned,
  },
  convolve: {
    ordinary: (input, _unused, out, n) => convolveOrdinary(input, out, n),
    tuned: (input, _unused, out, n) => convolveTuned(input, out, n),
  },
};

function createInputs(caseName, n) {
  const a = new Float64Array(n * n);
  const b = caseName === 'matmul' ? new Float64Array(n * n) : null;
  for (let row = 0; row < n; row += 1) {
    for (let col = 0; col < n; col += 1) {
      const index = row * n + col;
      if (caseName === 'matmul') {
        a[index] = (((row * 17 + col * 13) % 31) - 15) / 32;
        b[index] = (((row * 11 + col * 7) % 29) - 14) / 32;
      } else {
        a[index] = (((index * 17) % 37) - 18) / 32;
      }
    }
  }
  return [a, b];
}

function digestOutput() {
  return createHash('sha256')
    .update(Buffer.from(output.buffer, output.byteOffset, output.byteLength))
    .digest('hex');
}

function handle(command) {
  if (command?.cmd === 'init') {
    const caseName = command.case;
    const n = command.size;
    if (!(caseName in kernels)) throw new Error('case must be matmul or convolve');
    if (!Number.isSafeInteger(n) || n < 1 || (caseName === 'convolve' && n < 3)) {
      throw new Error('size must be a positive integer (at least 3 for convolve)');
    }
    activeCase = caseName;
    size = n;
    [left, right] = createInputs(caseName, n);
    output = new Float64Array(n * n);

    // Run each variant before timing so V8 can optimize its hot numeric loop.
    for (const kernel of Object.values(kernels[caseName])) {
      kernel(left, right ?? output, output, size);
      kernel(left, right ?? output, output, size);
    }
    return { ready: true, case: activeCase, size };
  }

  if (command?.cmd === 'run') {
    if (!activeCase) throw new Error('send init before run');
    const mode = command.mode;
    const repeat = command.repeat;
    const kernel = kernels[activeCase][mode];
    if (!kernel) throw new Error('mode must be ordinary or tuned');
    if (!Number.isSafeInteger(repeat) || repeat < 1) throw new Error('repeat must be an integer >= 1');

    const started = process.hrtime.bigint();
    for (let i = 0; i < repeat; i += 1) {
      kernel(left, right ?? output, output, size);
    }
    const durationNs = Number(process.hrtime.bigint() - started);
    return { durationNs, sha256: digestOutput() };
  }

  throw new Error('cmd must be init or run');
}

const input = createInterface({ input: process.stdin, crlfDelay: Infinity });
input.on('line', (line) => {
  try {
    const command = JSON.parse(line);
    process.stdout.write(`${JSON.stringify(handle(command))}\n`);
  } catch (error) {
    process.stdout.write(`${JSON.stringify({ error: error instanceof Error ? error.message : String(error) })}\n`);
  }
});
