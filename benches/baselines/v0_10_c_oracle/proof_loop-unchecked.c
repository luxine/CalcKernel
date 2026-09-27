#include <stdbool.h>
#include <stdint.h>

typedef struct CK_Slice_i64 {
  int64_t* data;
  uint32_t len;
} CK_Slice_i64;

int64_t kernel(int64_t* items_data, uint32_t items_len, int64_t seed);

int64_t kernel(int64_t* items_data, uint32_t items_len, int64_t seed) {
  CK_Slice_i64 items;
  uint32_t i;
  int64_t result;
  int64_t value;
  uint32_t ik_tmp0;
  uint32_t ik_tmp1;
  bool ik_tmp2;
  int64_t ik_tmp3;
  bool ik_tmp4;
  uint32_t ik_tmp5;
  uint32_t ik_tmp6;

  items.data = items_data;
  items.len = items_len;

  ik_tmp0 = 0;
  i = ik_tmp0;
  result = seed;
  goto bb1;

bb1:
  ik_tmp1 = items.len;
  ik_tmp2 = i < ik_tmp1;
  if (ik_tmp2) {
    goto bb2;
  } else {
    goto bb3;
  }

bb2:
  ik_tmp3 = items.data[i];
  value = ik_tmp3;
  ik_tmp4 = value > result;
  if (ik_tmp4) {
    goto bb4;
  } else {
    goto bb5;
  }

bb4:
  result = value;
  goto bb5;

bb5:
  ik_tmp5 = 1;
  ik_tmp6 = i + ik_tmp5;
  i = ik_tmp6;
  goto bb1;

bb3:
  return result;
}
