#ifndef CKC_PROFILE_ATOMIC_H
#define CKC_PROFILE_ATOMIC_H

#include <stdint.h>

#if defined(_MSC_VER) && defined(_M_ARM64)

#if !defined(__cplusplus)
#error Windows ARM64 profile atomics require the C++20 frontend
#endif

#include <atomic>
#include <new>

typedef struct alignas(std::atomic_ref<uint32_t>::required_alignment)
    CkProfileAtomicU32 {
  uint32_t value;
} CkProfileAtomicU32;

typedef struct alignas(std::atomic_ref<uint64_t>::required_alignment)
    CkProfileAtomicU64 {
  uint64_t value;
} CkProfileAtomicU64;

static_assert(std::atomic_ref<uint32_t>::is_always_lock_free,
              "Windows ARM64 requires lock-free 32-bit profile atomics");
static_assert(std::atomic_ref<uint64_t>::is_always_lock_free,
              "Windows ARM64 requires lock-free 64-bit profile atomics");

static uint32_t
ck_profile_atomic_u32_load_acquire(const CkProfileAtomicU32 *atomic) {
  return std::atomic_ref<uint32_t>(
             const_cast<uint32_t &>(atomic->value))
      .load(std::memory_order_acquire);
}

static uint32_t
ck_profile_atomic_u32_load_relaxed(const CkProfileAtomicU32 *atomic) {
  return std::atomic_ref<uint32_t>(
             const_cast<uint32_t &>(atomic->value))
      .load(std::memory_order_relaxed);
}

static void ck_profile_atomic_u32_store_release(CkProfileAtomicU32 *atomic,
                                                 uint32_t value) {
  std::atomic_ref<uint32_t>(atomic->value).store(value,
                                                  std::memory_order_release);
}

static void ck_profile_atomic_u32_store_relaxed(CkProfileAtomicU32 *atomic,
                                                 uint32_t value) {
  std::atomic_ref<uint32_t>(atomic->value).store(value,
                                                  std::memory_order_relaxed);
}

static uint32_t ck_profile_atomic_u32_fetch_add_relaxed(
    CkProfileAtomicU32 *atomic, uint32_t value) {
  return std::atomic_ref<uint32_t>(atomic->value).fetch_add(
      value, std::memory_order_relaxed);
}

static int ck_profile_atomic_u32_compare_exchange_acq_rel(
    CkProfileAtomicU32 *atomic, uint32_t *expected, uint32_t desired) {
  return std::atomic_ref<uint32_t>(atomic->value)
      .compare_exchange_strong(*expected, desired, std::memory_order_acq_rel,
                               std::memory_order_acquire);
}

static void ck_profile_atomic_u64_initialize(CkProfileAtomicU64 *atomic) {
  ::new ((void *)atomic) CkProfileAtomicU64{0u};
}

static uint64_t
ck_profile_atomic_u64_load_relaxed(const CkProfileAtomicU64 *atomic) {
  return std::atomic_ref<uint64_t>(
             const_cast<uint64_t &>(atomic->value))
      .load(std::memory_order_relaxed);
}

static uint64_t ck_profile_atomic_u64_fetch_add_relaxed(
    CkProfileAtomicU64 *atomic, uint64_t value) {
  return std::atomic_ref<uint64_t>(atomic->value).fetch_add(
      value, std::memory_order_relaxed);
}

static int ck_profile_atomic_u64_compare_exchange_relaxed(
    CkProfileAtomicU64 *atomic, uint64_t *expected, uint64_t desired) {
  return std::atomic_ref<uint64_t>(atomic->value)
      .compare_exchange_weak(*expected, desired, std::memory_order_relaxed,
                             std::memory_order_relaxed);
}

#elif defined(_MSC_VER)

#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>

#include <intrin.h>
#pragma intrinsic(_InterlockedCompareExchange)
#pragma intrinsic(_InterlockedCompareExchange64)
#pragma intrinsic(_InterlockedExchange)
#pragma intrinsic(_InterlockedExchangeAdd)
#pragma intrinsic(_InterlockedExchangeAdd64)

typedef struct CkProfileAtomicU32 {
  volatile LONG value;
} CkProfileAtomicU32;

typedef struct CkProfileAtomicU64 {
  volatile LONG64 value;
} CkProfileAtomicU64;

static uint32_t
ck_profile_atomic_u32_load_acquire(const CkProfileAtomicU32 *atomic) {
  return (uint32_t)_InterlockedCompareExchange(
      (volatile LONG *)&atomic->value, 0, 0);
}

static uint32_t
ck_profile_atomic_u32_load_relaxed(const CkProfileAtomicU32 *atomic) {
  return ck_profile_atomic_u32_load_acquire(atomic);
}

static void ck_profile_atomic_u32_store_release(CkProfileAtomicU32 *atomic,
                                                 uint32_t value) {
  (void)_InterlockedExchange(&atomic->value, (LONG)value);
}

static void ck_profile_atomic_u32_store_relaxed(CkProfileAtomicU32 *atomic,
                                                 uint32_t value) {
  ck_profile_atomic_u32_store_release(atomic, value);
}

static uint32_t ck_profile_atomic_u32_fetch_add_relaxed(
    CkProfileAtomicU32 *atomic, uint32_t value) {
  return (uint32_t)_InterlockedExchangeAdd(&atomic->value, (LONG)value);
}

static int ck_profile_atomic_u32_compare_exchange_acq_rel(
    CkProfileAtomicU32 *atomic, uint32_t *expected, uint32_t desired) {
  const LONG observed = _InterlockedCompareExchange(
      &atomic->value, (LONG)desired, (LONG)*expected);
  if ((uint32_t)observed == *expected) {
    return 1;
  }
  *expected = (uint32_t)observed;
  return 0;
}

static void ck_profile_atomic_u64_initialize(CkProfileAtomicU64 *atomic) {
  atomic->value = 0;
}

static uint64_t
ck_profile_atomic_u64_load_relaxed(const CkProfileAtomicU64 *atomic) {
  return (uint64_t)_InterlockedCompareExchange64(
      (volatile LONG64 *)&atomic->value, 0, 0);
}

static uint64_t ck_profile_atomic_u64_fetch_add_relaxed(
    CkProfileAtomicU64 *atomic, uint64_t value) {
  return (uint64_t)_InterlockedExchangeAdd64(&atomic->value, (LONG64)value);
}

static int ck_profile_atomic_u64_compare_exchange_relaxed(
    CkProfileAtomicU64 *atomic, uint64_t *expected, uint64_t desired) {
  const LONG64 observed = _InterlockedCompareExchange64(
      &atomic->value, (LONG64)desired, (LONG64)*expected);
  if ((uint64_t)observed == *expected) {
    return 1;
  }
  *expected = (uint64_t)observed;
  return 0;
}

#elif defined(__aarch64__) && defined(__linux__)

typedef struct CkProfileAtomicU32 {
  uint32_t value;
} CkProfileAtomicU32;

typedef struct CkProfileAtomicU64 {
  uint64_t value;
} CkProfileAtomicU64;

static uint32_t
ck_profile_atomic_u32_load_acquire(const CkProfileAtomicU32 *atomic) {
  uint32_t value;
  __asm__ volatile("ldar %w0, [%1]" : "=r"(value) : "r"(&atomic->value)
                   : "memory");
  return value;
}

static uint32_t
ck_profile_atomic_u32_load_relaxed(const CkProfileAtomicU32 *atomic) {
  uint32_t value;
  __asm__ volatile("ldr %w0, [%1]" : "=r"(value) : "r"(&atomic->value)
                   : "memory");
  return value;
}

static void ck_profile_atomic_u32_store_release(CkProfileAtomicU32 *atomic,
                                                 uint32_t value) {
  __asm__ volatile("stlr %w0, [%1]" : : "r"(value), "r"(&atomic->value)
                   : "memory");
}

static void ck_profile_atomic_u32_store_relaxed(CkProfileAtomicU32 *atomic,
                                                 uint32_t value) {
  __asm__ volatile("str %w0, [%1]" : : "r"(value), "r"(&atomic->value)
                   : "memory");
}

static inline uint32_t ck_profile_atomic_u32_fetch_add_relaxed(
    CkProfileAtomicU32 *atomic, uint32_t value) {
  uint32_t observed;
  uint32_t next;
  uint32_t status;
  __asm__ volatile(
      "0:\n"
      "ldxr %w0, [%3]\n"
      "add %w2, %w0, %w4\n"
      "stxr %w1, %w2, [%3]\n"
      "cbnz %w1, 0b\n"
      : "=&r"(observed), "=&r"(status), "=&r"(next)
      : "r"(&atomic->value), "r"(value)
      : "memory");
  return observed;
}

static int ck_profile_atomic_u32_compare_exchange_acq_rel(
    CkProfileAtomicU32 *atomic, uint32_t *expected, uint32_t desired) {
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
      : "r"(&atomic->value), "r"(expected_value), "r"(desired)
      : "cc", "memory");
  (void)status;
  if (observed == expected_value) {
    return 1;
  }
  *expected = observed;
  return 0;
}

static void ck_profile_atomic_u64_initialize(CkProfileAtomicU64 *atomic) {
  atomic->value = 0u;
}

static uint64_t
ck_profile_atomic_u64_load_relaxed(const CkProfileAtomicU64 *atomic) {
  uint64_t value;
  __asm__ volatile("ldr %0, [%1]" : "=r"(value) : "r"(&atomic->value)
                   : "memory");
  return value;
}

static uint64_t ck_profile_atomic_u64_fetch_add_relaxed(
    CkProfileAtomicU64 *atomic, uint64_t value) {
  uint64_t observed;
  uint64_t next;
  uint32_t status;
  __asm__ volatile(
      "0:\n"
      "ldxr %0, [%3]\n"
      "add %2, %0, %4\n"
      "stxr %w1, %2, [%3]\n"
      "cbnz %w1, 0b\n"
      : "=&r"(observed), "=&r"(status), "=&r"(next)
      : "r"(&atomic->value), "r"(value)
      : "memory");
  return observed;
}

static int ck_profile_atomic_u64_compare_exchange_relaxed(
    CkProfileAtomicU64 *atomic, uint64_t *expected, uint64_t desired) {
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
      : "r"(&atomic->value), "r"(expected_value), "r"(desired)
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

static inline uint32_t ck_profile_atomic_u32_fetch_add_relaxed(
    CkProfileAtomicU32 *atomic, uint32_t value) {
  return atomic_fetch_add_explicit(&atomic->value, value,
                                   memory_order_relaxed);
}

static int ck_profile_atomic_u32_compare_exchange_acq_rel(
    CkProfileAtomicU32 *atomic, uint32_t *expected, uint32_t desired) {
  return atomic_compare_exchange_strong_explicit(
      &atomic->value, expected, desired, memory_order_acq_rel,
      memory_order_acquire);
}

static void ck_profile_atomic_u64_initialize(CkProfileAtomicU64 *atomic) {
  atomic_init(&atomic->value, 0u);
}

static uint64_t
ck_profile_atomic_u64_load_relaxed(const CkProfileAtomicU64 *atomic) {
  return atomic_load_explicit(&atomic->value, memory_order_relaxed);
}

static uint64_t ck_profile_atomic_u64_fetch_add_relaxed(
    CkProfileAtomicU64 *atomic, uint64_t value) {
  return atomic_fetch_add_explicit(&atomic->value, value,
                                   memory_order_relaxed);
}

static int ck_profile_atomic_u64_compare_exchange_relaxed(
    CkProfileAtomicU64 *atomic, uint64_t *expected, uint64_t desired) {
  return atomic_compare_exchange_weak_explicit(
      &atomic->value, expected, desired, memory_order_relaxed,
      memory_order_relaxed);
}

#endif

#endif
