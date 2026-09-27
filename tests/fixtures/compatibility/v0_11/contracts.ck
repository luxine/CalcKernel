export unsafe fn bounded(items: slice<i32>, n: u32) -> i32
contract {
  requires n < items.len;
  requires aligned(items.data, 4);
  effects read(items);
}
{
  return items[n];
}
