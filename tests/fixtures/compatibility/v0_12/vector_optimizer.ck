export unsafe fn map_u32(a: slice<u32>, b: slice<u32>, n: u32) -> void
contract {
  requires noalias(a, b);
  effects read(a), write(b);
}
{
  let i: u32 = 0;
  while i < n {
    b[i] = a[i] + 7;
    i = i + 1;
  }
}

export fn sum_u32(a: slice<u32>, n: u32) -> u32 {
  let i: u32 = 0;
  let total: u32 = 0;
  while i < n {
    total = total + a[i];
    i = i + 1;
  }
  return total;
}
