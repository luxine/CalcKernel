export unsafe fn copy_u32(dst: ptr<u32>, src: ptr<u32>, start: u32, end: u32) -> void contract {
  requires start <= end;
} {
  let i: u32 = start;
  while i < end {
    dst[i] = src[i];
    i = i + 1;
  }
}
