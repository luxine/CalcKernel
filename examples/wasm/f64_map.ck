export unsafe fn map_f64(
  a: slice<f64>,
  b: slice<f64>,
  n: u32,
  factor: f64,
  bias: f64
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
