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

#if defined(__APPLE__) && !defined(CK_TUNE_DIAGNOSTIC_NO_RUSAGE)
#include <libproc.h>
#include <mach/mach_time.h>
#ifndef CK_TUNE_DIAGNOSTIC_RUSAGE_FLAVOR
#define CK_TUNE_DIAGNOSTIC_RUSAGE_FLAVOR RUSAGE_INFO_V4
#endif
static mach_timebase_info_data_t resource_timebase;
#endif

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

struct resource_observation {
    int exit_result;
    int exit_error;
    uint64_t exit_observed_ns;
    int usage_result;
    int usage_error;
    uint64_t queried_ns;
    uint32_t timebase_numer;
    uint32_t timebase_denom;
    uint64_t counters[10];
    unsigned char uuid[16];
};

static struct resource_observation observe_resources(pid_t child, uint64_t start) {
    struct resource_observation result = {
        .exit_result = -1, .exit_error = ENOTSUP,
        .usage_result = -1, .usage_error = ENOTSUP,
    };
#if defined(__APPLE__) && !defined(CK_TUNE_DIAGNOSTIC_NO_RUSAGE)
    if (resource_timebase.numer == 0 || resource_timebase.denom == 0) return result;
    result.timebase_numer = resource_timebase.numer;
    result.timebase_denom = resource_timebase.denom;
    siginfo_t exited = {0};
    do {
        result.exit_result = waitid(P_PID, (id_t)child, &exited, WEXITED | WNOWAIT);
    } while (result.exit_result < 0 && errno == EINTR);
    result.exit_error = result.exit_result < 0 ? errno : 0;
    result.exit_observed_ns = monotonic_ns() - start;
    if (result.exit_result == 0 && exited.si_pid != child) {
        result.exit_result = -1;
        result.exit_error = ECHILD;
    }
    if (result.exit_result == 0) {
        struct rusage_info_v4 usage = {0};
        result.usage_result = proc_pid_rusage(child, CK_TUNE_DIAGNOSTIC_RUSAGE_FLAVOR, (rusage_info_t *)&usage);
        result.usage_error = result.usage_result < 0 ? errno : 0;
        if (result.usage_result == 0) {
            // These time fields are Mach ticks, not nanoseconds. Keep the raw
            // values and timebase; wait4's separate CPU fields remain below.
            const uint64_t counters[] = {
                usage.ri_user_time, usage.ri_system_time,
                usage.ri_instructions, usage.ri_cycles, usage.ri_pageins,
                usage.ri_runnable_time, usage.ri_child_user_time,
                usage.ri_child_system_time, usage.ri_proc_start_abstime,
                usage.ri_proc_exit_abstime,
            };
            for (unsigned i = 0; i < 10; ++i) result.counters[i] = counters[i];
            for (unsigned i = 0; i < sizeof(result.uuid); ++i) result.uuid[i] = usage.ri_uuid[i];
        }
    } else {
        result.usage_error = result.exit_error;
    }
    result.queried_ns = monotonic_ns() - start;
#else
    (void)child;
    (void)start;
#endif
    return result;
}

static int write_resources(unsigned iteration, pid_t child, const struct resource_observation *row) {
    if (printf("CKTUNE-RESOURCES/1\t%u\t%ld\t%d\t%d\t%" PRIu64
               "\t%d\t%d\t%" PRIu64 "\t%" PRIu32 "\t%" PRIu32,
               iteration, (long)child, row->exit_result, row->exit_error,
               row->exit_observed_ns, row->usage_result, row->usage_error,
               row->queried_ns, row->timebase_numer, row->timebase_denom) < 0) return 0;
    for (unsigned i = 0; i < 10; ++i) {
        if (printf("\t%" PRIu64, row->counters[i]) < 0) return 0;
    }
    if (putchar('\t') == EOF) return 0;
    for (unsigned i = 0; i < sizeof(row->uuid); ++i) {
        if (printf("%02x", (unsigned)row->uuid[i]) < 0) return 0;
    }
    return putchar('\n') != EOF && fflush(stdout) == 0;
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
    // Observe without reaping so the PID still identifies this exact child.
    // Query failures affect only metadata, never real output/exit validity.
    const struct resource_observation resources = observe_resources(child, start);
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
    if (!write_resources(iteration, child, &resources)) return 0;
    return valid;
}

int main(int argc, char **argv) {
    unsigned iterations = 0;
    if (argc != 3 || argv[1][0] != '/' || !parse_iterations(argv[2], &iterations)) return 2;
#if defined(__APPLE__) && !defined(CK_TUNE_DIAGNOSTIC_NO_RUSAGE)
    if (mach_timebase_info(&resource_timebase) != KERN_SUCCESS) {
        resource_timebase.numer = 0;
        resource_timebase.denom = 0;
    }
#endif
    for (unsigned iteration = 0; iteration < iterations; ++iteration) {
        if (!observe_artifact(argv[1], iteration)) {
            fputs("diagnostic candidate execution/output failed\n", stderr);
            return 3;
        }
    }
    return 0;
}
