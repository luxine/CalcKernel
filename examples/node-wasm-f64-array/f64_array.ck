export fn axpy_f64(a: f64, x: ptr<f64>, y: ptr<f64>, len: i32) -> f64 {
  let i: i32 = 0;
  let checksum: f64 = 0.0;

  while i < len {
    let value: f64 = a * x[i] + y[i];
    y[i] = value;
    checksum = checksum + value;
    i = i + 1;
  }

  return checksum;
}
