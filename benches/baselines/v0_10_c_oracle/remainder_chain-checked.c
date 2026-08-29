#include <stdbool.h>
#include <stdint.h>
#include <stddef.h>

typedef int32_t CK_Status;

#define CK_OK ((CK_Status)0)
#define CK_ERR_OVERFLOW ((CK_Status)1)
#define CK_ERR_DIV_BY_ZERO ((CK_Status)2)
#define CK_ERR_NULL_POINTER ((CK_Status)3)
#define CK_ERR_OUT_OF_BOUNDS ((CK_Status)4)

CK_Status kernel(int64_t n, int64_t seed, int64_t* ck_return);

CK_Status kernel(int64_t n, int64_t seed, int64_t* ck_return) {
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

  if (ck_return == NULL) {
    return CK_ERR_NULL_POINTER;
  }

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
  if (__builtin_add_overflow(acc, i, &ik_tmp2)) {
    return CK_ERR_OVERFLOW;
  }
  ik_tmp3 = 17;
  if (__builtin_add_overflow(ik_tmp2, ik_tmp3, &ik_tmp4)) {
    return CK_ERR_OVERFLOW;
  }
  ik_tmp5 = 1000003;
  if (ik_tmp5 == 0) {
    return CK_ERR_DIV_BY_ZERO;
  }
  if (ik_tmp4 == INT64_MIN && ik_tmp5 == -1) {
    return CK_ERR_OVERFLOW;
  }
  ik_tmp6 = ik_tmp4 % ik_tmp5;
  acc = ik_tmp6;
  ik_tmp7 = 1;
  if (__builtin_add_overflow(i, ik_tmp7, &ik_tmp8)) {
    return CK_ERR_OVERFLOW;
  }
  i = ik_tmp8;
  goto bb1;

bb3:
  *ck_return = acc;
  return CK_OK;
}
