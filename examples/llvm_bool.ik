export fn not_bool(a: bool) -> bool {
  return !a;
}

export fn bool_local(a: bool) -> bool {
  let x: bool = !a;
  return x;
}

export fn choose_bool(a: bool, x: i32, y: i32) -> i32 {
  if a {
    return x;
  }
  return y;
}
