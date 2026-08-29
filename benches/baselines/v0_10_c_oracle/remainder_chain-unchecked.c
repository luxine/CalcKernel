#include <stdbool.h>
#include <stdint.h>

int64_t kernel(int64_t n, int64_t seed);

int64_t kernel(int64_t n, int64_t seed) {
  int64_t i;
  int64_t acc;
  int64_t ik_tmp0;
  bool ik_tmp1;
  int64_t ik_tmp2;
  int64_t ik_tmp3;
  int64_t ik_tmp4;
  int64_t ik_tmp5;
  int64_t ik_tmp6;
  int64_t ik_tmp7;
  int64_t ik_tmp8;

  ik_tmp0 = 0;
  i = ik_tmp0;
  acc = seed;
  goto bb1;

bb1:
  ik_tmp1 = i < n;
  if (ik_tmp1) {
    goto bb2;
  } else {
    goto bb3;
  }

bb2:
  ik_tmp2 = acc + i;
  ik_tmp3 = 17;
  ik_tmp4 = ik_tmp2 + ik_tmp3;
  ik_tmp5 = 1000003;
  ik_tmp6 = ik_tmp4 % ik_tmp5;
  acc = ik_tmp6;
  ik_tmp7 = 1;
  ik_tmp8 = i + ik_tmp7;
  i = ik_tmp8;
  goto bb1;

bb3:
  return acc;
}
