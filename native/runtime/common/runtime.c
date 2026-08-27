#include "ckc_runtime.h"

typedef struct CK_RuntimeFailure {
  const uint8_t *message;
  uint32_t length;
  int32_t exit_status;
} CK_RuntimeFailure;

static const uint8_t CKR0001[] = "CKR0001: integer overflow\n";
static const uint8_t CKR0002[] =
    "CKR0002: integer division or modulo by zero\n";
static const uint8_t CKR0003[] = "CKR0003: null checked result pointer\n";
static const uint8_t CKR0004[] =
    "CKR0004: slice index or sub-slice out of bounds\n";
static const uint8_t CKR0005[] = "CKR0005: standard output write failed\n";
static const uint8_t CKR0006[] =
    "CKR0006: native child terminated abnormally\n";

static CK_RuntimeFailure failure_for_status(int32_t status) {
  switch (status) {
  case 1:
    return (CK_RuntimeFailure){CKR0001, sizeof(CKR0001) - 1u, 240};
  case 2:
    return (CK_RuntimeFailure){CKR0002, sizeof(CKR0002) - 1u, 241};
  case 3:
    return (CK_RuntimeFailure){CKR0003, sizeof(CKR0003) - 1u, 242};
  case 4:
    return (CK_RuntimeFailure){CKR0004, sizeof(CKR0004) - 1u, 243};
  case 5:
    return (CK_RuntimeFailure){CKR0005, sizeof(CKR0005) - 1u, 244};
  default:
    return (CK_RuntimeFailure){CKR0006, sizeof(CKR0006) - 1u, 245};
  }
}

static bool write_all(int32_t stream, const uint8_t *bytes, uint32_t length) {
  uint32_t written = 0;
  while (written < length) {
    const int64_t count =
        __ck_platform_write(stream, bytes + written, length - written);
    if (count <= 0 || (uint64_t)count > (uint64_t)(length - written)) {
      return false;
    }
    written += (uint32_t)count;
  }
  return true;
}

void __ck_runtime_write_stdout(const uint8_t *bytes, uint32_t length) {
  if (!write_all(1, bytes, length)) {
    const CK_RuntimeFailure failure = failure_for_status(5);
    (void)write_all(2, failure.message, failure.length);
    __ck_platform_exit(failure.exit_status);
  }
}

void __ck_runtime_fail(int32_t status) {
  const CK_RuntimeFailure failure = failure_for_status(status);
  (void)write_all(2, failure.message, failure.length);
  __ck_platform_exit(failure.exit_status);
}
