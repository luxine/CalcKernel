#define _POSIX_C_SOURCE 200809L

#include <errno.h>
#include <inttypes.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

static int parse_u64(const char *text, uint64_t *value) {
    if (!text || !*text || (text[0] == '0' && text[1] != '\0')) return 0;
    for (const char *cursor = text; *cursor; ++cursor) {
        if (*cursor < '0' || *cursor > '9') return 0;
    }
    errno = 0;
    char *end = NULL;
    const unsigned long long number = strtoull(text, &end, 10);
    if (errno != 0 || !end || *end != '\0' || number > UINT64_MAX) return 0;
    *value = (uint64_t)number;
    return 1;
}

/* One logical iteration is one real CK program execution. The parent observes
 * actual output and exit; no sleep or runner-supplied clock stands in for work. */
static int run_artifact(const char *artifact) {
    int channel[2];
    if (pipe(channel) != 0) return 0;
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
    close(channel[1]);
    const char expected[] = "66\n";
    size_t received = 0;
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
        for (ssize_t index = 0; index < count; ++index) {
            if (received < sizeof(expected) - 1) {
                if (buffer[index] != expected[received]) valid = 0;
                ++received;
            } else {
                valid = 0;
            }
        }
    }
    close(channel[0]);
    int status = 0;
    pid_t waited;
    do {
        waited = waitpid(child, &status, 0);
    } while (waited < 0 && errno == EINTR);
    return valid && received == sizeof(expected) - 1 && waited == child
        && WIFEXITED(status) && WEXITSTATUS(status) == 0;
}

int main(void) {
    const char *protocol = getenv("CK_TUNE_PROTOCOL");
    const char *kind = getenv("CK_TUNE_ARTIFACT_KIND");
    const char *artifact = getenv("CK_TUNE_ARTIFACT");
    const char *case_id = getenv("CK_TUNE_CASE");
    uint64_t seed = 0;
    uint64_t iterations = 0;
    if (!protocol || strcmp(protocol, "1") != 0 || !kind
        || strcmp(kind, "executable") != 0 || !artifact || artifact[0] != '/'
        || !case_id || !parse_u64(getenv("CK_TUNE_SEED"), &seed)
        || !parse_u64(getenv("CK_TUNE_ITERATIONS"), &iterations) || iterations == 0) {
        return 2;
    }
    if (!((strcmp(case_id, "search") == 0 && seed == 7)
          || (strcmp(case_id, "validation") == 0 && seed == 8))) return 2;

    uint64_t completed = 0;
    while (completed < iterations) {
        if (!run_artifact(artifact)) {
            fputs("fixture candidate execution/output failed\n", stderr);
            return 3;
        }
        ++completed;
    }
    /* SHA-256 of the only accepted result bytes, "66\n". Every actual child
     * result is checked above, including the final one, before this is emitted. */
    return printf("CKTUNE/1 %s %" PRIu64 " %" PRIu64 " %" PRIu64
                  " 8e37bed9dff3949ffd23ae638260dff869f5cc26e551f2a9e5e289a8888949fa\n",
                  case_id, seed, iterations, completed) < 0 ? 4 : 0;
}
