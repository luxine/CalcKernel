#include "ckc_profile_atomic.h"
#include "ckc_profile_runtime.h"
#include "ckc_profile_platform.h"

#define CK_PROFILE_NO_OFFSET 0xffffffffu

typedef struct CkProfileState {
  uint8_t *shard;
  uint64_t shard_length;
  const uint32_t *counter_offsets;
  uint32_t counter_count;
  const uint32_t *site_first;
  const uint32_t *site_counts;
  const uint32_t *site_saturation_offsets;
  uint32_t site_count;
  uint32_t run_id_offset;
  uint32_t overflow_flag_offset;
  uint32_t digest_offset;
  const uint8_t *directory;
  uint32_t directory_length;
  uint64_t identity_first;
  uint64_t identity_second;
  ckc_profile_atomic_u64 *counters;
} CkProfileState;

static CkProfileState ck_profile_state;
static ckc_profile_atomic_u32 ck_profile_initialize_state;
static ckc_profile_atomic_u32 ck_profile_flush_state;
static ckc_profile_atomic_u32 ck_profile_overflowed;
static ckc_profile_atomic_u32 ck_profile_incomplete;

static int ck_profile_equal(const uint8_t *left, const uint8_t *right,
                            uint64_t length) {
  for (uint64_t index = 0; index < length; ++index) {
    if (left[index] != right[index]) {
      return 0;
    }
  }
  return 1;
}

static void ck_profile_store_u64_be(uint8_t *output, uint64_t value) {
  for (uint32_t index = 0; index < 8; ++index) {
    output[index] = (uint8_t)(value >> (56u - index * 8u));
  }
}

static int32_t ck_profile_validate_config(void) {
  CkProfileState *state = &ck_profile_state;
  if (state->shard == (void *)0 || state->counter_offsets == (void *)0 ||
      state->site_first == (void *)0 || state->site_counts == (void *)0 ||
      state->site_saturation_offsets == (void *)0 ||
      state->directory == (void *)0 || state->directory_length == 0u ||
      state->directory[state->directory_length] != 0u ||
      state->shard_length < 44u || state->digest_offset + 32u != state->shard_length ||
      state->run_id_offset + 16u > state->digest_offset ||
      state->overflow_flag_offset >= state->digest_offset ||
      state->digest_offset - state->overflow_flag_offset < 2u ||
      !ck_profile_equal(state->shard, (const uint8_t *)"CKPART01", 8u)) {
    return CKC_PROFILE_RUNTIME_STATUS_CONFIG;
  }
  for (uint32_t site = 0; site < state->site_count; ++site) {
    const uint32_t first = state->site_first[site];
    const uint32_t count = state->site_counts[site];
    if (count == 0u || first > state->counter_count ||
        count > state->counter_count - first) {
      return CKC_PROFILE_RUNTIME_STATUS_CONFIG;
    }
  }
  for (uint32_t index = 0; index < state->counter_count; ++index) {
    if (state->counter_offsets[index] > state->digest_offset ||
        state->digest_offset - state->counter_offsets[index] < 8u) {
      return CKC_PROFILE_RUNTIME_STATUS_CONFIG;
    }
  }
  return CKC_PROFILE_RUNTIME_STATUS_OK;
}

int32_t __ck_profile_initialize(
    uint8_t *shard, uint64_t shard_length, const uint32_t *counter_offsets,
    uint32_t counter_count, const uint32_t *site_first_counters,
    const uint32_t *site_counter_counts,
    const uint32_t *site_saturation_offsets, uint32_t site_count,
    uint32_t run_id_offset, uint32_t overflow_flag_offset,
    uint32_t digest_offset, const uint8_t *directory, uint32_t directory_length,
    uint64_t directory_identity_first, uint64_t directory_identity_second) {
  uint32_t state =
      ckc_profile_atomic_load_acquire_u32(&ck_profile_initialize_state);
  if (state >= 2u) {
    return state == 2u ? CKC_PROFILE_RUNTIME_STATUS_OK
                       : CKC_PROFILE_RUNTIME_STATUS_CONFIG;
  }
  uint32_t expected = 0u;
  if (!ckc_profile_atomic_compare_exchange_strong_u32(
          &ck_profile_initialize_state, &expected, 1u)) {
    do {
      state =
          ckc_profile_atomic_load_acquire_u32(&ck_profile_initialize_state);
    } while (state == 1u);
    return state == 2u ? CKC_PROFILE_RUNTIME_STATUS_OK
                       : CKC_PROFILE_RUNTIME_STATUS_CONFIG;
  }

  ck_profile_state.shard = shard;
  ck_profile_state.shard_length = shard_length;
  ck_profile_state.counter_offsets = counter_offsets;
  ck_profile_state.counter_count = counter_count;
  ck_profile_state.site_first = site_first_counters;
  ck_profile_state.site_counts = site_counter_counts;
  ck_profile_state.site_saturation_offsets = site_saturation_offsets;
  ck_profile_state.site_count = site_count;
  ck_profile_state.run_id_offset = run_id_offset;
  ck_profile_state.overflow_flag_offset = overflow_flag_offset;
  ck_profile_state.digest_offset = digest_offset;
  ck_profile_state.directory = directory;
  ck_profile_state.directory_length = directory_length;
  ck_profile_state.identity_first = directory_identity_first;
  ck_profile_state.identity_second = directory_identity_second;
  int32_t status = ck_profile_validate_config();
  if (status == CKC_PROFILE_RUNTIME_STATUS_OK && counter_count != 0u) {
    const uint64_t bytes =
        (uint64_t)counter_count * sizeof(ckc_profile_atomic_u64);
    ck_profile_state.counters = (ckc_profile_atomic_u64 *)
        __ck_profile_platform_allocate(bytes);
    if (ck_profile_state.counters == (void *)0) {
      status = CKC_PROFILE_RUNTIME_STATUS_MEMORY;
    }
  }
  ckc_profile_atomic_store_release_u32(
      &ck_profile_initialize_state,
      status == CKC_PROFILE_RUNTIME_STATUS_OK ? 2u : 3u);
  return status;
}

static void ck_profile_add_cell(uint32_t index, uint64_t value) {
  if (value == 0u) {
    return;
  }
  if (ckc_profile_atomic_load_acquire_u32(&ck_profile_initialize_state) != 2u ||
      index >= ck_profile_state.counter_count) {
    return;
  }
  ckc_profile_atomic_u64 *counter = &ck_profile_state.counters[index];
  const uint64_t previous =
      ckc_profile_atomic_fetch_add_relaxed_u64(counter, value);
  if (value <= UINT64_MAX - previous) {
    return;
  }
  ckc_profile_atomic_store_relaxed_u32(&ck_profile_overflowed, 1u);
  uint64_t observed = previous + value;
  while (observed != UINT64_MAX &&
         !ckc_profile_atomic_compare_exchange_weak_u64(
             counter, &observed, UINT64_MAX)) {
  }
}

void __ck_profile_increment(uint32_t site_index) {
  if (site_index < ck_profile_state.site_count) {
    ck_profile_add_cell(ck_profile_state.site_first[site_index], 1u);
  }
}

void __ck_profile_add(uint32_t site_index, uint64_t value) {
  if (site_index < ck_profile_state.site_count) {
    ck_profile_add_cell(ck_profile_state.site_first[site_index], value);
  }
}

void __ck_profile_observe(uint32_t site_index, uint32_t bucket_index) {
  if (site_index >= ck_profile_state.site_count) {
    return;
  }
  const uint32_t count = ck_profile_state.site_counts[site_index];
  if (bucket_index >= count) {
    bucket_index = count - 1u;
  }
  ck_profile_add_cell(ck_profile_state.site_first[site_index] + bucket_index,
                      1u);
}

static uint32_t ck_profile_histogram_bucket(uint32_t value) {
  if (value <= 2u) {
    return value;
  }
  if (value <= 4u) {
    return 3u;
  }
  if (value <= 8u) {
    return 4u;
  }
  if (value <= 16u) {
    return 5u;
  }
  if (value <= 32u) {
    return 6u;
  }
  if (value <= 64u) {
    return 7u;
  }
  if (value <= 128u) {
    return 8u;
  }
  if (value <= 256u) {
    return 9u;
  }
  if (value <= 512u) {
    return 10u;
  }
  if (value <= 1024u) {
    return 11u;
  }
  if (value <= 2048u) {
    return 12u;
  }
  if (value <= 4096u) {
    return 13u;
  }
  return value <= 65536u ? 14u : 15u;
}

void __ck_profile_observe_u32(uint32_t site_index, uint32_t value) {
  __ck_profile_observe(site_index, ck_profile_histogram_bucket(value));
}

void __ck_profile_observe_trip(uint32_t site_index, uint64_t value) {
  if (value > UINT32_MAX) {
    ckc_profile_atomic_store_relaxed_u32(&ck_profile_incomplete, 1u);
    value = UINT32_MAX;
  }
  __ck_profile_observe_u32(site_index, (uint32_t)value);
}

void __ck_profile_candidate_i64(uint32_t site_index, int64_t value,
                                int64_t candidate) {
  __ck_profile_observe(site_index, value == candidate ? 0u : 1u);
}

static void ck_profile_digest_shard(void) {
  static const uint8_t domain[] = "CK-PROFILE-SHARD\0";
  CkProfileSha256 sha;
  ck_sha_init(&sha);
  ck_sha_update(&sha, domain, sizeof(domain) - 1u);
  ck_sha_update(&sha, ck_profile_state.shard, ck_profile_state.digest_offset);
  ck_sha_finish(&sha, ck_profile_state.shard + ck_profile_state.digest_offset);
}

static int32_t ck_profile_materialize(const uint8_t run_id[16]) {
  uint32_t overflowed =
      ckc_profile_atomic_load_relaxed_u32(&ck_profile_overflowed);
  for (uint32_t index = 0; index < ck_profile_state.counter_count; ++index) {
    const uint64_t value = ckc_profile_atomic_load_relaxed_u64(
        &ck_profile_state.counters[index]);
    ck_profile_store_u64_be(
        ck_profile_state.shard + ck_profile_state.counter_offsets[index], value);
    overflowed |= value == UINT64_MAX;
  }
  for (uint32_t site = 0; site < ck_profile_state.site_count; ++site) {
    const uint32_t saturation =
        ck_profile_state.site_saturation_offsets[site];
    if (saturation == CK_PROFILE_NO_OFFSET) {
      continue;
    }
    uint8_t saturated = 0;
    const uint32_t first = ck_profile_state.site_first[site];
    const uint32_t count = ck_profile_state.site_counts[site];
    for (uint32_t cell = 0; cell < count; ++cell) {
      saturated |= ckc_profile_atomic_load_relaxed_u64(
                       &ck_profile_state.counters[first + cell]) == UINT64_MAX;
    }
    ck_profile_state.shard[saturation] = saturated;
  }
  for (uint32_t index = 0; index < 16; ++index) {
    ck_profile_state.shard[ck_profile_state.run_id_offset + index] = run_id[index];
  }
  ck_profile_state.shard[ck_profile_state.overflow_flag_offset] =
      overflowed != 0u;
  ck_profile_state.shard[ck_profile_state.overflow_flag_offset + 1u] =
      ckc_profile_atomic_load_relaxed_u32(&ck_profile_incomplete) != 0u;
  ck_profile_digest_shard();

  uint8_t expected[32];
  CkProfileSha256 sha;
  static const uint8_t domain[] = "CK-PROFILE-SHARD\0";
  ck_sha_init(&sha);
  ck_sha_update(&sha, domain, sizeof(domain) - 1u);
  ck_sha_update(&sha, ck_profile_state.shard, ck_profile_state.digest_offset);
  ck_sha_finish(&sha, expected);
  return ck_profile_equal(expected,
                          ck_profile_state.shard + ck_profile_state.digest_offset,
                          32u)
             ? CKC_PROFILE_RUNTIME_STATUS_OK
             : CKC_PROFILE_RUNTIME_STATUS_VALIDATE;
}

int32_t __ck_profile_flush(void) {
  uint32_t terminal =
      ckc_profile_atomic_load_acquire_u32(&ck_profile_flush_state);
  if (terminal >= 2u) {
    return terminal == 2u ? CKC_PROFILE_RUNTIME_STATUS_OK
                          : (int32_t)(terminal - 3u);
  }
  uint32_t expected = 0u;
  if (!ckc_profile_atomic_compare_exchange_strong_u32(
          &ck_profile_flush_state, &expected, 1u)) {
    do {
      terminal = ckc_profile_atomic_load_acquire_u32(&ck_profile_flush_state);
    } while (terminal == 1u);
    return terminal == 2u ? CKC_PROFILE_RUNTIME_STATUS_OK
                          : (int32_t)(terminal - 3u);
  }

  int32_t status =
      ckc_profile_atomic_load_acquire_u32(&ck_profile_initialize_state) == 2u
          ? CKC_PROFILE_RUNTIME_STATUS_OK
          : CKC_PROFILE_RUNTIME_STATUS_CONFIG;
  for (uint32_t attempt = 0;
       status == CKC_PROFILE_RUNTIME_STATUS_OK && attempt < 16u; ++attempt) {
    uint8_t run_id[16];
    if (__ck_profile_platform_random(run_id) != CKC_PROFILE_PLATFORM_OK) {
      status = CKC_PROFILE_RUNTIME_STATUS_WRITE;
      break;
    }
    status = ck_profile_materialize(run_id);
    if (status != CKC_PROFILE_RUNTIME_STATUS_OK) {
      break;
    }
    const int32_t publish = __ck_profile_platform_publish(
        ck_profile_state.directory, ck_profile_state.directory_length,
        ck_profile_state.identity_first, ck_profile_state.identity_second,
        run_id, ck_profile_state.shard, ck_profile_state.shard_length);
    if (publish == CKC_PROFILE_PLATFORM_OK) {
      break;
    }
    if (publish != CKC_PROFILE_PLATFORM_COLLISION) {
      status = CKC_PROFILE_RUNTIME_STATUS_DIRECTORY;
      break;
    }
    status = attempt == 15u ? CKC_PROFILE_RUNTIME_STATUS_WRITE
                            : CKC_PROFILE_RUNTIME_STATUS_OK;
  }
  ckc_profile_atomic_store_release_u32(
      &ck_profile_flush_state,
      status == CKC_PROFILE_RUNTIME_STATUS_OK ? 2u : (uint32_t)status + 3u);
  return status;
}
