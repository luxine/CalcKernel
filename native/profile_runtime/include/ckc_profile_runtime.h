#ifndef CKC_PROFILE_RUNTIME_H
#define CKC_PROFILE_RUNTIME_H

#include <stddef.h>
#include <stdint.h>

#if defined(_MSC_VER)
#define CKC_PROFILE_HIDDEN
#else
#define CKC_PROFILE_HIDDEN __attribute__((visibility("hidden")))
#endif

#define CKC_PROFILE_RUNTIME_SCHEMA 1u
#define CKC_PROFILE_RUNTIME_STATUS_OK 0
#define CKC_PROFILE_RUNTIME_STATUS_CONFIG 41
#define CKC_PROFILE_RUNTIME_STATUS_MEMORY 42
#define CKC_PROFILE_RUNTIME_STATUS_DIRECTORY 43
#define CKC_PROFILE_RUNTIME_STATUS_WRITE 44
#define CKC_PROFILE_RUNTIME_STATUS_VALIDATE 45

CKC_PROFILE_HIDDEN int32_t __ck_profile_initialize(
    uint8_t *shard, uint64_t shard_length, const uint32_t *counter_offsets,
    uint32_t counter_count, const uint32_t *site_first_counters,
    const uint32_t *site_counter_counts,
    const uint32_t *site_saturation_offsets, uint32_t site_count,
    uint32_t run_id_offset, uint32_t overflow_flag_offset,
    uint32_t digest_offset, const uint8_t *directory, uint32_t directory_length,
    uint64_t directory_identity_first, uint64_t directory_identity_second);
CKC_PROFILE_HIDDEN void __ck_profile_increment(uint32_t site_index);
CKC_PROFILE_HIDDEN void __ck_profile_add(uint32_t site_index, uint64_t value);
CKC_PROFILE_HIDDEN void __ck_profile_observe(uint32_t site_index,
                                             uint32_t bucket_index);
CKC_PROFILE_HIDDEN void __ck_profile_observe_u32(uint32_t site_index,
                                                 uint32_t value);
CKC_PROFILE_HIDDEN void __ck_profile_observe_trip(uint32_t site_index,
                                                  uint64_t value);
CKC_PROFILE_HIDDEN void __ck_profile_candidate_i64(uint32_t site_index,
                                                   int64_t value,
                                                   int64_t candidate);
CKC_PROFILE_HIDDEN int32_t __ck_profile_flush(void);

#endif
