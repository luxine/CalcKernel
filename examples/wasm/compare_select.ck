export unsafe fn map_u32_compare_select(
  a: slice<u32>,
  b: slice<u32>,
  n: u32,
  pivot: u32
) -> void contract {
  requires n <= a.len && n <= b.len;
  requires noalias(a, b);
  effects read(a), write(b);
} {
  let i: u32 = 0;
  while i < n {
    let x: u32 = a[i];
    let selected: u32 = 0;
    if x < pivot { selected = x + 1; } else { selected = x - 1; }
    b[i] = selected;
    i = i + 1;
  }
}
