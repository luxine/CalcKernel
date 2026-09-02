export fn sum_f64(values: ptr<f64>, len: i32) -> f64 {
  let i: i32 = 0;
  let total: f64 = 0.0;

  while i < len {
    total = total + values[i];
    i = i + 1;
  }

  return total;
}
