struct Quote {
  price: f64;
  tax: f64;
}

struct NestedQuote {
  quote: Quote;
  fee: f64;
}

export fn finite_add() -> f64 {
  return 1.25 + 2.75;
}

export fn finite_sub() -> f64 {
  return 5.5 - 2.0;
}

export fn finite_mul() -> f64 {
  return 1.5 * 2.5;
}

export fn finite_div() -> f64 {
  return 7.0 / 2.0;
}

export fn tolerance_calc() -> f64 {
  return (10.0 / 3.0) * 3.0;
}

export fn finite_less() -> bool {
  return 1.25 < 2.75;
}

export fn finite_less_equal() -> bool {
  return 2.75 <= 2.75;
}

export fn finite_equal() -> bool {
  return 4.0 == 4.0;
}

export fn positive_infinity() -> f64 {
  return 1.0 / 0.0;
}

export fn negative_infinity() -> f64 {
  return -1.0 / 0.0;
}

export fn not_a_number() -> f64 {
  return 0.0 / 0.0;
}

export fn negative_zero() -> f64 {
  return 0.0 * -1.0;
}

export fn zero_equals_negative_zero() -> bool {
  return 0.0 == negative_zero();
}

export fn nan_equals_nan() -> bool {
  let value: f64 = not_a_number();
  return value == value;
}

export fn nan_not_equals_nan() -> bool {
  let value: f64 = not_a_number();
  return value != value;
}

export fn nan_less_than_one() -> bool {
  return not_a_number() < 1.0;
}

export fn nan_less_equal_one() -> bool {
  return not_a_number() <= 1.0;
}

export fn nan_greater_than_one() -> bool {
  return not_a_number() > 1.0;
}

export fn nan_greater_equal_one() -> bool {
  return not_a_number() >= 1.0;
}

export fn infinity_plus_finite() -> f64 {
  return positive_infinity() + 42.0;
}

export fn infinity_minus_infinity() -> f64 {
  return positive_infinity() - positive_infinity();
}

export fn overflow_to_infinity() -> f64 {
  return 1.0e308 * 1.0e308;
}

export fn underflow_smoke() -> f64 {
  return 1.0e-308 * 1.0e-308;
}

export fn infinity_greater_than_finite() -> bool {
  return positive_infinity() > 1.0;
}

export fn negative_infinity_less_than_finite() -> bool {
  return negative_infinity() < -1.0;
}

export fn ptr_read(values: ptr<f64>, index: i32) -> f64 {
  return values[index];
}

export fn ptr_write(values: ptr<f64>, index: i32, value: f64) -> f64 {
  values[index] = value;
  return values[index];
}

export fn struct_read(quotes: ptr<Quote>, index: i32) -> f64 {
  return quotes[index].price + quotes[index].tax;
}

export fn struct_write(quotes: ptr<Quote>, index: i32, value: f64) -> f64 {
  quotes[index].tax = value;
  return quotes[index].price + quotes[index].tax;
}

export fn nested_struct_read(nested: ptr<NestedQuote>, index: i32) -> f64 {
  return nested[index].quote.price + nested[index].quote.tax + nested[index].fee;
}

export fn nested_struct_write(nested: ptr<NestedQuote>, index: i32, value: f64) -> f64 {
  nested[index].quote.tax = value;
  return nested[index].quote.price + nested[index].quote.tax + nested[index].fee;
}
