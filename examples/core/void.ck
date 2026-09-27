fn clear(out: ptr<i32>, len: u32) -> void {
  let i: u32 = 0;
  while i < len {
    out[i] = 0;
    i = i + 1;
  }
}

export fn maybe_clear(run: bool, out: ptr<i32>, len: u32) -> void {
  if !run {
    return;
  }

  clear(out, len);
}
