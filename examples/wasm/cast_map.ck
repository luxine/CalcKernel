export unsafe fn map_i32_to_f64(
  a: slice<i32>,
  b: slice<f64>,
  n: u32
) -> void contract {
  requires n <= a.len && n <= b.len;
  requires noalias(a, b);
  effects read(a), write(b);
} {
  let i: u32 = 0;
  while i < n {
    b[i] = i32_to_f64(a[i]);
    i = i + 1;
  }
}

export unsafe fn map_u32_to_f64(
  a: slice<u32>,
  b: slice<f64>,
  n: u32
) -> void contract {
  requires n <= a.len && n <= b.len;
  requires noalias(a, b);
  effects read(a), write(b);
} {
  let i: u32 = 0;
  while i < n {
    b[i] = u32_to_f64(a[i]);
    i = i + 1;
  }
}
