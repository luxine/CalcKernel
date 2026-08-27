struct Small {
  a: i64;
  b: i64;
}

struct Big {
  a: i64;
  b: i64;
  c: i64;
}

export fn echo_bool(value: bool) -> bool {
  return value;
}

export fn echo_small(value: Small) -> Small {
  return value;
}

export fn echo_big(value: Big) -> Big {
  return value;
}

export fn pointer_value(value: ptr<i64>) -> i64 {
  return value[0];
}

export fn slice_count(items: slice<i32>) -> u32 {
  return items.len;
}
