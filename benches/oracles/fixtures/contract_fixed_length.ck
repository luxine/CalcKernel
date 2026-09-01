export unsafe fn kernel(a: slice<u32>, out: slice<u32>, n: u32) -> void
contract { requires n == 16 && n <= a.len && n <= out.len; requires noalias(a, out); effects read(a), write(out); }
{
  let i: u32 = 0;
  while i < n { out[i] = a[i] + 17; i = i + 1; }
}
