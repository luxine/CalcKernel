export fn kernel(n: i64, seed: i64) -> i64 {
  let i: i64 = 0;
  let acc: i64 = seed;
  while i < n {
    acc = (acc + i + 17) % 1000003;
    i = i + 1;
  }
  return acc;
}
