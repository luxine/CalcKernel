#ifndef CKC_PROFILE_PLATFORM_H
#define CKC_PROFILE_PLATFORM_H

#include <stdint.h>

enum {
  CKC_PROFILE_PLATFORM_OK = 0,
  CKC_PROFILE_PLATFORM_COLLISION = 1,
  CKC_PROFILE_PLATFORM_ERROR = -1,
  CKC_PROFILE_PLATFORM_OPEN_ERROR = -2,
  CKC_PROFILE_PLATFORM_IDENTITY_ERROR = -3,
  CKC_PROFILE_PLATFORM_CREATE_ERROR = -4,
  CKC_PROFILE_PLATFORM_WRITE_ERROR = -5,
  CKC_PROFILE_PLATFORM_FILE_SYNC_ERROR = -6,
  CKC_PROFILE_PLATFORM_RENAME_ERROR = -7,
  CKC_PROFILE_PLATFORM_DIRECTORY_SYNC_ERROR = -8
};

static void *__ck_profile_platform_allocate(uint64_t length);
static int32_t __ck_profile_platform_random(uint8_t output[16]);
static int32_t __ck_profile_platform_publish(
    const uint8_t *directory, uint32_t directory_length,
    uint64_t identity_first, uint64_t identity_second,
    const uint8_t run_id[16], const uint8_t *bytes, uint64_t length);

#endif
