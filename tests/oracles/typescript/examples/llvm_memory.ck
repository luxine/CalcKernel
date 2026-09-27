struct Item {
  price: i64;
  qty: i64;
  discount: i64;
  tax_rate_ppm: i64;
}

export fn first_price(items: ptr<Item>) -> i64 {
  return items[0].price;
}

export fn get_price(items: ptr<Item>, i: i32) -> i64 {
  return items[i].price;
}

export fn write_i64(out: ptr<i64>, value: i64) -> i32 {
  out[0] = value;
  return 0;
}
