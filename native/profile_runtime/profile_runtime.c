#include "ckc_profile_runtime.h"
#include "ckc_profile_platform.h"

#include "common/sha256.c"
#if defined(__APPLE__)
#include "platform/darwin.c"
#elif defined(_WIN32)
#include "platform/windows.c"
#elif defined(__linux__)
#include "platform/linux.c"
#else
#error unsupported CK profile runtime platform
#endif
#include "common/collector.c"
