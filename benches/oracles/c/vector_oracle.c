#include <stdint.h>

#ifndef ORACLE_CASE
#error "ORACLE_CASE must select one pinned kernel"
#endif

typedef uint32_t u32x4 __attribute__((vector_size(16)));
typedef double f64x2 __attribute__((vector_size(16)));

static inline u32x4 load_u32x4(const uint32_t *source) {
  u32x4 value;
  __builtin_memcpy(&value, source, sizeof(value));
  return value;
}

static inline void store_u32x4(uint32_t *target, u32x4 value) {
  __builtin_memcpy(target, &value, sizeof(value));
}

static inline f64x2 load_f64x2(const double *source) {
  f64x2 value;
  __builtin_memcpy(&value, source, sizeof(value));
  return value;
}

static inline void store_f64x2(double *target, f64x2 value) {
  __builtin_memcpy(target, &value, sizeof(value));
}

#if ORACLE_CHECKED
#define VOID_RESULT int32_t
#define RETURN_VOID return 0
#else
#define VOID_RESULT void
#define RETURN_VOID return
#endif

#if ORACLE_CHECKED
static int32_t checked_add_map(const uint32_t *a, uint32_t *out,
                               uint32_t n, uint32_t add) {
  for (uint32_t i = 0; i < n; ++i) {
    if (__builtin_add_overflow(a[i], add, out + i)) return 1;
  }
  return 0;
}
#endif

#if ORACLE_CASE == 1
VOID_RESULT ck_oracle_kernel(const uint32_t *restrict a, uint32_t a_len,
                             uint32_t *restrict out, uint32_t out_len,
                             uint32_t n) {
  (void)a_len; (void)out_len;
#if ORACLE_CHECKED
  return checked_add_map(a, out, n, 7);
#else
  uint32_t i = 0;
  const u32x4 add = {7, 7, 7, 7};
  for (; i + 4 <= n; i += 4) store_u32x4(out + i, load_u32x4(a + i) + add);
  for (; i < n; ++i) out[i] = a[i] + 7;
  RETURN_VOID;
#endif
}
#elif ORACLE_CASE == 2
VOID_RESULT ck_oracle_kernel(const uint32_t *restrict a, uint32_t a_len,
                             const uint32_t *restrict b, uint32_t b_len,
                             uint32_t *restrict out, uint32_t out_len,
                             uint32_t n) {
  (void)a_len; (void)b_len; (void)out_len;
#if ORACLE_CHECKED
  for (uint32_t i = 0; i < n; ++i) {
    if (__builtin_add_overflow(a[i], b[i], out + i)) return 1;
  }
  return 0;
#else
  uint32_t i = 0;
  for (; i + 4 <= n; i += 4)
    store_u32x4(out + i, load_u32x4(a + i) + load_u32x4(b + i));
  for (; i < n; ++i) out[i] = a[i] + b[i];
  RETURN_VOID;
#endif
}
#elif ORACLE_CASE == 3
VOID_RESULT ck_oracle_kernel(const double *restrict a, uint32_t a_len,
                             double *restrict out, uint32_t out_len,
                             uint32_t n, double factor) {
  (void)a_len; (void)out_len;
  uint32_t i = 0;
  const f64x2 factors = {factor, factor};
  for (; i + 2 <= n; i += 2) store_f64x2(out + i, load_f64x2(a + i) * factors);
  for (; i < n; ++i) out[i] = a[i] * factor;
  RETURN_VOID;
}
#elif ORACLE_CASE == 4
VOID_RESULT ck_oracle_kernel(const uint32_t *restrict a, uint32_t a_len,
                             double *restrict out, uint32_t out_len,
                             uint32_t n) {
  (void)a_len; (void)out_len;
  uint32_t i = 0;
  for (; i + 2 <= n; i += 2) {
    typedef uint32_t u32x2 __attribute__((vector_size(8)));
    u32x2 source;
    __builtin_memcpy(&source, a + i, sizeof(source));
    f64x2 converted = __builtin_convertvector(source, f64x2);
    store_f64x2(out + i, converted);
  }
  for (; i < n; ++i) out[i] = (double)a[i];
  RETURN_VOID;
}
#elif ORACLE_CASE == 5
static uint32_t reduce(const uint32_t *a, uint32_t n) {
  uint32_t i = 0;
  u32x4 lanes = {0, 0, 0, 0};
  for (; i + 4 <= n; i += 4) lanes += load_u32x4(a + i);
  uint32_t total = lanes[0] + lanes[1] + lanes[2] + lanes[3];
  for (; i < n; ++i) total += a[i];
  return total;
}
#if ORACLE_CHECKED
int32_t ck_oracle_kernel(const uint32_t *a, uint32_t a_len, uint32_t n,
                         uint32_t *result) {
  uint32_t total = 0;
  for (uint32_t i = 0; i < n; ++i) {
    if (i >= a_len || __builtin_add_overflow(total, a[i], &total)) return 1;
  }
  *result = total; return 0;
}
#else
uint32_t ck_oracle_kernel(const uint32_t *a, uint32_t a_len, uint32_t n) {
  (void)a_len; return reduce(a, n);
}
#endif
#elif ORACLE_CASE == 6
VOID_RESULT ck_oracle_kernel(const uint32_t *restrict a, uint32_t a_len,
                             const uint32_t *restrict b, uint32_t b_len,
                             uint32_t *restrict out, uint32_t out_len) {
#if ORACLE_CHECKED
  if (a_len < 4 || b_len < 4 || out_len < 4) return 1;
  for (uint32_t i = 0; i < 4; ++i) {
    if (__builtin_add_overflow(a[i], b[i], out + i)) return 1;
  }
  RETURN_VOID;
#else
  (void)a_len; (void)b_len; (void)out_len;
  store_u32x4(out, load_u32x4(a) + load_u32x4(b));
  RETURN_VOID;
#endif
}
#elif ORACLE_CASE == 7
VOID_RESULT ck_oracle_kernel(const uint32_t *a, uint32_t a_len,
                             uint32_t *out, uint32_t out_len, uint32_t n) {
#if ORACLE_CHECKED
  for (uint32_t i = 0; i < n; ++i) {
    uint32_t value;
    if (i >= a_len || __builtin_add_overflow(a[i], 11u, &value)) return 1;
    if (i >= out_len) return 1;
    out[i] = value;
  }
  return 0;
#else
  (void)a_len; (void)out_len;
  uintptr_t a0 = (uintptr_t)a, a1 = a0 + (uintptr_t)n * 4u;
  uintptr_t b0 = (uintptr_t)out, b1 = b0 + (uintptr_t)n * 4u;
  if (a1 <= b0 || b1 <= a0) {
    uint32_t i = 0;
    const u32x4 add = {11, 11, 11, 11};
    for (; i + 4 <= n; i += 4)
      store_u32x4(out + i, load_u32x4(a + i) + add);
    for (; i < n; ++i) out[i] = a[i] + 11;
  } else {
    for (uint32_t i = 0; i < n; ++i) out[i] = a[i] + 11;
  }
  RETURN_VOID;
#endif
}
#elif ORACLE_CASE == 8
VOID_RESULT ck_oracle_kernel(const uint32_t *restrict a, uint32_t a_len,
                             uint32_t *restrict out, uint32_t out_len) {
  (void)a_len; (void)out_len;
#if ORACLE_CHECKED
  return checked_add_map(a, out, 4000, 13);
#else
  const u32x4 add = {13, 13, 13, 13};
  for (uint32_t i = 0; i < 4000; i += 4) {
    store_u32x4(out + i, load_u32x4(a + i) + add);
  }
  RETURN_VOID;
#endif
}
#elif ORACLE_CASE == 9
VOID_RESULT ck_oracle_kernel(const uint32_t *a, uint32_t a_len,
                             uint32_t *out, uint32_t out_len, uint32_t n) {
  (void)a_len; (void)out_len;
#if ORACLE_CHECKED
  return checked_add_map(a, out, n, 17);
#else
  for (uint32_t i = 0; i < n; ++i) out[i] = a[i] + 17;
  RETURN_VOID;
#endif
}
#elif ORACLE_CASE == 10
VOID_RESULT ck_oracle_kernel(const uint32_t *a, uint32_t a_len,
                             uint32_t *out, uint32_t out_len, uint32_t n) {
  (void)a_len; (void)out_len;
#if ORACLE_CHECKED
  return checked_add_map(a, out, n, 17);
#else
  for (uint32_t i = 0; i < n; ++i) out[i] = a[i] + 17;
  RETURN_VOID;
#endif
}
#else
#error "unsupported ORACLE_CASE"
#endif
