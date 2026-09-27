export fn alias_map(a: slice<u32>, b: slice<u32>, n: u32) -> void {
  let i: u32 = 0;
  while i < n {
    b[i] = a[i] + 1;
    i = i + 1;
  }
}
