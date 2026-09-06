#include "ckc_dispatch_runtime.h"

#if defined(_MSC_VER)
#include <intrin.h>
#if defined(_M_ARM64)
#pragma intrinsic(_InterlockedCompareExchange_acq)
#pragma intrinsic(_InterlockedExchange_rel)
#else
#pragma intrinsic(_InterlockedCompareExchange)
#pragma intrinsic(_InterlockedExchange)
#endif
#elif !defined(__aarch64__) || !defined(__linux__)
#include <stdatomic.h>
#endif

#if defined(_MSC_VER)
typedef volatile long ck_dispatch_atomic_u32;
#elif defined(__aarch64__) && defined(__linux__)
typedef uint32_t ck_dispatch_atomic_u32;
#else
typedef _Atomic uint32_t ck_dispatch_atomic_u32;
#endif

static ck_dispatch_atomic_u32 ck_capability_state;

static uint32_t
ck_dispatch_load_acquire(ck_dispatch_atomic_u32 *object) {
#if defined(_MSC_VER)
#if defined(_M_ARM64)
  return (uint32_t)_InterlockedCompareExchange_acq(object, 0, 0);
#else
  return (uint32_t)_InterlockedCompareExchange(object, 0, 0);
#endif
#elif defined(__aarch64__) && defined(__linux__)
  uint32_t value;
  __asm__ volatile("ldar %w0, [%1]" : "=r"(value) : "r"(object) : "memory");
  return value;
#else
  return atomic_load_explicit(object, memory_order_acquire);
#endif
}

static void ck_dispatch_store_release(ck_dispatch_atomic_u32 *object,
                                      uint32_t value) {
#if defined(_MSC_VER)
#if defined(_M_ARM64)
  (void)_InterlockedExchange_rel(object, (long)value);
#else
  (void)_InterlockedExchange(object, (long)value);
#endif
#elif defined(__aarch64__) && defined(__linux__)
  __asm__ volatile("stlr %w0, [%1]" : : "r"(value), "r"(object) : "memory");
#else
  atomic_store_explicit(object, value, memory_order_release);
#endif
}

static int ck_dispatch_compare_exchange(ck_dispatch_atomic_u32 *object,
                                        uint32_t *expected,
                                        uint32_t desired) {
#if defined(_MSC_VER)
#if defined(_M_ARM64)
  const long observed =
      _InterlockedCompareExchange_acq(object, (long)desired, (long)*expected);
#else
  const long observed =
      _InterlockedCompareExchange(object, (long)desired, (long)*expected);
#endif
  if ((uint32_t)observed == *expected) {
    return 1;
  }
  *expected = (uint32_t)observed;
  return 0;
#elif defined(__aarch64__) && defined(__linux__)
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
      : "r"(object), "r"(expected_value), "r"(desired)
      : "cc", "memory");
  (void)status;
  if (observed == expected_value) {
    return 1;
  }
  *expected = observed;
  return 0;
#else
  return atomic_compare_exchange_strong_explicit(
      object, expected, desired, memory_order_acq_rel, memory_order_acquire);
#endif
}

#if defined(__aarch64__) && defined(__linux__)
static uintptr_t ck_initial_hwcap;
static uintptr_t ck_initial_hwcap2;
static uint32_t ck_initial_auxv_valid;

enum {
  CK_LINUX_AT_FDCWD = -100,
  CK_LINUX_SYS_OPENAT = 56,
  CK_LINUX_SYS_CLOSE = 57,
  CK_LINUX_SYS_READ = 63
};

static long ck_linux_syscall4(long number, long argument0, long argument1,
                              long argument2, long argument3) {
  register long x0 __asm__("x0") = argument0;
  register long x1 __asm__("x1") = argument1;
  register long x2 __asm__("x2") = argument2;
  register long x3 __asm__("x3") = argument3;
  register long x8 __asm__("x8") = number;
  __asm__ volatile("svc #0"
                   : "+r"(x0)
                   : "r"(x1), "r"(x2), "r"(x3), "r"(x8)
                   : "cc", "memory");
  return x0;
}

static uint32_t ck_dispatch_read_proc_auxv(uintptr_t *hwcap,
                                           uintptr_t *hwcap2) {
  enum { CK_AT_NULL = 0, CK_AT_HWCAP = 16, CK_AT_HWCAP2 = 26 };
  static const char path[] = "/proc/self/auxv";
  struct ck_auxv_entry {
    uintptr_t type;
    uintptr_t value;
  } entry;
  const long descriptor = ck_linux_syscall4(
      CK_LINUX_SYS_OPENAT, CK_LINUX_AT_FDCWD, (long)(uintptr_t)path, 0, 0);
  uint32_t saw_hwcap = 0;
  uint32_t saw_hwcap2 = 0;
  if (descriptor < 0) {
    return 0;
  }
  for (;;) {
    const long bytes = ck_linux_syscall4(
        CK_LINUX_SYS_READ, descriptor, (long)(uintptr_t)&entry,
        (long)sizeof(entry), 0);
    if (bytes != (long)sizeof(entry)) {
      break;
    }
    if (entry.type == CK_AT_NULL) {
      break;
    }
    if (entry.type == CK_AT_HWCAP) {
      *hwcap = entry.value;
      saw_hwcap = 1u;
    } else if (entry.type == CK_AT_HWCAP2) {
      *hwcap2 = entry.value;
      saw_hwcap2 = 1u;
    }
  }
  (void)ck_linux_syscall4(CK_LINUX_SYS_CLOSE, descriptor, 0, 0, 0);
  return saw_hwcap & saw_hwcap2;
}
#endif

void __ck_dispatch_capture_initial_stack(const uintptr_t *stack) {
#if defined(__aarch64__) && defined(__linux__)
  enum { CK_AT_NULL = 0, CK_AT_HWCAP = 16, CK_AT_HWCAP2 = 26 };
  uintptr_t hwcap = 0;
  uintptr_t hwcap2 = 0;
  uint32_t saw_hwcap = 0;
  uint32_t saw_hwcap2 = 0;
  if (stack == (const uintptr_t *)0 || ck_initial_auxv_valid != 0u) {
    return;
  }
  const uintptr_t argc = stack[0];
  const uintptr_t *cursor = stack + argc + 2u;
  while (*cursor != 0u) {
    ++cursor;
  }
  ++cursor;
  while (cursor[0] != CK_AT_NULL) {
    if (cursor[0] == CK_AT_HWCAP) {
      hwcap = cursor[1];
      saw_hwcap = 1u;
    } else if (cursor[0] == CK_AT_HWCAP2) {
      hwcap2 = cursor[1];
      saw_hwcap2 = 1u;
    }
    cursor += 2;
  }
  ck_initial_hwcap = hwcap;
  ck_initial_hwcap2 = hwcap2;
  ck_initial_auxv_valid = saw_hwcap & saw_hwcap2;
#else
  (void)stack;
#endif
}

#if defined(__x86_64__) || defined(_M_X64)
static void ck_cpuid(uint32_t leaf, uint32_t subleaf, uint32_t *a,
                     uint32_t *b, uint32_t *c, uint32_t *d) {
#if defined(_MSC_VER)
  int registers[4];
  __cpuidex(registers, (int)leaf, (int)subleaf);
  *a = (uint32_t)registers[0];
  *b = (uint32_t)registers[1];
  *c = (uint32_t)registers[2];
  *d = (uint32_t)registers[3];
#else
  __asm__ volatile("cpuid"
                   : "=a"(*a), "=b"(*b), "=c"(*c), "=d"(*d)
                   : "a"(leaf), "c"(subleaf));
#endif
}

static uint64_t ck_xgetbv(void) {
#if defined(_MSC_VER)
  return _xgetbv(0);
#else
  uint32_t low;
  uint32_t high;
  __asm__ volatile("xgetbv" : "=a"(low), "=d"(high) : "c"(0));
  return ((uint64_t)high << 32) | low;
#endif
}

static uint32_t ck_detect_x86(void) {
  uint32_t a;
  uint32_t b;
  uint32_t c;
  uint32_t d;
  ck_cpuid(0, 0, &a, &b, &c, &d);
  if (a < 7u) {
    return CK_DISPATCH_BASELINE;
  }
  ck_cpuid(0x80000000u, 0, &a, &b, &c, &d);
  if (a < 0x80000001u) {
    return CK_DISPATCH_BASELINE;
  }
  ck_cpuid(1, 0, &a, &b, &c, &d);
  const uint32_t required_leaf1 = (1u << 0) | (1u << 9) | (1u << 12) |
                                  (1u << 19) | (1u << 20) | (1u << 22) |
                                  (1u << 23) | (1u << 27) | (1u << 28) |
                                  (1u << 29);
  if ((c & required_leaf1) != required_leaf1) {
    return CK_DISPATCH_BASELINE;
  }
  const uint64_t xcr0 = ck_xgetbv();
  if ((xcr0 & ((1u << 1) | (1u << 2))) !=
      ((1u << 1) | (1u << 2))) {
    return CK_DISPATCH_BASELINE;
  }
  ck_cpuid(7, 0, &a, &b, &c, &d);
  const uint32_t required_v3_leaf7 = (1u << 3) | (1u << 5) | (1u << 8);
  if ((b & required_v3_leaf7) != required_v3_leaf7) {
    return CK_DISPATCH_BASELINE;
  }
  ck_cpuid(0x80000001u, 0, &a, &b, &c, &d);
  if ((c & (1u << 5)) == 0u) {
    return CK_DISPATCH_BASELINE;
  }
  ck_cpuid(7, 0, &a, &b, &c, &d);
  const uint32_t required_v4_leaf7 = (1u << 16) | (1u << 17) |
                                    (1u << 28) | (1u << 30) | (1u << 31);
  const uint64_t required_v4_xcr0 =
      (1u << 5) | (1u << 6) | (1u << 7);
  if ((b & required_v4_leaf7) == required_v4_leaf7 &&
      (xcr0 & required_v4_xcr0) == required_v4_xcr0) {
    return CK_DISPATCH_X86_V4;
  }
  return CK_DISPATCH_X86_V3;
}
#endif

static uint32_t ck_detect_uncached(void) {
#if defined(__x86_64__) || defined(_M_X64)
  return ck_detect_x86();
#elif defined(__aarch64__) && defined(__linux__)
  enum { CK_HWCAP_SVE = 1u << 22, CK_HWCAP2_SVE2 = 1u << 1 };
  uintptr_t hwcap = ck_initial_hwcap;
  uintptr_t hwcap2 = ck_initial_hwcap2;
  if (ck_initial_auxv_valid == 0u &&
      ck_dispatch_read_proc_auxv(&hwcap, &hwcap2) == 0u) {
    return CK_DISPATCH_BASELINE;
  }
  if ((hwcap & CK_HWCAP_SVE) == 0u) {
    return CK_DISPATCH_BASELINE;
  }
  if ((hwcap2 & CK_HWCAP2_SVE2) != 0u) {
    return CK_DISPATCH_ARM_SVE2;
  }
  return CK_DISPATCH_ARM_SVE;
#else
  return CK_DISPATCH_BASELINE;
#endif
}

uint32_t __ck_dispatch_detect_capabilities(void) {
  uint32_t state = ck_dispatch_load_acquire(&ck_capability_state);
  if (state == 0u) {
    uint32_t expected = 0u;
    if (ck_dispatch_compare_exchange(&ck_capability_state, &expected, 1u)) {
      const uint32_t capabilities = ck_detect_uncached();
      ck_dispatch_store_release(&ck_capability_state, capabilities + 2u);
      return capabilities;
    }
    state = expected;
  }
  while (state == 1u) {
    state = ck_dispatch_load_acquire(&ck_capability_state);
  }
  return state - 2u;
}

uint32_t __ck_dispatch_select_ranked(const uint32_t *required,
                                     uint32_t count) {
  const uint32_t capabilities = __ck_dispatch_detect_capabilities();
  if (required == (const uint32_t *)0 || count == 0u) {
    return 0xffffffffu;
  }
  for (uint32_t index = 0; index < count; ++index) {
    if ((capabilities & required[index]) == required[index]) {
      return index;
    }
  }
  return count - 1u;
}
