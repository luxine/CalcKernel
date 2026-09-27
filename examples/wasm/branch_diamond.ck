export fn branch_diamond(n: i64) -> i64 {
  let i: i64 = 0;
  let total: i64 = 0;

  while i < n {
    if i % 2 == 0 {
      total = total + i + 7;
    } else {
      total = total + 2 * i + 5;
    }
    if i % 3 == 0 {
      total = total + 13;
    } else {
      total = total + 17;
    }
    i = i + 1;
  }

  return total;
}
