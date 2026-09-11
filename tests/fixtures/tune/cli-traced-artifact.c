#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>

int main(void) {
    const char *mode = getenv("CK_FIXTURE_MODE");
    if (mode && strcmp(mode, "exit") == 0) return 7;
    if (mode && strcmp(mode, "signal") == 0) {
        const struct rlimit limit = {0, 0};
        if (setrlimit(RLIMIT_CORE, &limit) != 0) return 8;
        raise(SIGTERM);
        return 9;
    }
    const char *path = getenv("CK_FIXTURE_TRACE");
    if (path) {
        FILE *trace = fopen(path, "ab");
        if (!trace) return 10;
        if (fputc('x', trace) == EOF) {
            fclose(trace);
            return 11;
        }
        if (fclose(trace) != 0) return 12;
    }
    if (mode && strcmp(mode, "empty-output") == 0) return 0;
    const char *output = mode && strcmp(mode, "wrong-output") == 0 ? "67\n" : "66\n";
    if (mode && strcmp(mode, "extra-output") == 0) output = "66\nextra";
    if (fputs(output, stdout) == EOF) {
        return 13;
    }
    return 0;
}
