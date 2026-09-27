export fn kernel(a: slice<u32>, out: slice<u32>, n: u32) -> void {
  let i: u32 = 0;
  while i < n { out[i] = a[i] + 11; i = i + 1; }
}
