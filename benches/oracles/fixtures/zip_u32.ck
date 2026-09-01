export unsafe fn kernel(a: slice<u32>, b: slice<u32>, out: slice<u32>, n: u32) -> void
contract { requires n <= a.len && n <= b.len && n <= out.len; requires noalias(a, b) && noalias(a, out) && noalias(b, out); effects read(a), read(b), write(out); }
{
  let i: u32 = 0;
  while i < n { out[i] = a[i] + b[i]; i = i + 1; }
}
