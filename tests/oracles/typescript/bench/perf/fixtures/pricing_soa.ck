export fn pricing_soa(
  prices: ptr<i64>,
  quantities: ptr<i64>,
  discounts: ptr<i64>,
  tax_rates_ppm: ptr<i64>,
  out_totals: ptr<i64>,
  n: i32
) -> i32 {
  let i: i32 = 0;

  while i < n {
    let subtotal: i64 = prices[i] * quantities[i];
    let after_discount: i64 = subtotal - discounts[i];
    let tax: i64 = after_discount * tax_rates_ppm[i] / 1000000;
    let total: i64 = after_discount + tax;

    out_totals[i] = total;
    i = i + 1;
  }

  return 0;
}
