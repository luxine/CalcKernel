#define CK_LINUX_AT_FDCWD (-100)
#define CK_LINUX_O_RDONLY 0
#define CK_LINUX_O_WRONLY 1
#define CK_LINUX_O_CREAT 0100
#define CK_LINUX_O_EXCL 0200
#define CK_LINUX_O_DIRECTORY 00200000
#define CK_LINUX_O_NOFOLLOW 00400000
#define CK_LINUX_O_CLOEXEC 02000000
#define CK_LINUX_MAP_PRIVATE 2
#define CK_LINUX_MAP_ANONYMOUS 32
#define CK_LINUX_PROT_READ 1
#define CK_LINUX_PROT_WRITE 2
#define CK_LINUX_RENAME_NOREPLACE 1

#if defined(__x86_64__)
#define CK_SYS_WRITE 1
#define CK_SYS_MMAP 9
#define CK_SYS_CLOSE 3
#define CK_SYS_FSTAT 5
#define CK_SYS_FSYNC 74
#define CK_SYS_OPENAT 257
#define CK_SYS_UNLINKAT 263
#define CK_SYS_RENAMEAT2 316
#define CK_SYS_GETRANDOM 318
static long ck_linux_syscall6(long number, long first, long second, long third,
                              long fourth, long fifth, long sixth) {
  register long r10 __asm__("r10") = fourth;
  register long r8 __asm__("r8") = fifth;
  register long r9 __asm__("r9") = sixth;
  long result;
  __asm__ volatile("syscall"
                   : "=a"(result)
                   : "a"(number), "D"(first), "S"(second), "d"(third),
                     "r"(r10), "r"(r8), "r"(r9)
                   : "rcx", "r11", "memory");
  return result;
}
#elif defined(__aarch64__)
#define CK_SYS_WRITE 64
#define CK_SYS_MMAP 222
#define CK_SYS_CLOSE 57
#define CK_SYS_FSTAT 80
#define CK_SYS_FSYNC 82
#define CK_SYS_OPENAT 56
#define CK_SYS_UNLINKAT 35
#define CK_SYS_RENAMEAT2 276
#define CK_SYS_GETRANDOM 278
static long ck_linux_syscall6(long number, long first, long second, long third,
                              long fourth, long fifth, long sixth) {
  register long x8 __asm__("x8") = number;
  register long x0 __asm__("x0") = first;
  register long x1 __asm__("x1") = second;
  register long x2 __asm__("x2") = third;
  register long x3 __asm__("x3") = fourth;
  register long x4 __asm__("x4") = fifth;
  register long x5 __asm__("x5") = sixth;
  __asm__ volatile("svc #0"
                   : "+r"(x0)
                   : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5)
                   : "memory");
  return x0;
}
#else
#error unsupported Linux profile runtime architecture
#endif

typedef struct CkLinuxStatIdentity {
  uint64_t device;
  uint64_t inode;
  uint8_t remainder[240];
} CkLinuxStatIdentity;

static long ck_linux_call3(long number, long first, long second, long third) {
  return ck_linux_syscall6(number, first, second, third, 0, 0, 0);
}

static void *__ck_profile_platform_allocate(uint64_t length) {
  if (length == 0u || length > (uint64_t)SIZE_MAX) {
    return (void *)0;
  }
  const long result = ck_linux_syscall6(
      CK_SYS_MMAP, 0, (long)length, CK_LINUX_PROT_READ | CK_LINUX_PROT_WRITE,
      CK_LINUX_MAP_PRIVATE | CK_LINUX_MAP_ANONYMOUS, -1, 0);
  return result < 0 ? (void *)0 : (void *)(uintptr_t)result;
}

static int32_t __ck_profile_platform_random(uint8_t output[16]) {
  uint32_t offset = 0;
  while (offset < 16u) {
    const long result = ck_linux_call3(CK_SYS_GETRANDOM,
                                       (long)(uintptr_t)(output + offset),
                                       (long)(16u - offset), 0);
    if (result <= 0) {
      return CKC_PROFILE_PLATFORM_ERROR;
    }
    offset += (uint32_t)result;
  }
  return CKC_PROFILE_PLATFORM_OK;
}

static int ck_profile_open_directory(const uint8_t *path, uint32_t length) {
  static const char root[] = "/";
  if (length < 2u || path[0] != '/' || path[length] != 0u) {
    return -1;
  }
  long current = ck_linux_syscall6(
      CK_SYS_OPENAT, CK_LINUX_AT_FDCWD, (long)(uintptr_t)root,
      CK_LINUX_O_RDONLY | CK_LINUX_O_DIRECTORY | CK_LINUX_O_NOFOLLOW |
          CK_LINUX_O_CLOEXEC,
      0, 0, 0);
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
        (void)ck_linux_call3(CK_SYS_CLOSE, current, 0, 0);
        return -1;
      }
      component[count++] = (char)path[offset++];
    }
    component[count] = 0;
    if ((count == 1u && component[0] == '.') ||
        (count == 2u && component[0] == '.' && component[1] == '.')) {
      (void)ck_linux_call3(CK_SYS_CLOSE, current, 0, 0);
      return -1;
    }
    const long next = ck_linux_syscall6(
        CK_SYS_OPENAT, current, (long)(uintptr_t)component,
        CK_LINUX_O_RDONLY | CK_LINUX_O_DIRECTORY | CK_LINUX_O_NOFOLLOW |
            CK_LINUX_O_CLOEXEC,
        0, 0, 0);
    (void)ck_linux_call3(CK_SYS_CLOSE, current, 0, 0);
    current = next;
  }
  return (int)current;
}

static char ck_profile_hex(uint8_t nibble) {
  if (nibble < 10u) {
    return (char)('0' + (int)nibble);
  }
  return (char)('a' + (int)nibble - 10);
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

static int32_t __ck_profile_platform_publish(
    const uint8_t *directory, uint32_t directory_length,
    uint64_t identity_first, uint64_t identity_second,
    const uint8_t run_id[16], const uint8_t *bytes, uint64_t length) {
  const int directory_fd =
      ck_profile_open_directory(directory, directory_length);
  if (directory_fd < 0) {
    return CKC_PROFILE_PLATFORM_ERROR;
  }
  CkLinuxStatIdentity metadata;
  if (ck_linux_call3(CK_SYS_FSTAT, directory_fd,
                     (long)(uintptr_t)&metadata, 0) < 0 ||
      metadata.device != identity_first || metadata.inode != identity_second) {
    (void)ck_linux_call3(CK_SYS_CLOSE, directory_fd, 0, 0);
    return CKC_PROFILE_PLATFORM_ERROR;
  }
  char temporary[49];
  char completed[53];
  ck_profile_names(run_id, temporary, completed);
  const long file = ck_linux_syscall6(
      CK_SYS_OPENAT, directory_fd, (long)(uintptr_t)temporary,
      CK_LINUX_O_WRONLY | CK_LINUX_O_CREAT | CK_LINUX_O_EXCL |
          CK_LINUX_O_NOFOLLOW | CK_LINUX_O_CLOEXEC,
      0600, 0, 0);
  if (file < 0) {
    (void)ck_linux_call3(CK_SYS_CLOSE, directory_fd, 0, 0);
    return CKC_PROFILE_PLATFORM_COLLISION;
  }
  uint64_t offset = 0;
  int failed = 0;
  while (offset < length) {
    const uint64_t remaining = length - offset;
    const long request =
        remaining > 0x7fffffffu ? 0x7fffffffu : (long)remaining;
    const long written = ck_linux_call3(
        CK_SYS_WRITE, file, (long)(uintptr_t)(bytes + offset), request);
    if (written <= 0) {
      failed = 1;
      break;
    }
    offset += (uint64_t)written;
  }
  if (!failed && ck_linux_call3(CK_SYS_FSYNC, file, 0, 0) < 0) {
    failed = 1;
  }
  if (ck_linux_call3(CK_SYS_CLOSE, file, 0, 0) < 0) {
    failed = 1;
  }
  if (!failed && ck_linux_syscall6(
                     CK_SYS_RENAMEAT2, directory_fd,
                     (long)(uintptr_t)temporary, directory_fd,
                     (long)(uintptr_t)completed, CK_LINUX_RENAME_NOREPLACE,
                     0) < 0) {
    failed = 1;
  }
  if (!failed && ck_linux_call3(CK_SYS_FSYNC, directory_fd, 0, 0) < 0) {
    failed = 1;
  }
  if (failed) {
    (void)ck_linux_call3(CK_SYS_UNLINKAT, directory_fd,
                         (long)(uintptr_t)temporary, 0);
  }
  (void)ck_linux_call3(CK_SYS_CLOSE, directory_fd, 0, 0);
  return failed ? CKC_PROFILE_PLATFORM_ERROR : CKC_PROFILE_PLATFORM_OK;
}
