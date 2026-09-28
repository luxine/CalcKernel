fn pricing_total(price: i64, quantity: i64, discount: i64, tax_rate_ppm: i64) -> i64 {
  let subtotal: i64 = price * quantity;
  let after_discount: i64 = subtotal - discount;
  let tax: i64 = after_discount * tax_rate_ppm / 1000000;
  return after_discount + tax;
}

export fn pricing_one(price: i64, quantity: i64, discount: i64, tax_rate_ppm: i64) -> i64 {
  return pricing_total(price, quantity, discount, tax_rate_ppm);
}

export fn pricing_batch(
  prices: ptr<i64>,
  quantities: ptr<i64>,
  discounts: ptr<i64>,
  tax_rates_ppm: ptr<i64>,
  out_totals: ptr<i64>,
  n: i32
) -> i32 {
  let i: i32 = 0;

  while i < n {
    out_totals[i] = pricing_total(
      prices[i],
      quantities[i],
      discounts[i],
      tax_rates_ppm[i]
    );
    i = i + 1;
  }

  return 0;
}
