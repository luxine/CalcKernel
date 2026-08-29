export fn kernel(items: slice<i64>, seed: i64) -> i64 {
  let i: u32 = 0;
  let result: i64 = seed;
  while i < items.len {
    let value: i64 = items[i];
    if value > result {
      result = value;
    }
    i = i + 1;
  }
  return result;
}
