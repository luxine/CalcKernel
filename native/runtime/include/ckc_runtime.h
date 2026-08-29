#ifndef CKC_RUNTIME_H
#define CKC_RUNTIME_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#if defined(_MSC_VER)
#define CKC_NORETURN __declspec(noreturn)
#define CKC_HIDDEN
#else
#define CKC_NORETURN __attribute__((noreturn))
#define CKC_HIDDEN __attribute__((visibility("hidden")))
#endif

#define CKC_RUNTIME_ABI_VERSION 2u
#define CKC_RUNTIME_BUFFER_SIZE 64u

CKC_HIDDEN int64_t __ck_platform_write(int32_t stream, const uint8_t *bytes,
                                        uint64_t length);
CKC_HIDDEN CKC_NORETURN void __ck_platform_exit(int32_t status);
CKC_HIDDEN CKC_NORETURN void __ck_runtime_fail(int32_t status);
CKC_HIDDEN void __ck_runtime_write_stdout(const uint8_t *bytes, uint32_t length);
CKC_HIDDEN bool __ck_contract_noalias_u64(
    uint64_t left, uint32_t left_length, uint64_t left_element_size,
    uint64_t right, uint32_t right_length, uint64_t right_element_size);

CKC_HIDDEN void __ck_print_i32(int32_t value);
CKC_HIDDEN void __ck_print_i64(int64_t value);
CKC_HIDDEN void __ck_print_u32(uint32_t value);
CKC_HIDDEN void __ck_print_u64(uint64_t value);
CKC_HIDDEN void __ck_print_f64(double value);
CKC_HIDDEN void __ck_print_bool(bool value);
CKC_HIDDEN void __ck_print_newline(void);

#endif
