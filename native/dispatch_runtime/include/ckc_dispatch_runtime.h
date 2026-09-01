#ifndef CKC_DISPATCH_RUNTIME_H
#define CKC_DISPATCH_RUNTIME_H

#include <stddef.h>
#include <stdint.h>

#if defined(_MSC_VER)
#define CKC_DISPATCH_HIDDEN
#define CKC_DISPATCH_WEAK
#define CKC_DISPATCH_NOINLINE __declspec(noinline)
#else
#define CKC_DISPATCH_HIDDEN __attribute__((visibility("hidden")))
#define CKC_DISPATCH_WEAK __attribute__((weak))
#define CKC_DISPATCH_NOINLINE __attribute__((noinline))
#endif

enum CK_DispatchCapability {
  CK_DISPATCH_BASELINE = 0,
  CK_DISPATCH_X86_V3 = 1u << 0,
  CK_DISPATCH_X86_V4 = (1u << 0) | (1u << 1),
  CK_DISPATCH_ARM_SVE = 1u << 2,
  CK_DISPATCH_ARM_SVE2 = (1u << 2) | (1u << 3),
};

/* Compiler-private ABI. No declaration from this header enters CK headers. */
CKC_DISPATCH_HIDDEN CKC_DISPATCH_WEAK void
__ck_dispatch_capture_initial_stack(const uintptr_t *stack);
CKC_DISPATCH_HIDDEN CKC_DISPATCH_WEAK uint32_t
__ck_dispatch_detect_capabilities(void);
CKC_DISPATCH_HIDDEN CKC_DISPATCH_WEAK uint32_t
__ck_dispatch_select_ranked(const uint32_t *required, uint32_t count);

#endif
