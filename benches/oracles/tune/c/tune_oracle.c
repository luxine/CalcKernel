#include <stdint.h>

#ifndef CK_TUNE_ORACLE_CASE
#error "CK_TUNE_ORACLE_CASE is required"
#endif

#if defined(CK_TUNE_GENERIC)
#define CK_RESTRICT
#define CK_VECTOR_HINT
#else
#define CK_RESTRICT restrict
#define CK_VECTOR_HINT _Pragma("clang loop vectorize(enable) interleave(enable)")
#endif

#if CK_TUNE_ORACLE_CASE == 1
static uint64_t add_path(uint64_t acc, uint64_t value) {
  return acc * UINT64_C(3) + value;
}
static uint64_t subtract_path(uint64_t acc, uint64_t value) {
  uint64_t next = acc * UINT64_C(5);
  next -= value;
  next *= UINT64_C(7);
  next += value;
  next *= UINT64_C(3);
  return next + UINT64_C(11);
}
uint64_t kernel(const uint64_t *items, uint32_t items_len, uint32_t n,
                uint64_t seed) {
  (void)items_len;
  uint64_t result = seed;
  for (uint32_t i = 0; i < n; ++i) {
    uint64_t value = items[i];
    result = value == UINT64_C(3) ? add_path(result, value)
                                  : subtract_path(result, value);
  }
  return result;
}
#elif CK_TUNE_ORACLE_CASE == 2
static uint32_t hot_step(uint32_t acc, uint32_t value) {
  return acc * UINT32_C(3) + value;
}
static uint32_t cold_step(uint32_t acc, uint32_t value) {
  uint32_t next = acc * UINT32_C(5);
  next -= value;
  next *= UINT32_C(7);
  next += value;
  next *= UINT32_C(3);
  return next + UINT32_C(11);
}
void kernel(const uint32_t *CK_RESTRICT a, uint32_t a_len,
            uint32_t *CK_RESTRICT out, uint32_t out_len) {
  (void)a_len;
  (void)out_len;
  uint32_t acc = 0;
  for (uint32_t i = 0; i < UINT32_C(4000); ++i) {
    uint32_t value = a[i];
    acc = value == UINT32_C(13) ? hot_step(acc, value)
                                : cold_step(acc, value);
    out[i] = acc;
  }
}
#elif CK_TUNE_ORACLE_CASE == 3
void kernel(const uint32_t *CK_RESTRICT a, uint32_t a_len,
            uint32_t *CK_RESTRICT out, uint32_t out_len, uint32_t n) {
  (void)a_len;
  (void)out_len;
  CK_VECTOR_HINT
  for (uint32_t i = 0; i < n; ++i) out[i] = a[i] + UINT32_C(7);
}
#elif CK_TUNE_ORACLE_CASE == 4
void kernel(const uint32_t *CK_RESTRICT a, uint32_t a_len,
            const uint32_t *CK_RESTRICT b, uint32_t b_len,
            uint32_t *CK_RESTRICT out, uint32_t out_len, uint32_t n) {
  (void)a_len;
  (void)b_len;
  (void)out_len;
  CK_VECTOR_HINT
  for (uint32_t i = 0; i < n; ++i) out[i] = a[i] + b[i];
}
#elif CK_TUNE_ORACLE_CASE == 5
void kernel(const double *CK_RESTRICT a, uint32_t a_len,
            double *CK_RESTRICT out, uint32_t out_len, uint32_t n,
            double factor) {
  (void)a_len;
  (void)out_len;
  CK_VECTOR_HINT
  for (uint32_t i = 0; i < n; ++i) {
    double value = a[i];
    double x = value * factor;
    x += value;
    x *= factor;
    x -= value;
    x *= x;
    x += factor;
    x *= factor;
    x -= value;
    x *= x;
    x += value;
    x *= factor;
    x -= value;
    out[i] = x;
  }
}
#elif CK_TUNE_ORACLE_CASE == 6 || CK_TUNE_ORACLE_CASE == 7
void kernel(const uint32_t *CK_RESTRICT a, uint32_t a_len,
            uint32_t *CK_RESTRICT out, uint32_t out_len, uint32_t n) {
  (void)a_len;
  (void)out_len;
  CK_VECTOR_HINT
  for (uint32_t i = 0; i < n; ++i) out[i] = a[i] + UINT32_C(17);
}
#else
#error "unsupported CK_TUNE_ORACLE_CASE"
#endif
