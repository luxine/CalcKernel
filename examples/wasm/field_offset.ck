struct Slot {
  head: u32;
  value: u32;
  tail: u64;
}

export unsafe fn load_aligned(items: ptr<Slot>, index: u32) -> u32 contract {
  requires aligned(items, 16);
} {
  return items[index].value;
}

export unsafe fn load_unaligned(items: ptr<Slot>, index: u32) -> u32 contract {
  requires index >= 0;
} {
  return items[index].value;
}
