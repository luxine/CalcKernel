export fn max_i32(a: i32, b: i32) -> i32 {
  if a > b {
    return a;
  } else {
    return b;
  }
}

export fn sum_to_n(n: i64) -> i64 {
  let i: i64 = 0;
  let sum: i64 = 0;

  while i < n {
    sum = sum + i;
    i = i + 1;
  }

  return sum;
}
