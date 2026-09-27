export unsafe fn kernel(a: slice<u32>, n: u32) -> u32
contract { requires n <= a.len; effects read(a); }
{
  let i: u32 = 0;
  let total: u32 = 0;
  while i < n { total = total + a[i]; i = i + 1; }
  return total;
}
