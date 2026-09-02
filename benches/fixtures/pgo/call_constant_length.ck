fn hot_step(acc: u32, value: u32) -> u32 {
  return acc * 3 + value;
}

fn cold_step(acc: u32, value: u32) -> u32 {
  let next: u32 = acc * 5;
  next = next - value;
  next = next * 7;
  next = next + value;
  next = next * 3;
  return next + 11;
}

unsafe fn fixed_map(a: slice<u32>, out: slice<u32>, n: u32) -> void
contract { requires n <= a.len && n <= out.len; requires noalias(a, out); effects read(a), write(out); }
{
  let i: u32 = 0;
  let acc: u32 = 0;
  while i < n {
    let value: u32 = a[i];
    if value == 13 {
      acc = hot_step(acc, value);
    } else {
      acc = cold_step(acc, value);
    }
    out[i] = acc;
    i = i + 1;
  }
}

export unsafe fn kernel(a: slice<u32>, out: slice<u32>) -> void
contract { requires a.len >= 4000 && out.len >= 4000; requires noalias(a, out); effects read(a), write(out); }
{
  unsafe { fixed_map(a, out, 4000); }
}
