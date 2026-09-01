export unsafe fn kernel(a: slice<u32>, out: slice<f64>, n: u32) -> void
contract { requires n <= a.len && n <= out.len; requires noalias(a, out); effects read(a), write(out); }
{
  let i: u32 = 0;
  while i < n { out[i] = u32_to_f64(a[i]); i = i + 1; }
}
