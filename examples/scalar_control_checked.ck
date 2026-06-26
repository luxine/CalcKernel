export fn sum_to_n(n: i64) -> i64 {
  let i: i64 = 0;
  let sum: i64 = 0;

  while i < n {
    sum = sum + i;
    i = i + 1;
  }

  return sum;
}

export fn choose(a: i64, b: i64) -> i64 {
  if a > b {
    return a;
  } else {
    return b;
  }
}

export fn condition_overflow(a: i64, b: i64) -> i64 {
  if a + b > 0 {
    return 1;
  }
  return 0;
}
