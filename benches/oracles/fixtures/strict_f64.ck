export unsafe fn kernel(a: slice<f64>, out: slice<f64>, n: u32, factor: f64) -> void
contract { requires n <= a.len && n <= out.len; requires noalias(a, out); effects read(a), write(out); }
{
  let i: u32 = 0;
  while i < n { out[i] = a[i] * factor; i = i + 1; }
}
