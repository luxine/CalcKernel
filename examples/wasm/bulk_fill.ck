export unsafe fn fill_u32(dst: ptr<u32>, start: u32, end: u32) -> void contract {
  requires start <= end;
} {
  let i: u32 = start;
  while i < end {
    dst[i] = 1515870810;
    i = i + 1;
  }
}
