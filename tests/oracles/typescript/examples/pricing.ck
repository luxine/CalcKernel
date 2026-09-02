struct Item {
  price: i64;
  qty: i64;
  discount: i64;
  tax_rate_ppm: i64;
}

export fn calc_items(items: ptr<Item>, len: i32, out: ptr<i64>) -> i32 {
  let i: i32 = 0;

  while i < len {
    let subtotal: i64 = items[i].price * items[i].qty;
    let after_discount: i64 = subtotal - items[i].discount;
    let tax: i64 = after_discount * items[i].tax_rate_ppm / 1000000;
    out[i] = after_discount + tax;
    i = i + 1;
  }

  return 0;
}
