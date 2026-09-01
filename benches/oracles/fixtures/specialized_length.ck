unsafe fn fixed_map(a: slice<u32>, out: slice<u32>, n: u32) -> void
contract { requires n <= a.len && n <= out.len; requires noalias(a, out); effects read(a), write(out); }
{
  let i: u32 = 0;
  while i < n { out[i] = a[i] + 13; i = i + 1; }
}

export unsafe fn kernel(a: slice<u32>, out: slice<u32>) -> void
contract { requires a.len >= 4000 && out.len >= 4000; requires noalias(a, out); effects read(a), write(out); }
{
  unsafe { fixed_map(a, out, 4000); }
}
