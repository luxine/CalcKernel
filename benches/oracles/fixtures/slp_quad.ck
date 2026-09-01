export unsafe fn kernel(a: slice<u32>, b: slice<u32>, out: slice<u32>) -> void
contract {
  requires a.len >= 4 && b.len >= 4 && out.len >= 4;
  requires noalias(a, b) && noalias(a, out) && noalias(b, out);
  effects read(a), read(b), write(out);
}
{
  out[0] = a[0] + b[0];
  out[1] = a[1] + b[1];
  out[2] = a[2] + b[2];
  out[3] = a[3] + b[3];
}
