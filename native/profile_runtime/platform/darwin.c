#include <fcntl.h>
#include <sys/attr.h>
#include <sys/mman.h>
#include <unistd.h>

extern void arc4random_buf(void *buffer, size_t length);
extern int renameatx_np(int from, const char *from_name, int to,
                        const char *to_name, unsigned int flags);

#ifndef RENAME_EXCL
#define RENAME_EXCL 0x00000004u
#endif

static void *__ck_profile_platform_allocate(uint64_t length) {
  if (length == 0u || length > (uint64_t)SIZE_MAX) {
    return (void *)0;
  }
  void *memory = mmap((void *)0, (size_t)length, PROT_READ | PROT_WRITE,
                      MAP_PRIVATE | MAP_ANON, -1, 0);
  return memory == MAP_FAILED ? (void *)0 : memory;
}

static int32_t __ck_profile_platform_random(uint8_t output[16]) {
  arc4random_buf(output, 16u);
  return CKC_PROFILE_PLATFORM_OK;
}

static int ck_profile_open_directory(const uint8_t *path, uint32_t length) {
  if (length < 2u || path[0] != '/' || path[length] != 0u) {
    return -1;
  }
  int current = open("/", O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC);
  uint32_t offset = 1u;
  while (current >= 0 && offset < length) {
    while (offset < length && path[offset] == '/') {
      ++offset;
    }
    if (offset == length) {
      break;
    }
    char component[256];
    uint32_t count = 0;
    while (offset < length && path[offset] != '/') {
      if (count == 255u) {
        close(current);
        return -1;
      }
      component[count++] = (char)path[offset++];
    }
    component[count] = 0;
    if ((count == 1u && component[0] == '.') ||
        (count == 2u && component[0] == '.' && component[1] == '.')) {
      close(current);
      return -1;
    }
    const int next = openat(current, component,
                            O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC);
    close(current);
    current = next;
  }
  return current;
}

static char ck_profile_hex(uint8_t nibble) {
  return (char)(nibble < 10u ? '0' + nibble : 'a' + nibble - 10u);
}

static void ck_profile_names(const uint8_t run_id[16], char temporary[49],
                             char completed[53]) {
  static const char temporary_prefix[] = ".ck-profile-";
  static const char completed_prefix[] = "ck-";
  for (uint32_t index = 0; index < sizeof(temporary_prefix) - 1u; ++index) {
    temporary[index] = temporary_prefix[index];
  }
  for (uint32_t index = 0; index < sizeof(completed_prefix) - 1u; ++index) {
    completed[index] = completed_prefix[index];
  }
  for (uint32_t index = 0; index < 16u; ++index) {
    const char high = ck_profile_hex((uint8_t)(run_id[index] >> 4u));
    const char low = ck_profile_hex((uint8_t)(run_id[index] & 15u));
    temporary[12u + index * 2u] = high;
    temporary[13u + index * 2u] = low;
    completed[3u + index * 2u] = high;
    completed[4u + index * 2u] = low;
  }
  temporary[44] = '.';
  temporary[45] = 't';
  temporary[46] = 'm';
  temporary[47] = 'p';
  temporary[48] = 0;
  static const char suffix[] = ".ckprof-part";
  for (uint32_t index = 0; index < sizeof(suffix); ++index) {
    completed[35u + index] = suffix[index];
  }
}

typedef struct CkProfileDirectoryIdentity {
  uint32_t length;
  dev_t device;
  uint64_t inode;
} CkProfileDirectoryIdentity;

_Static_assert(sizeof(dev_t) == sizeof(uint32_t),
               "Darwin dev_t layout changed");
_Static_assert(sizeof(CkProfileDirectoryIdentity) == 16u,
               "Darwin attribute identity layout changed");

static int ck_profile_matches_directory_identity(int directory_fd,
                                                 uint64_t expected_device,
                                                 uint64_t expected_inode) {
  struct attrlist attributes = {0};
  attributes.bitmapcount = ATTR_BIT_MAP_COUNT;
  attributes.commonattr = ATTR_CMN_DEVID | ATTR_CMN_FILEID;
  CkProfileDirectoryIdentity identity = {0};
  if (fgetattrlist(directory_fd, &attributes, &identity, sizeof(identity), 0u) !=
      0) {
    return 0;
  }
  return identity.length == sizeof(identity) &&
         (uint64_t)identity.device == expected_device &&
         identity.inode == expected_inode;
}

static int32_t __ck_profile_platform_publish(
    const uint8_t *directory, uint32_t directory_length,
    uint64_t identity_first, uint64_t identity_second,
    const uint8_t run_id[16], const uint8_t *bytes, uint64_t length) {
  const int directory_fd =
      ck_profile_open_directory(directory, directory_length);
  if (directory_fd < 0) {
    return CKC_PROFILE_PLATFORM_OPEN_ERROR;
  }
  if (!ck_profile_matches_directory_identity(directory_fd, identity_first,
                                             identity_second)) {
    close(directory_fd);
    return CKC_PROFILE_PLATFORM_IDENTITY_ERROR;
  }
  char temporary[49];
  char completed[53];
  ck_profile_names(run_id, temporary, completed);
  const int file = openat(directory_fd, temporary,
                          O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC,
                          0600);
  if (file < 0) {
    const int existing =
        openat(directory_fd, temporary, O_RDONLY | O_NOFOLLOW | O_CLOEXEC);
    const int32_t failure = existing >= 0 ? CKC_PROFILE_PLATFORM_COLLISION
                                         : CKC_PROFILE_PLATFORM_CREATE_ERROR;
    if (existing >= 0) {
      (void)close(existing);
    }
    close(directory_fd);
    return failure;
  }
  uint64_t offset = 0;
  int32_t failure = CKC_PROFILE_PLATFORM_OK;
  while (offset < length) {
    const size_t request =
        length - offset > (uint64_t)SIZE_MAX ? SIZE_MAX : (size_t)(length - offset);
    const ssize_t written = write(file, bytes + offset, request);
    if (written <= 0) {
      failure = CKC_PROFILE_PLATFORM_WRITE_ERROR;
      break;
    }
    offset += (uint64_t)written;
  }
  if (failure == CKC_PROFILE_PLATFORM_OK && fsync(file) != 0) {
    failure = CKC_PROFILE_PLATFORM_FILE_SYNC_ERROR;
  }
  if (close(file) != 0) {
    failure = CKC_PROFILE_PLATFORM_WRITE_ERROR;
  }
  if (failure == CKC_PROFILE_PLATFORM_OK &&
      renameatx_np(directory_fd, temporary, directory_fd, completed,
                   RENAME_EXCL) != 0) {
    failure = CKC_PROFILE_PLATFORM_RENAME_ERROR;
  }
  if (failure == CKC_PROFILE_PLATFORM_OK && fsync(directory_fd) != 0) {
    failure = CKC_PROFILE_PLATFORM_DIRECTORY_SYNC_ERROR;
  }
  if (failure != CKC_PROFILE_PLATFORM_OK) {
    (void)unlinkat(directory_fd, temporary, 0);
  }
  close(directory_fd);
  return failure;
}
