#include "ckc_runtime.h"

extern long write(int file_descriptor, const void *bytes, unsigned long length);
extern int fcntl(int file_descriptor, int command, int value);
extern void *signal(int signal_number, void *handler);
extern CKC_NORETURN void _exit(int status);

// LC_MAIN enters at a process stack boundary rather than through a C call.
// Route it through a runtime-owned stub so the CK `main` body always observes
// the platform C ABI and a normal return address. This is observable on real
// Intel Darwin when LLVM emits aligned stack accesses for non-trivial code.
#if defined(__x86_64__)
__asm__(".text\n"
        ".p2align 4, 0x90\n"
        ".globl ___ck_start\n"
        "___ck_start:\n"
        "andq $-16, %rsp\n"
        "callq _main\n"
        "movl %eax, %edi\n"
        "callq ___ck_platform_exit\n"
        "ud2\n");
#elif defined(__aarch64__)
__asm__(".text\n"
        ".p2align 2\n"
        ".globl ___ck_start\n"
        "___ck_start:\n"
        "bl _main\n"
        "bl ___ck_platform_exit\n"
        "brk #0\n");
#else
#error unsupported Darwin runtime architecture
#endif

int64_t __ck_platform_write(int32_t stream, const uint8_t *bytes,
                            uint64_t length) {
  enum { CK_F_SETNOSIGPIPE = 73 };
  (void)fcntl(stream, CK_F_SETNOSIGPIPE, 1);
  (void)signal(13, (void *)(uintptr_t)1);
  return (int64_t)write(stream, bytes, (unsigned long)length);
}

void __ck_platform_exit(int32_t status) { _exit(status); }
