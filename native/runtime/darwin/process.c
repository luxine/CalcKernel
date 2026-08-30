#include "ckc_runtime.h"

extern long write(int file_descriptor, const void *bytes, unsigned long length);
extern int fcntl(int file_descriptor, int command, int value);
extern void *signal(int signal_number, void *handler);
extern CKC_NORETURN void _exit(int status);

int64_t __ck_platform_write(int32_t stream, const uint8_t *bytes,
                            uint64_t length) {
  enum { CK_F_SETNOSIGPIPE = 73 };
  (void)fcntl(stream, CK_F_SETNOSIGPIPE, 1);
  (void)signal(13, (void *)(uintptr_t)1);
  return (int64_t)write(stream, bytes, (unsigned long)length);
}

void __ck_platform_exit(int32_t status) { _exit(status); }
