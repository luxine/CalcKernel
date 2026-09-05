#ifndef CKC_PROFILE_ATOMIC_H
#define CKC_PROFILE_ATOMIC_H

#include <stdint.h>

#if defined(_MSC_VER)

#include <intrin.h>
#pragma intrinsic(_InterlockedCompareExchange)
#pragma intrinsic(_InterlockedCompareExchange64)
#pragma intrinsic(_InterlockedExchange)
#pragma intrinsic(_InterlockedIncrement)

typedef struct ckc_profile_atomic_u32 {
  volatile long value;
} ckc_profile_atomic_u32;

typedef struct ckc_profile_atomic_u64 {
  volatile __int64 value;
} ckc_profile_atomic_u64;

static __inline uint32_t
ckc_profile_atomic_load_acquire_u32(ckc_profile_atomic_u32 *object) {
  return (uint32_t)_InterlockedCompareExchange(&object->value, 0, 0);
}

static __inline uint32_t
ckc_profile_atomic_load_relaxed_u32(ckc_profile_atomic_u32 *object) {
  return ckc_profile_atomic_load_acquire_u32(object);
}

static __inline void
ckc_profile_atomic_store_release_u32(ckc_profile_atomic_u32 *object,
                                     uint32_t value) {
  (void)_InterlockedExchange(&object->value, (long)value);
}

static __inline void
ckc_profile_atomic_store_relaxed_u32(ckc_profile_atomic_u32 *object,
                                     uint32_t value) {
  ckc_profile_atomic_store_release_u32(object, value);
}

static __inline int ckc_profile_atomic_compare_exchange_strong_u32(
    ckc_profile_atomic_u32 *object, uint32_t *expected, uint32_t desired) {
  const long observed = _InterlockedCompareExchange(
      &object->value, (long)desired, (long)*expected);
  if ((uint32_t)observed == *expected) {
    return 1;
  }
  *expected = (uint32_t)observed;
  return 0;
}

static __inline uint64_t
ckc_profile_atomic_load_relaxed_u64(ckc_profile_atomic_u64 *object) {
  return (uint64_t)_InterlockedCompareExchange64(&object->value, 0, 0);
}

static __inline int ckc_profile_atomic_compare_exchange_weak_u64(
    ckc_profile_atomic_u64 *object, uint64_t *expected, uint64_t desired) {
  const __int64 observed = _InterlockedCompareExchange64(
      &object->value, (__int64)desired, (__int64)*expected);
  if ((uint64_t)observed == *expected) {
    return 1;
  }
  *expected = (uint64_t)observed;
  return 0;
}

#elif defined(__aarch64__) && defined(__linux__)

typedef struct ckc_profile_atomic_u32 {
  uint32_t value;
} ckc_profile_atomic_u32;

typedef struct ckc_profile_atomic_u64 {
  uint64_t value;
} ckc_profile_atomic_u64;

static inline uint32_t
ckc_profile_atomic_load_acquire_u32(ckc_profile_atomic_u32 *object) {
  uint32_t value;
  __asm__ volatile("ldar %w0, [%1]" : "=r"(value) : "r"(&object->value)
                   : "memory");
  return value;
}

static inline uint32_t
ckc_profile_atomic_load_relaxed_u32(ckc_profile_atomic_u32 *object) {
  uint32_t value;
  __asm__ volatile("ldr %w0, [%1]" : "=r"(value) : "r"(&object->value)
                   : "memory");
  return value;
}

static inline void
ckc_profile_atomic_store_release_u32(ckc_profile_atomic_u32 *object,
                                     uint32_t value) {
  __asm__ volatile("stlr %w0, [%1]" : : "r"(value), "r"(&object->value)
                   : "memory");
}

static inline void
ckc_profile_atomic_store_relaxed_u32(ckc_profile_atomic_u32 *object,
                                     uint32_t value) {
  __asm__ volatile("str %w0, [%1]" : : "r"(value), "r"(&object->value)
                   : "memory");
}

static inline int ckc_profile_atomic_compare_exchange_strong_u32(
    ckc_profile_atomic_u32 *object, uint32_t *expected, uint32_t desired) {
  const uint32_t expected_value = *expected;
  uint32_t observed;
  uint32_t status;
  __asm__ volatile(
      "0:\n"
      "ldaxr %w0, [%2]\n"
      "cmp %w0, %w3\n"
      "b.ne 1f\n"
      "stlxr %w1, %w4, [%2]\n"
      "cbnz %w1, 0b\n"
      "b 2f\n"
      "1:\n"
      "clrex\n"
      "mov %w1, #1\n"
      "2:\n"
      : "=&r"(observed), "=&r"(status)
      : "r"(&object->value), "r"(expected_value), "r"(desired)
      : "cc", "memory");
  (void)status;
  if (observed == expected_value) {
    return 1;
  }
  *expected = observed;
  return 0;
}

static inline uint64_t
ckc_profile_atomic_load_relaxed_u64(ckc_profile_atomic_u64 *object) {
  uint64_t value;
  __asm__ volatile("ldr %0, [%1]" : "=r"(value) : "r"(&object->value)
                   : "memory");
  return value;
}

static inline int ckc_profile_atomic_compare_exchange_weak_u64(
    ckc_profile_atomic_u64 *object, uint64_t *expected, uint64_t desired) {
  const uint64_t expected_value = *expected;
  uint64_t observed;
  uint32_t status;
  __asm__ volatile(
      "0:\n"
      "ldxr %0, [%2]\n"
      "cmp %0, %3\n"
      "b.ne 1f\n"
      "stxr %w1, %4, [%2]\n"
      "cbnz %w1, 0b\n"
      "b 2f\n"
      "1:\n"
      "clrex\n"
      "mov %w1, #1\n"
      "2:\n"
      : "=&r"(observed), "=&r"(status)
      : "r"(&object->value), "r"(expected_value), "r"(desired)
      : "cc", "memory");
  (void)status;
  if (observed == expected_value) {
    return 1;
  }
  *expected = observed;
  return 0;
}

#else

#include <stdatomic.h>

#if ATOMIC_LLONG_LOCK_FREE != 2
#error CK profile generation requires lock-free 64-bit atomics
#endif

typedef _Atomic uint32_t ckc_profile_atomic_u32;
typedef _Atomic uint64_t ckc_profile_atomic_u64;

static inline uint32_t
ckc_profile_atomic_load_acquire_u32(ckc_profile_atomic_u32 *object) {
  return atomic_load_explicit(object, memory_order_acquire);
}

static inline uint32_t
ckc_profile_atomic_load_relaxed_u32(ckc_profile_atomic_u32 *object) {
  return atomic_load_explicit(object, memory_order_relaxed);
}

static inline void
ckc_profile_atomic_store_release_u32(ckc_profile_atomic_u32 *object,
                                     uint32_t value) {
  atomic_store_explicit(object, value, memory_order_release);
}

static inline void
ckc_profile_atomic_store_relaxed_u32(ckc_profile_atomic_u32 *object,
                                     uint32_t value) {
  atomic_store_explicit(object, value, memory_order_relaxed);
}

static inline int ckc_profile_atomic_compare_exchange_strong_u32(
    ckc_profile_atomic_u32 *object, uint32_t *expected, uint32_t desired) {
  return atomic_compare_exchange_strong_explicit(
      object, expected, desired, memory_order_acq_rel, memory_order_acquire);
}

static inline uint64_t
ckc_profile_atomic_load_relaxed_u64(ckc_profile_atomic_u64 *object) {
  return atomic_load_explicit(object, memory_order_relaxed);
}

static inline int ckc_profile_atomic_compare_exchange_weak_u64(
    ckc_profile_atomic_u64 *object, uint64_t *expected, uint64_t desired) {
  return atomic_compare_exchange_weak_explicit(
      object, expected, desired, memory_order_relaxed, memory_order_relaxed);
}

#endif

#endif
