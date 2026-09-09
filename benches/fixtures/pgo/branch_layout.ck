fn add_path(acc: u64, value: u64) -> u64 {
  return acc * 3 + value;
}

fn subtract_path(acc: u64, value: u64) -> u64 {
  let next: u64 = acc * 5;
  next = next - value;
  next = next * 7;
  next = next + value;
  next = next * 3;
  return next + 11;
}

export unsafe fn kernel(items: slice<u64>, n: u32, seed: u64) -> u64
contract { requires n <= items.len; effects read(items); }
{
  let i: u32 = 0;
  let result: u64 = seed;
  while i < n {
    let value: u64 = items[i];
    if value == 3 {
      result = add_path(result, value);
    } else {
      result = subtract_path(result, value);
    }
    i = i + 1;
  }
  return result;
}
