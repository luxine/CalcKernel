fn add_i64(a: i64, b: i64) -> i64 {
  return a + b;
}

fn double_i64(a: i64) -> i64 {
  return a * 2;
}

export fn calc(a: i64, b: i64) -> i64 {
  return double_i64(add_i64(a, b));
}
