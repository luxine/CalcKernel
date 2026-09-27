export fn sum_u32(a: slice<u32>, n: u32, initial: u32) -> u32 {
  let i: u32 = 0;
  let total: u32 = initial;
  while i < n {
    total = total + a[i];
    i = i + 1;
  }
  return total;
}

export fn product_u32(a: slice<u32>, n: u32, initial: u32) -> u32 {
  let i: u32 = 0;
  let total: u32 = initial;
  while i < n {
    total = total * a[i];
    i = i + 1;
  }
  return total;
}

export fn sum_i32(a: slice<i32>, n: u32, initial: i32) -> i32 {
  let i: u32 = 0;
  let total: i32 = initial;
  while i < n {
    total = total + a[i];
    i = i + 1;
  }
  return total;
}

export fn product_i32(a: slice<i32>, n: u32, initial: i32) -> i32 {
  let i: u32 = 0;
  let total: i32 = initial;
  while i < n {
    total = total * a[i];
    i = i + 1;
  }
  return total;
}
