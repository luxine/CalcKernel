#define _POSIX_C_SOURCE 200809L
#define _DEFAULT_SOURCE
#define _DARWIN_C_SOURCE

#include <errno.h>
#include <inttypes.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/resource.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

/* This separate, failure-only comparator is never a CKTUNE/1 runner. It
 * observes real children after the original test has already failed. The Rust
 * caller owns a fresh process group and a deadline, including error cleanup. */
static uint64_t monotonic_ns(void) {
    struct timespec now;
    if (clock_gettime(CLOCK_MONOTONIC, &now) != 0 || now.tv_sec < 0) {
        fputs("diagnostic monotonic clock failed\n", stderr);
        exit(70);
    }
    return (uint64_t)now.tv_sec * UINT64_C(1000000000) + (uint64_t)now.tv_nsec;
}

static uint64_t cpu_ns(struct timeval value) {
    return (uint64_t)value.tv_sec * UINT64_C(1000000000)
        + (uint64_t)value.tv_usec * UINT64_C(1000);
}

static int parse_iterations(const char *text, unsigned *value) {
    if (!text || !*text || text[0] == '0') return 0;
    *value = 0;
    for (const char *cursor = text; *cursor; ++cursor) {
        if (*cursor < '0' || *cursor > '9') return 0;
        *value = *value * 10 + (unsigned)(*cursor - '0');
        if (*value > 16) return 0;
    }
    return *value > 0;
}

static int observe_artifact(const char *artifact, unsigned iteration) {
    int channel[2];
    if (pipe(channel) != 0) return 0;
    const uint64_t start = monotonic_ns();
    const pid_t child = fork();
    if (child < 0) {
        close(channel[0]);
        close(channel[1]);
        return 0;
    }
    if (child == 0) {
        close(channel[0]);
        if (channel[1] != STDOUT_FILENO) {
            if (dup2(channel[1], STDOUT_FILENO) < 0) _exit(126);
            close(channel[1]);
        }
        execl(artifact, artifact, (char *)NULL);
        _exit(127);
    }
    const uint64_t fork_return = monotonic_ns() - start;
    close(channel[1]);
    const char expected[] = "66\n";
    uint64_t received = 0;
    uint64_t first_byte = 0;
    int valid = 1;
    for (;;) {
        char buffer[64];
        const ssize_t count = read(channel[0], buffer, sizeof(buffer));
        if (count == 0) break;
        if (count < 0) {
            if (errno == EINTR) continue;
            valid = 0;
            kill(child, SIGKILL);
            break;
        }
        if (first_byte == 0) first_byte = monotonic_ns() - start;
        for (ssize_t index = 0; index < count; ++index) {
            if (received >= sizeof(expected) - 1 || buffer[index] != expected[received]) {
                valid = 0;
            }
            ++received;
        }
    }
    const uint64_t eof = monotonic_ns() - start;
    close(channel[0]);
    int status = -1;
    struct rusage usage = {0};
    pid_t waited;
    do {
        waited = wait4(child, &status, 0, &usage);
    } while (waited < 0 && errno == EINTR);
    const uint64_t reaped = monotonic_ns() - start;
    valid = valid && received == sizeof(expected) - 1 && waited == child
        && WIFEXITED(status) && WEXITSTATUS(status) == 0;
    /* Parent-observed elapsed ns include scheduling/pipe observation, not pure
     * loader durations. A zero first-byte time means no stdout was observed. */
    if (printf("CKTUNE-CHILD/1\t%u\t%ld\t%ld\t%" PRIu64 "\t%" PRIu64
               "\t%" PRIu64 "\t%" PRIu64 "\t%" PRIu64 "\t%" PRIu64
               "\t%" PRIu64 "\t%d\t%d\n",
               iteration, (long)child, (long)getpgrp(), fork_return, first_byte,
               eof, reaped, cpu_ns(usage.ru_utime), cpu_ns(usage.ru_stime),
               received, valid, status) < 0 || fflush(stdout) != 0) return 0;
    return valid;
}

int main(int argc, char **argv) {
    unsigned iterations = 0;
    if (argc != 3 || argv[1][0] != '/' || !parse_iterations(argv[2], &iterations)) return 2;
    for (unsigned iteration = 0; iteration < iterations; ++iteration) {
        if (!observe_artifact(argv[1], iteration)) {
            fputs("diagnostic candidate execution/output failed\n", stderr);
            return 3;
        }
    }
    return 0;
}
