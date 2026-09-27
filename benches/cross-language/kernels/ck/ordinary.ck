// Reference CK kernels used by the cross-language benchmark suite.
// Matrices are row-major, and callers provide valid, non-overlapping buffers.

export fn matmul(a: ptr<f64>, b: ptr<f64>, out: ptr<f64>, n: i32) -> void {
  let i: i32 = 0;
  while i < n {
    let j: i32 = 0;
    while j < n {
      let sum: f64 = 0.0;
      let k: i32 = 0;
      while k < n {
        let a_index: i32 = i * n + k;
        let b_index: i32 = k * n + j;
        sum = sum + a[a_index] * b[b_index];
        k = k + 1;
      }

      let out_index: i32 = i * n + j;
      out[out_index] = sum;
      j = j + 1;
    }
    i = i + 1;
  }
}

// Apply a 3x3 Gaussian kernel and divide by 16. Border cells are zero.
export fn convolve(input: ptr<f64>, out: ptr<f64>, n: i32) -> void {
  if n <= 0 {
    return;
  }

  let y: i32 = 0;
  while y < n {
    let x: i32 = 0;
    while x < n {
      let out_index: i32 = y * n + x;

      if y == 0 || x == 0 || y == n - 1 || x == n - 1 {
        out[out_index] = 0.0;
      } else {
        let sum: f64 = 0.0;
        sum = sum + 1.0 * input[(y - 1) * n + x - 1];
        sum = sum + 2.0 * input[(y - 1) * n + x];
        sum = sum + input[(y - 1) * n + x + 1];
        sum = sum + 2.0 * input[y * n + x - 1];
        sum = sum + 4.0 * input[y * n + x];
        sum = sum + 2.0 * input[y * n + x + 1];
        sum = sum + input[(y + 1) * n + x - 1];
        sum = sum + 2.0 * input[(y + 1) * n + x];
        sum = sum + input[(y + 1) * n + x + 1];
        out[out_index] = sum / 16.0;
      }

      x = x + 1;
    }
    y = y + 1;
  }
}
