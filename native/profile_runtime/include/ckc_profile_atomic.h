#ifndef CKC_PROFILE_ATOMIC_H
#define CKC_PROFILE_ATOMIC_H

#include <stdint.h>

#if defined(_MSC_VER)
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>

typedef struct CkProfileAtomicU32 {
  volatile LONG value;
} CkProfileAtomicU32;

typedef struct CkProfileAtomicU64 {
  volatile LONG64 value;
} CkProfileAtomicU64;

static uint32_t
ck_profile_atomic_u32_load_acquire(const CkProfileAtomicU32 *atomic) {
  return (uint32_t)InterlockedCompareExchange((volatile LONG *)&atomic->value,
                                               0, 0);
}

static uint32_t
ck_profile_atomic_u32_load_relaxed(const CkProfileAtomicU32 *atomic) {
  return (uint32_t)InterlockedCompareExchange((volatile LONG *)&atomic->value,
                                               0, 0);
}

static void ck_profile_atomic_u32_store_release(CkProfileAtomicU32 *atomic,
                                                 uint32_t value) {
  (void)InterlockedExchange(&atomic->value, (LONG)value);
}

static void ck_profile_atomic_u32_store_relaxed(CkProfileAtomicU32 *atomic,
                                                 uint32_t value) {
  (void)InterlockedExchange(&atomic->value, (LONG)value);
}

static int ck_profile_atomic_u32_compare_exchange_acq_rel(
    CkProfileAtomicU32 *atomic, uint32_t *expected, uint32_t desired) {
  const LONG observed = InterlockedCompareExchange(
      &atomic->value, (LONG)desired, (LONG)*expected);
  if ((uint32_t)observed == *expected) {
    return 1;
  }
  *expected = (uint32_t)observed;
  return 0;
}

static uint64_t
ck_profile_atomic_u64_load_relaxed(const CkProfileAtomicU64 *atomic) {
  return (uint64_t)InterlockedCompareExchange64(
      (volatile LONG64 *)&atomic->value, 0, 0);
}

static int ck_profile_atomic_u64_compare_exchange_relaxed(
    CkProfileAtomicU64 *atomic, uint64_t *expected, uint64_t desired) {
  const LONG64 observed = InterlockedCompareExchange64(
      &atomic->value, (LONG64)desired, (LONG64)*expected);
  if ((uint64_t)observed == *expected) {
    return 1;
  }
  *expected = (uint64_t)observed;
  return 0;
}

#else

#include <stdatomic.h>

_Static_assert(ATOMIC_LLONG_LOCK_FREE == 2,
               "CK profile generation requires lock-free 64-bit atomics");

typedef struct CkProfileAtomicU32 {
  _Atomic uint32_t value;
} CkProfileAtomicU32;

typedef struct CkProfileAtomicU64 {
  _Atomic uint64_t value;
} CkProfileAtomicU64;

static uint32_t
ck_profile_atomic_u32_load_acquire(const CkProfileAtomicU32 *atomic) {
  return atomic_load_explicit(&atomic->value, memory_order_acquire);
}

static uint32_t
ck_profile_atomic_u32_load_relaxed(const CkProfileAtomicU32 *atomic) {
  return atomic_load_explicit(&atomic->value, memory_order_relaxed);
}

static void ck_profile_atomic_u32_store_release(CkProfileAtomicU32 *atomic,
                                                 uint32_t value) {
  atomic_store_explicit(&atomic->value, value, memory_order_release);
}

static void ck_profile_atomic_u32_store_relaxed(CkProfileAtomicU32 *atomic,
                                                 uint32_t value) {
  atomic_store_explicit(&atomic->value, value, memory_order_relaxed);
}

static int ck_profile_atomic_u32_compare_exchange_acq_rel(
    CkProfileAtomicU32 *atomic, uint32_t *expected, uint32_t desired) {
  return atomic_compare_exchange_strong_explicit(
      &atomic->value, expected, desired, memory_order_acq_rel,
      memory_order_acquire);
}

static uint64_t
ck_profile_atomic_u64_load_relaxed(const CkProfileAtomicU64 *atomic) {
  return atomic_load_explicit(&atomic->value, memory_order_relaxed);
}

static int ck_profile_atomic_u64_compare_exchange_relaxed(
    CkProfileAtomicU64 *atomic, uint64_t *expected, uint64_t desired) {
  return atomic_compare_exchange_weak_explicit(
      &atomic->value, expected, desired, memory_order_relaxed,
      memory_order_relaxed);
}

#endif

#endif
