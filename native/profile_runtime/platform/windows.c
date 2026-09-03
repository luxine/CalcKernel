#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>

static void *__ck_profile_platform_allocate(uint64_t length) {
  if (length == 0u || length > (uint64_t)SIZE_MAX) {
    return (void *)0;
  }
  return VirtualAlloc((void *)0, (SIZE_T)length, MEM_RESERVE | MEM_COMMIT,
                      PAGE_READWRITE);
}

static int32_t __ck_profile_platform_random(uint8_t output[16]) {
  static volatile LONG serial;
  FILETIME time;
  LARGE_INTEGER ticks;
  GetSystemTimeAsFileTime(&time);
  if (!QueryPerformanceCounter(&ticks)) {
    return CKC_PROFILE_PLATFORM_ERROR;
  }
  const uint64_t values[3] = {
      ((uint64_t)time.dwHighDateTime << 32u) | time.dwLowDateTime,
      (uint64_t)ticks.QuadPart,
      ((uint64_t)GetCurrentProcessId() << 32u) |
          ((uint64_t)GetCurrentThreadId() << 1u) |
          (uint32_t)InterlockedIncrement(&serial)};
  CkProfileSha256 sha;
  uint8_t digest[32];
  static const uint8_t domain[] = "CK-PROFILE-WINDOWS-RUN-ID\0";
  ck_sha_init(&sha);
  ck_sha_update(&sha, domain, sizeof(domain) - 1u);
  ck_sha_update(&sha, (const uint8_t *)values, sizeof(values));
  ck_sha_finish(&sha, digest);
  for (uint32_t index = 0; index < 16u; ++index) {
    output[index] = digest[index];
  }
  return CKC_PROFILE_PLATFORM_OK;
}

static wchar_t *ck_profile_wide_path(const uint8_t *path, uint32_t length,
                                     uint32_t extra, int *characters) {
  if (length == 0u || length > 0x7fffffffu) {
    return (wchar_t *)0;
  }
  const int required = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS,
                                            (const char *)path, (int)length,
                                            (wchar_t *)0, 0);
  if (required <= 0 || (uint32_t)required > 0xffffffffu - extra - 1u) {
    return (wchar_t *)0;
  }
  wchar_t *wide = (wchar_t *)VirtualAlloc(
      (void *)0, ((SIZE_T)required + extra + 1u) * sizeof(wchar_t),
      MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE);
  if (wide == (wchar_t *)0 ||
      MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, (const char *)path,
                          (int)length, wide, required) != required) {
    return (wchar_t *)0;
  }
  wide[required] = 0;
  *characters = required;
  return wide;
}

static int ck_profile_directory_identity(const wchar_t *path, int length,
                                         uint64_t first, uint64_t second) {
  for (int index = 0; index <= length; ++index) {
    if (index != length && path[index] != L'/' && path[index] != L'\\') {
      continue;
    }
    if (index <= 2) {
      continue;
    }
    wchar_t *mutable_path = (wchar_t *)path;
    const wchar_t saved = mutable_path[index];
    mutable_path[index] = 0;
    HANDLE handle = CreateFileW(
        path, FILE_READ_ATTRIBUTES,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, (void *)0,
        OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
        (HANDLE)0);
    mutable_path[index] = saved;
    if (handle == INVALID_HANDLE_VALUE) {
      return 0;
    }
    BY_HANDLE_FILE_INFORMATION information;
    const int ok = GetFileInformationByHandle(handle, &information) &&
                   (information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT) ==
                       0;
    CloseHandle(handle);
    if (!ok) {
      return 0;
    }
  }
  HANDLE directory = CreateFileW(
      path, FILE_READ_ATTRIBUTES,
      FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, (void *)0,
      OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
      (HANDLE)0);
  if (directory == INVALID_HANDLE_VALUE) {
    return 0;
  }
  BY_HANDLE_FILE_INFORMATION information;
  const int valid = GetFileInformationByHandle(directory, &information) &&
                    (information.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) != 0 &&
                    (information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT) == 0 &&
                    (uint64_t)information.dwVolumeSerialNumber == first &&
                    (((uint64_t)information.nFileIndexHigh << 32u) |
                     information.nFileIndexLow) == second;
  CloseHandle(directory);
  return valid;
}

static wchar_t ck_profile_hex(uint8_t nibble) {
  return (wchar_t)(nibble < 10u ? L'0' + nibble : L'a' + nibble - 10u);
}

static int32_t __ck_profile_platform_publish(
    const uint8_t *directory, uint32_t directory_length,
    uint64_t identity_first, uint64_t identity_second,
    const uint8_t run_id[16], const uint8_t *bytes, uint64_t length) {
  int directory_characters = 0;
  wchar_t *directory_path = ck_profile_wide_path(
      directory, directory_length, directory_length * 2u + 130u,
      &directory_characters);
  if (directory_path == (wchar_t *)0 ||
      !ck_profile_directory_identity(directory_path, directory_characters,
                                     identity_first, identity_second)) {
    return directory_path == (wchar_t *)0
               ? CKC_PROFILE_PLATFORM_OPEN_ERROR
               : CKC_PROFILE_PLATFORM_IDENTITY_ERROR;
  }
  wchar_t *temporary = directory_path + directory_characters + 1;
  wchar_t *completed = temporary + directory_characters + 64;
  for (int index = 0; index < directory_characters; ++index) {
    temporary[index] = directory_path[index];
    completed[index] = directory_path[index];
  }
  temporary[directory_characters] = L'\\';
  completed[directory_characters] = L'\\';
  static const wchar_t temporary_prefix[] = L".ck-profile-";
  static const wchar_t completed_prefix[] = L"ck-";
  int temporary_offset = directory_characters + 1;
  int completed_offset = directory_characters + 1;
  for (uint32_t index = 0; index < 12u; ++index) {
    temporary[temporary_offset++] = temporary_prefix[index];
  }
  for (uint32_t index = 0; index < 3u; ++index) {
    completed[completed_offset++] = completed_prefix[index];
  }
  for (uint32_t index = 0; index < 16u; ++index) {
    const wchar_t high = ck_profile_hex((uint8_t)(run_id[index] >> 4u));
    const wchar_t low = ck_profile_hex((uint8_t)(run_id[index] & 15u));
    temporary[temporary_offset++] = high;
    temporary[temporary_offset++] = low;
    completed[completed_offset++] = high;
    completed[completed_offset++] = low;
  }
  static const wchar_t temporary_suffix[] = L".tmp";
  static const wchar_t completed_suffix[] = L".ckprof-part";
  for (uint32_t index = 0; index < 5u; ++index) {
    temporary[temporary_offset++] = temporary_suffix[index];
  }
  for (uint32_t index = 0; index < 13u; ++index) {
    completed[completed_offset++] = completed_suffix[index];
  }
  HANDLE file = CreateFileW(temporary, GENERIC_WRITE, 0, (void *)0, CREATE_NEW,
                            FILE_ATTRIBUTE_NORMAL, (HANDLE)0);
  if (file == INVALID_HANDLE_VALUE) {
    return GetLastError() == ERROR_FILE_EXISTS
               ? CKC_PROFILE_PLATFORM_COLLISION
               : CKC_PROFILE_PLATFORM_CREATE_ERROR;
  }
  uint64_t offset = 0;
  int32_t failure = CKC_PROFILE_PLATFORM_OK;
  while (offset < length) {
    const uint64_t remaining = length - offset;
    const DWORD request = remaining > 0x7fffffffu ? 0x7fffffffu : (DWORD)remaining;
    DWORD written = 0;
    if (!WriteFile(file, bytes + offset, request, &written, (void *)0) ||
        written == 0u) {
      failure = CKC_PROFILE_PLATFORM_WRITE_ERROR;
      break;
    }
    offset += written;
  }
  if (failure == CKC_PROFILE_PLATFORM_OK && !FlushFileBuffers(file)) {
    failure = CKC_PROFILE_PLATFORM_FILE_SYNC_ERROR;
  }
  if (!CloseHandle(file)) {
    failure = CKC_PROFILE_PLATFORM_WRITE_ERROR;
  }
  if (failure == CKC_PROFILE_PLATFORM_OK &&
      !MoveFileExW(temporary, completed, MOVEFILE_WRITE_THROUGH)) {
    failure = CKC_PROFILE_PLATFORM_RENAME_ERROR;
  }
  if (failure != CKC_PROFILE_PLATFORM_OK) {
    (void)DeleteFileW(temporary);
  }
  return failure;
}
