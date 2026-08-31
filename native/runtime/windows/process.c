#include "ckc_runtime.h"

typedef void *CK_HANDLE;
typedef unsigned long CK_DWORD;
typedef int CK_BOOL;

CK_HANDLE __stdcall GetStdHandle(CK_DWORD number);
CK_BOOL __stdcall WriteFile(CK_HANDLE handle, const void *bytes,
                            CK_DWORD length, CK_DWORD *written,
                            void *overlapped);
CKC_NORETURN void __stdcall ExitProcess(CK_DWORD status);
extern int main(void);

#if defined(_MSC_VER)
#pragma optimize("", off)
#endif

void *memcpy(void *destination, const void *source, size_t length) {
  unsigned char *output = (unsigned char *)destination;
  const unsigned char *input = (const unsigned char *)source;
  while (length != 0u) {
    *output++ = *input++;
    --length;
  }
  return destination;
}

void *memset(void *destination, int value, size_t length) {
  unsigned char *output = (unsigned char *)destination;
  while (length != 0u) {
    *output++ = (unsigned char)value;
    --length;
  }
  return destination;
}

#if defined(_MSC_VER)
#pragma optimize("", on)
#endif

int64_t __ck_platform_write(int32_t stream, const uint8_t *bytes,
                            uint64_t length) {
  const CK_DWORD selector = stream == 1 ? (CK_DWORD)-11 : (CK_DWORD)-12;
  const CK_HANDLE handle = GetStdHandle(selector);
  CK_DWORD written = 0;
  const CK_DWORD request =
      length > 0xffffffffu ? 0xffffffffu : (CK_DWORD)length;
  if (handle == (CK_HANDLE)0 || handle == (CK_HANDLE)(intptr_t)-1 ||
      !WriteFile(handle, bytes, request, &written, (void *)0)) {
    return -1;
  }
  return (int64_t)written;
}

void __ck_platform_exit(int32_t status) { ExitProcess((CK_DWORD)status); }

void mainCRTStartup(void) { ExitProcess((CK_DWORD)main()); }
