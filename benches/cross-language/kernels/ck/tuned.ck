// Cache-friendly CK kernels used by the cross-language benchmark suite.
// Matrices are row-major, and callers provide valid, non-overlapping buffers.

export fn matmul(a: ptr<f64>, b: ptr<f64>, out: ptr<f64>, n: i32) -> void {
  if n <= 0 {
    return;
  }

  let output_count: i32 = n * n;
  let index: i32 = 0;
  while index < output_count {
    out[index] = 0.0;
    index = index + 1;
  }

  let i: i32 = 0;
  while i < n {
    let a_row: i32 = i * n;
    let k: i32 = 0;
    while k < n {
      let a_value: f64 = a[a_row + k];
      let b_row: i32 = k * n;
      let out_row: i32 = i * n;
      let j: i32 = 0;
      while j < n {
        let out_index: i32 = out_row + j;
        out[out_index] = out[out_index] + a_value * b[b_row + j];
        j = j + 1;
      }
      k = k + 1;
    }
    i = i + 1;
  }
}

// Apply a 3x3 Gaussian kernel and divide by 16. Border cells are zero.
export fn convolve(input: ptr<f64>, out: ptr<f64>, n: i32) -> void {
  if n <= 0 {
    return;
  }

  let output_count: i32 = n * n;
  let index: i32 = 0;
  while index < output_count {
    out[index] = 0.0;
    index = index + 1;
  }

  let y: i32 = 1;
  while y < n - 1 {
    let top_row: i32 = (y - 1) * n;
    let middle_row: i32 = y * n;
    let bottom_row: i32 = (y + 1) * n;
    let x: i32 = 1;
    while x < n - 1 {
      let top_left: f64 = input[top_row + x - 1];
      let top_center: f64 = input[top_row + x];
      let top_right: f64 = input[top_row + x + 1];
      let middle_left: f64 = input[middle_row + x - 1];
      let middle_center: f64 = input[middle_row + x];
      let middle_right: f64 = input[middle_row + x + 1];
      let bottom_left: f64 = input[bottom_row + x - 1];
      let bottom_center: f64 = input[bottom_row + x];
      let bottom_right: f64 = input[bottom_row + x + 1];

      // Keep the ordinary kernel's row-major, ascending accumulation order.
      let sum: f64 = 0.0;
      sum = sum + 1.0 * top_left;
      sum = sum + 2.0 * top_center;
      sum = sum + top_right;
      sum = sum + 2.0 * middle_left;
      sum = sum + 4.0 * middle_center;
      sum = sum + 2.0 * middle_right;
      sum = sum + bottom_left;
      sum = sum + 2.0 * bottom_center;
      sum = sum + bottom_right;

      out[middle_row + x] = sum / 16.0;
      x = x + 1;
    }
    y = y + 1;
  }
}
