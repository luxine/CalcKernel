export unsafe fn map_i32(
  a: slice<i32>,
  b: slice<i32>,
  n: u32,
  factor: i32,
  bias: i32
) -> void contract {
  requires n <= a.len && n <= b.len;
  requires noalias(a, b);
  effects read(a), write(b);
} {
  let i: u32 = 0;
  while i < n {
    b[i] = a[i] * factor + bias;
    i = i + 1;
  }
}
