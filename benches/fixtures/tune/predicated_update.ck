export unsafe fn floyd(distance: slice<f64>, n: u32) -> void
contract {
  requires n <= 65535;
  effects readwrite(distance);
}
{
  let k: u32 = 0;
  while k < n {
    let k_row: u32 = k * n;
    let i: u32 = 0;
    while i < n {
      let i_row: u32 = i * n;
      let dik: f64 = distance[i_row + k];
      let j: u32 = 0;
      while j < n {
        let index: u32 = i_row + j;
        let candidate: f64 = dik + distance[k_row + j];
        let old: f64 = distance[index];
        if candidate < old {
          distance[index] = candidate;
        }
        j = j + 1;
      }
      i = i + 1;
    }
    k = k + 1;
  }
}
