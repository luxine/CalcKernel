#include "ckc_runtime.h"

typedef void *CK_HANDLE;
typedef unsigned long CK_DWORD;
typedef int CK_BOOL;

__declspec(dllimport) CK_HANDLE __stdcall GetStdHandle(CK_DWORD number);
__declspec(dllimport) CK_BOOL __stdcall WriteFile(CK_HANDLE handle,
                                                  const void *bytes,
                                                  CK_DWORD length,
                                                  CK_DWORD *written,
                                                  void *overlapped);
__declspec(dllimport) CKC_NORETURN void __stdcall ExitProcess(CK_DWORD status);
extern int main(void);

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
