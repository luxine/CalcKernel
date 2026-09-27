#include <stdbool.h>
#include <stdint.h>
#include <stddef.h>

typedef int32_t CK_Status;

#define CK_OK ((CK_Status)0)
#define CK_ERR_OVERFLOW ((CK_Status)1)
#define CK_ERR_DIV_BY_ZERO ((CK_Status)2)
#define CK_ERR_NULL_POINTER ((CK_Status)3)
#define CK_ERR_OUT_OF_BOUNDS ((CK_Status)4)

typedef struct CK_Slice_i64 {
  int64_t* data;
  uint32_t len;
} CK_Slice_i64;

CK_Status kernel(int64_t* items_data, uint32_t items_len, int64_t seed, int64_t* ck_return);

CK_Status kernel(int64_t* items_data, uint32_t items_len, int64_t seed, int64_t* ck_return) {
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

  if (ck_return == NULL) {
    return CK_ERR_NULL_POINTER;
  }

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
  if (i >= items.len) {
    return CK_ERR_OUT_OF_BOUNDS;
  }
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
  if (__builtin_add_overflow(i, ik_tmp5, &ik_tmp6)) {
    return CK_ERR_OVERFLOW;
  }
  i = ik_tmp6;
  goto bb1;

bb3:
  *ck_return = result;
  return CK_OK;
}
