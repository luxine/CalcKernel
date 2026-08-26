export fn and_short_circuit(a: i64, b: i64) -> bool {
  return a != 0 && b / a > 1;
}

export fn or_short_circuit(a: i64, b: i64) -> bool {
  return a == 0 || b / a > 1;
}

export fn and_rhs_error(run_rhs: bool, numerator: i64, divisor: i64) -> bool {
  return run_rhs && numerator / divisor > 1;
}
