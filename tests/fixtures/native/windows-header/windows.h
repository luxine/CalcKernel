#ifndef CKC_TEST_WINDOWS_H
#define CKC_TEST_WINDOWS_H

#include <stdint.h>

// Macro-surface regression only; the real Windows SDK remains the ABI oracle.
#if !defined(NOMINMAX) || defined(CKC_TEST_PREEXISTING_MINMAX)
#define min(a, b) (((a) < (b)) ? (a) : (b))
#define max(a, b) (((a) > (b)) ? (a) : (b))
#endif
#define IMAGE_FILE_DLL 0x2000
#define IMAGE_FILE_EXECUTABLE_IMAGE 0x0002

extern "C" void *GetStdHandle(uint32_t);
extern "C" int32_t WriteFile(void *, const void *, uint32_t, uint32_t *, void *);
extern "C" void ExitProcess(uint32_t);

#endif
