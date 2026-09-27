export fn nested_control(n: i64) -> i64 {
  let outer: i64 = 0;
  let total: i64 = 0;

  while outer < n {
    let inner: i64 = 0;
    while inner < 10 {
      if inner == 7 {
        break;
      }
      if (outer + inner) % 3 == 0 {
        inner = inner + 1;
        continue;
      }
      total = total + (outer + 1) * (inner + 2);
      inner = inner + 1;
    }
    outer = outer + 1;
  }

  return total;
}
