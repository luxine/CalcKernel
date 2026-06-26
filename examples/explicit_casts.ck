export fn avg_i32(sum: i32, count: i32) -> f64 {
  return i32_to_f64(sum) / i32_to_f64(count);
}

export fn ratio_u32(a: u32, b: u32) -> f64 {
  return u32_to_f64(a) / u32_to_f64(b);
}
