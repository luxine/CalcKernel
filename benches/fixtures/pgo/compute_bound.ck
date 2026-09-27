export unsafe fn kernel(a: slice<f64>, out: slice<f64>, n: u32, factor: f64) -> void
contract { requires n <= a.len && n <= out.len; requires noalias(a, out); effects read(a), write(out); }
{
  let i: u32 = 0;
  while i < n {
    let value: f64 = a[i];
    let x: f64 = value * factor;
    x = x + value;
    x = x * factor;
    x = x - value;
    x = x * x;
    x = x + factor;
    x = x * factor;
    x = x - value;
    x = x * x;
    x = x + value;
    x = x * factor;
    x = x - value;
    out[i] = x;
    i = i + 1;
  }
}
