/* Separate bounded experiment, never linked into ckc or its acceptance harness. */
#define _GNU_SOURCE
#if !defined(__linux__) || !defined(__aarch64__)
#error "The checked-layout comparison requires Linux/AArch64."
#endif
#include <dlfcn.h>
#include <errno.h>
#include <inttypes.h>
#include <linux/perf_event.h>
#include <sched.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/resource.h>
#include <sys/syscall.h>
#include <time.h>
#include <unistd.h>

enum { LENGTH = 4000, CALLS = 5000, CHANNELS = 3, LAYOUTS = 4, ROWS = 1716 };
typedef int32_t (*Kernel)(uint32_t *, uint32_t, uint32_t *, uint32_t);
/* These complete bodies contain only internal relative branches, no relocations. */
static const uint32_t bodies[CHANNELS][14] = {
    {0xaa1f03e8,0xb8687809,0x31003529,0x540000e2,0xb8287849,0x91000508,
     0xf13e811f,0x54ffff41,0x2a1f03e0,0xd65f03c0,0x52800020,0xd65f03c0},
    {0xaa1f03e8,0xb8687809,0x31003529,0xb8287849,0x540000c2,0x91000508,
     0xf13e811f,0x54ffff41,0x2a1f03e0,0xd65f03c0,0x52800020,0xd65f03c0},
    {0xaa1f03e8,0xb8687809,0x3100393f,0x54000128,0x9100050a,0x11003529,
     0xb8287849,0xf13e815f,0xaa0a03e8,0x54ffff01,0x2a1f03e0,0xd65f03c0,
     0x52800020,0xd65f03c0}
};
static const size_t body_bytes[CHANNELS] = {48, 48, 56};
static const size_t offsets[3] = {0, 136, 284}; /* 0, 8, 28 modulo 64 */

struct Counts { uint64_t nr, enabled, running, values[3]; };
struct Row {
    int sequence, warmup, round, repetition, layout, channel, cpu_before, cpu_after;
    uint64_t cpu_ns, wall_ns, input_digest, output_digest;
    struct rusage before, after;
    struct Counts pmu_before, pmu_after;
    int pmu_error;
};

static void fail(const char *message) {
    fprintf(stderr, "%s: %s\n", message, strerror(errno));
    exit(2);
}

static uint64_t nanos(clockid_t clock) {
    struct timespec value;
    if (clock_gettime(clock, &value) || value.tv_sec < 0 || value.tv_nsec < 0 ||
        value.tv_nsec >= 1000000000) fail("comparison clock");
    return (uint64_t)value.tv_sec * UINT64_C(1000000000) + (uint64_t)value.tv_nsec;
}

static uint32_t input_at(size_t index) {
    return ((uint32_t)(index + 7) * UINT32_C(2654435761)) % 1000002 + 1;
}

static uint64_t digest(const uint32_t *values) {
    uint64_t value = UINT64_C(14695981039346656037);
    for (size_t i = 0; i < LENGTH; ++i)
        for (unsigned byte = 0; byte < 4; ++byte)
            value = (value ^ ((values[i] >> (8 * byte)) & 255)) * UINT64_C(1099511628211);
    return value;
}

static Kernel as_kernel(void *address) {
    Kernel function;
    _Static_assert(sizeof(function) == sizeof(address), "POSIX function pointer representation");
    memcpy(&function, &address, sizeof(function));
    return function;
}

static unsigned verify_copies(Kernel functions[LAYOUTS][CHANNELS], uint32_t *source) {
    uint32_t reference[LENGTH], actual[LENGTH];
    const int positions[] = {-2, -1, 0, 1, 1999, 3999};
    unsigned checks = 0;
    for (int channel = 0; channel < CHANNELS; ++channel) {
        for (size_t test = 0; test < sizeof(positions) / sizeof(positions[0]); ++test) {
            int position = positions[test];
            for (size_t i = 0; i < LENGTH; ++i) {
                source[i] = position == -2 ? UINT32_MAX - 13 : input_at(i);
                reference[i] = UINT32_C(0x5a5a5a5a);
            }
            if (position >= 0) source[position] = UINT32_MAX;
            uint64_t input = digest(source);
            int32_t expected = functions[0][channel](source, LENGTH, reference, LENGTH);
            if (expected != (position >= 0) || digest(source) != input)
                fail("original kernel semantic boundary");
            for (int layout = 1; layout < LAYOUTS; ++layout) {
                for (size_t i = 0; i < LENGTH; ++i) actual[i] = UINT32_C(0x5a5a5a5a);
                int32_t result = functions[layout][channel](source, LENGTH, actual, LENGTH);
                if (result != expected || memcmp(reference, actual, sizeof(actual)) ||
                    digest(source) != input) fail("copied kernel semantic boundary");
                ++checks;
            }
        }
    }
    return checks;
}

static int open_pmu(int descriptors[3]) {
    const uint64_t events[] = {PERF_COUNT_HW_CPU_CYCLES, PERF_COUNT_HW_INSTRUCTIONS,
                              PERF_COUNT_HW_BRANCH_MISSES};
    for (int i = 0; i < 3; ++i) {
        struct perf_event_attr attribute = {0};
        attribute.type = PERF_TYPE_HARDWARE;
        attribute.size = sizeof(attribute);
        attribute.config = events[i];
        attribute.exclude_kernel = 1;
        attribute.exclude_hv = 1;
        attribute.pinned = i == 0;
        attribute.read_format = PERF_FORMAT_GROUP | PERF_FORMAT_TOTAL_TIME_ENABLED |
                                PERF_FORMAT_TOTAL_TIME_RUNNING;
        descriptors[i] = (int)syscall(SYS_perf_event_open, &attribute, 0, -1,
                                     i ? descriptors[0] : -1, PERF_FLAG_FD_CLOEXEC);
        if (descriptors[i] < 0) {
            int error = errno;
            for (int j = 0; j < i; ++j) close(descriptors[j]);
            for (int j = 0; j < 3; ++j) descriptors[j] = -1;
            return error;
        }
    }
    return 0;
}

static int read_pmu(int descriptor, struct Counts *value) {
    if (descriptor < 0) return ENOTSUP;
    ssize_t bytes = read(descriptor, value, sizeof(*value));
    if (bytes < 0) return errno;
    return bytes == (ssize_t)sizeof(*value) && value->nr == 3 ? 0 : EIO;
}

/* One unchanged indirect-call body for all layouts/channels; no I/O inside it. */
__attribute__((noinline))
static void batch(Kernel function, uint32_t *source, uint32_t *output,
                  struct Row *row, int pmu_descriptor) {
    row->cpu_before = sched_getcpu();
    if (getrusage(RUSAGE_THREAD, &row->before)) fail("thread resource snapshot");
    int before_error = read_pmu(pmu_descriptor, &row->pmu_before);
    uint64_t wall_start = nanos(CLOCK_MONOTONIC_RAW);
    uint64_t cpu_start = nanos(CLOCK_THREAD_CPUTIME_ID);
    for (size_t call = 0; call < CALLS; ++call)
        if (function(source, LENGTH, output, LENGTH) != 0) fail("timed kernel result");
    uint64_t cpu_end = nanos(CLOCK_THREAD_CPUTIME_ID);
    uint64_t wall_end = nanos(CLOCK_MONOTONIC_RAW);
    int after_error = read_pmu(pmu_descriptor, &row->pmu_after);
    row->pmu_error = before_error ? before_error : after_error;
    if (getrusage(RUSAGE_THREAD, &row->after)) fail("thread resource snapshot");
    row->cpu_after = sched_getcpu();
    if (cpu_end <= cpu_start || wall_end <= wall_start) fail("nonpositive comparison interval");
    row->cpu_ns = cpu_end - cpu_start;
    row->wall_ns = wall_end - wall_start;
    for (size_t i = 0; i < LENGTH; ++i)
        if (source[i] != input_at(i) || output[i] != input_at(i) + 13)
            fail("timed kernel changed input/output");
    row->input_digest = digest(source);
    row->output_digest = digest(output);
}

static void print_counts(const struct Counts *counts) {
    printf("[%" PRIu64 ",%" PRIu64 ",%" PRIu64 ",%" PRIu64 ",%" PRIu64 "]",
           counts->enabled, counts->running, counts->values[0], counts->values[1], counts->values[2]);
}

static void print_row(const struct Row *row) {
    printf("{\"type\":\"batch\",\"sequence\":%d,\"warmup\":%s,\"round\":%d,"
           "\"repetition\":%d,\"layout\":%d,\"channel\":%d,\"calls\":%d,\"elements\":%d,"
           "\"threadCpuNs\":%" PRIu64 ",\"wallNs\":%" PRIu64 ",\"cpuBefore\":%d,\"cpuAfter\":%d,"
           "\"inputDigest\":\"%016" PRIx64 "\",\"outputDigest\":\"%016" PRIx64 "\","
           "\"minorFaults\":%ld,\"majorFaults\":%ld,\"voluntarySwitches\":%ld,\"involuntarySwitches\":%ld,"
           "\"pmuReadError\":%d,\"pmuRaw\":",
           row->sequence, row->warmup ? "true" : "false", row->round, row->repetition,
           row->layout, row->channel, CALLS, CALLS * LENGTH, row->cpu_ns, row->wall_ns,
           row->cpu_before, row->cpu_after, row->input_digest, row->output_digest,
           row->after.ru_minflt - row->before.ru_minflt, row->after.ru_majflt - row->before.ru_majflt,
           row->after.ru_nvcsw - row->before.ru_nvcsw, row->after.ru_nivcsw - row->before.ru_nivcsw,
           row->pmu_error);
    bool monotonic = !row->pmu_error && row->pmu_after.enabled >= row->pmu_before.enabled &&
                     row->pmu_after.running >= row->pmu_before.running;
    for (int i = 0; i < 3; ++i)
        monotonic = monotonic && row->pmu_after.values[i] >= row->pmu_before.values[i];
    if (row->pmu_error) printf("null");
    else {
        printf("["); print_counts(&row->pmu_before); printf(",");
        print_counts(&row->pmu_after); printf("]");
    }
    printf(",\"pmu\":");
    if (monotonic && row->pmu_after.enabled - row->pmu_before.enabled > 0 &&
        row->pmu_after.enabled - row->pmu_before.enabled == row->pmu_after.running - row->pmu_before.running &&
        row->pmu_after.values[0] > row->pmu_before.values[0] && row->pmu_after.values[1] > row->pmu_before.values[1]) {
        printf("{\"enabled\":%" PRIu64 ",\"running\":%" PRIu64 ",\"cycles\":%" PRIu64
               ",\"instructions\":%" PRIu64 ",\"branchMisses\":%" PRIu64 "}",
               row->pmu_after.enabled - row->pmu_before.enabled, row->pmu_after.running - row->pmu_before.running,
               row->pmu_after.values[0] - row->pmu_before.values[0], row->pmu_after.values[1] - row->pmu_before.values[1],
               row->pmu_after.values[2] - row->pmu_before.values[2]);
    } else printf("null");
    puts("}");
}

int main(int argc, char **argv) {
    if (argc != 5 || (strcmp(argv[1], "--self-test") && strcmp(argv[1], "--measure"))) return 64;
    bool self_test = strcmp(argv[1], "--self-test") == 0;
    void *libraries[CHANNELS], *entries[LAYOUTS][CHANNELS], *copies[CHANNELS];
    Kernel functions[LAYOUTS][CHANNELS];
    long page_size = sysconf(_SC_PAGESIZE);
    if (page_size < 4096 || page_size % 64) fail("comparison page size");
    for (int channel = 0; channel < CHANNELS; ++channel) {
        libraries[channel] = dlopen(argv[channel + 2], RTLD_NOW | RTLD_LOCAL);
        if (!libraries[channel]) { fprintf(stderr, "%s\n", dlerror()); return 65; }
        entries[0][channel] = dlsym(libraries[channel], channel == 0 ? "kernel" : "ck_oracle_kernel");
        if (!entries[0][channel]) return 66;
        if (memcmp(entries[0][channel], bodies[channel], body_bytes[channel])) {
            fprintf(stderr, "unrecognized complete instruction body for channel %d\n", channel);
            return 77;
        }
        functions[0][channel] = as_kernel(entries[0][channel]);
    }
    for (int channel = 0; channel < CHANNELS; ++channel) {
        copies[channel] = mmap(NULL, (size_t)page_size, PROT_READ | PROT_WRITE,
                               MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
        if (copies[channel] == MAP_FAILED) fail("code-copy mapping");
        for (int layout = 1; layout < LAYOUTS; ++layout) {
            entries[layout][channel] = (char *)copies[channel] + offsets[layout - 1];
            memcpy(entries[layout][channel], entries[0][channel], body_bytes[channel]);
            __builtin___clear_cache(entries[layout][channel], (char *)entries[layout][channel] + body_bytes[channel]);
            functions[layout][channel] = as_kernel(entries[layout][channel]);
        }
        if (mprotect(copies[channel], (size_t)page_size, PROT_READ | PROT_EXEC)) fail("read-execute code mapping");
        for (int layout = 1; layout < LAYOUTS; ++layout)
            if (memcmp(entries[layout][channel], entries[0][channel], body_bytes[channel])) fail("copy bytes changed");
    }
    cpu_set_t previous, pinned;
    int cpu = sched_getcpu();
    if (sched_getaffinity(0, sizeof(previous), &previous) || cpu < 0 || cpu >= CPU_SETSIZE ||
        !CPU_ISSET(cpu, &previous)) fail("read comparison CPU affinity");
    CPU_ZERO(&pinned); CPU_SET(cpu, &pinned);
    if (sched_setaffinity(0, sizeof(pinned), &pinned)) fail("pin comparison CPU");
    uint32_t *source = malloc(LENGTH * sizeof(uint32_t)), *output = calloc(LENGTH, sizeof(uint32_t));
    struct Row *rows = calloc(ROWS, sizeof(*rows));
    if (!source || !output || !rows) fail("comparison workspace");
    unsigned checks = verify_copies(functions, source);
    for (size_t i = 0; i < LENGTH; ++i) source[i] = input_at(i);
    int descriptors[3] = {-1, -1, -1};
    int pmu_error = self_test ? ENOTSUP : open_pmu(descriptors);
    printf("{\"type\":\"identity\",\"pid\":%d,\"copiesVerified\":true,\"semanticChecks\":%u,"
           "\"inputAddress\":%" PRIuPTR ",\"outputAddress\":%" PRIuPTR ",\"cpu\":%d,"
           "\"pmuOpenError\":%d,\"pmuUserOnly\":true,\"entries\":[",
           getpid(), checks, (uintptr_t)source, (uintptr_t)output, cpu, pmu_error);
    for (int layout = 0; layout < LAYOUTS; ++layout) {
        printf("%s[", layout ? "," : "");
        for (int channel = 0; channel < CHANNELS; ++channel)
            printf("%s%" PRIuPTR, channel ? "," : "", (uintptr_t)entries[layout][channel]);
        printf("]");
    }
    puts("]}");
    fflush(stdout);
    FILE *maps = fopen("/proc/self/maps", "r");
    if (!maps) fail("read comparison mappings");
    char line[4096];
    while (fgets(line, sizeof(line), maps)) fputs(line, stderr);
    fclose(maps);
    size_t count = 0;
    if (!self_test) {
        for (int phase = 0; phase < 2; ++phase)
            for (int round = 0; round < (phase ? 20 : 3); ++round)
                for (int repetition = 0; repetition < (phase ? 7 : 1); ++repetition)
                    for (int l = 0; l < LAYOUTS; ++l)
                        for (int c = 0; c < CHANNELS; ++c) {
                            if (count >= ROWS) fail("comparison row bound");
                            struct Row *row = &rows[count];
                            row->sequence = (int)count++; row->warmup = !phase; row->round = round;
                            row->repetition = repetition; row->layout = (round + repetition + l) % LAYOUTS;
                            row->channel = (round + repetition + c) % CHANNELS;
                            batch(functions[row->layout][row->channel], source, output, row, descriptors[0]);
                        }
        if (count != ROWS) fail("comparison incomplete");
        for (size_t i = 0; i < count; ++i) print_row(&rows[i]);
    }
    for (int i = 0; i < 3; ++i) if (descriptors[i] >= 0) close(descriptors[i]);
    if (sched_setaffinity(0, sizeof(previous), &previous)) fail("restore comparison CPU affinity");
    for (int channel = 0; channel < CHANNELS; ++channel) {
        munmap(copies[channel], (size_t)page_size);
        dlclose(libraries[channel]);
    }
    free(rows); free(output); free(source);
    return 0;
}
