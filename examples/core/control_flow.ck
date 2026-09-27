export fn sum_until_limit(limit: u32) -> u32 {
  let i: u32 = 0;
  let total: u32 = 0;

  while i < limit {
    i = i + 1;
    if i == 2 {
      continue;
    }

    total = total + i;
    if total > 10 {
      break;
    }
  }

  return total;
}
