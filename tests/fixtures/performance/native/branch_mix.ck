export fn kernel(n: i64, seed: i64) -> i64 {
  let i: i64 = 0;
  let acc: i64 = seed;
  while i < n {
    if i % 3 == 0 {
      acc = acc + 7;
    } else {
      acc = acc - 3;
    }
    i = i + 1;
  }
  return acc;
}
