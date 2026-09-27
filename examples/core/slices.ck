struct Item {
  value: i32;
}

struct Bucket {
  items: slice<Item>;
}

fn forward(items: slice<Item>) -> slice<Item> {
  return items;
}

fn select_range(items: slice<Item>, start: u32, end: u32) -> slice<Item> {
  return items[start..end];
}

export fn slice_sum(data: ptr<Item>, len: u32, start: u32, end: u32) -> i32 {
  let items: slice<Item> = slice(data, len);
  let forwarded: slice<Item> = forward(items);
  let selected: slice<Item> = select_range(forwarded, start, end);
  return selected[0].value + selected[1].value;
}

export fn slice_len(items: slice<Item>) -> u32 {
  return items.len;
}

export fn slice_data(items: slice<Item>) -> ptr<Item> {
  return items.data;
}

export fn remember(bucket: ptr<Bucket>, items: slice<Item>) -> void {
  bucket[0].items = items;
}
