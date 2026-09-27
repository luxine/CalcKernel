#include "ckc_runtime.h"

/*
 * Contract affine expressions are emitted as sufficiently wide LLVM integers,
 * which is an exact overflow-safe evaluator for the closed 0.11 expression
 * language. This translation unit owns the freestanding address-range model
 * used by runtime-oriented tests and documents the checked arithmetic shared
 * by all host implementations without comparing unrelated C pointers.
 */
static bool checked_interval(uint64_t address, uint32_t length,
                             uint64_t element_size, uint64_t *end) {
  if (length != 0u && element_size > UINT64_MAX / (uint64_t)length) {
    return false;
  }
  const uint64_t bytes = element_size * (uint64_t)length;
  if (address > UINT64_MAX - bytes) {
    return false;
  }
  *end = address + bytes;
  return true;
}

bool __ck_contract_noalias_u64(uint64_t left, uint32_t left_length,
                               uint64_t left_element_size, uint64_t right,
                               uint32_t right_length,
                               uint64_t right_element_size) {
  if (left_length == 0u || right_length == 0u) {
    return true;
  }
  uint64_t left_end;
  uint64_t right_end;
  return checked_interval(left, left_length, left_element_size, &left_end) &&
         checked_interval(right, right_length, right_element_size,
                          &right_end) &&
         (left_end <= right || right_end <= left);
}
