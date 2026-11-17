/* Linux/glibc test fixture, loaded ONLY into the disposable initiator.
 * No Werewolf runtime changes. Interpose selected .stage52 lookups to
 * expose job lifetime and inject missing/blocked results after preflight.
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <netdb.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <time.h>
#include <unistd.h>

typedef int (*lookup_fn)(const char *, const char *, const struct addrinfo *, struct addrinfo **);
static lookup_fn real_lookup;
static pthread_once_t once = PTHREAD_ONCE_INIT;
static _Atomic int active;
static _Atomic unsigned long sequence;

static void initialise(void) {
    real_lookup = (lookup_fn)dlsym(RTLD_NEXT, "getaddrinfo");
}

static double now(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (double)t.tv_sec + (double)t.tv_nsec / 1000000000.0;
}

static void event(const char *kind, unsigned long id, const char *node, int count, int rc) {
    const char *path = getenv("STAGE52_RESOLVER_LOG");
    if (!path) return;
    int fd = open(path, O_WRONLY | O_CREAT | O_APPEND | O_CLOEXEC, 0600);
    if (fd < 0) return;
    char line[512];
    /* Fixture names contain only letters, digits and dots. */
    int n = snprintf(line, sizeof(line),
        "{\"event\":\"%s\",\"id\":%lu,\"name\":\"%.128s\","
        "\"active\":%d,\"rc\":%d,\"at\":%.6f,\"pid\":%ld,\"tid\":%ld}\n",
        kind, id, node, count, rc, now(), (long)getpid(), (long)syscall(SYS_gettid));
    if (n > 0 && (size_t)n < sizeof(line)) {
        ssize_t ignored = write(fd, line, (size_t)n);
        (void)ignored;
    }
    close(fd);
}

int getaddrinfo(const char *node, const char *service, const struct addrinfo *hints,
                struct addrinfo **result) {
    pthread_once(&once, initialise);
    if (!real_lookup) { errno = ENOSYS; return EAI_SYSTEM; }
    size_t n = node ? strlen(node) : 0;
    const char *path = getenv("STAGE52_RESOLVER_MODE");
    if (!path || n < 8 || strcmp(node + n - 8, ".stage52") != 0)
        return real_lookup(node, service, hints, result);
    int fd = open(path, O_RDONLY | O_CLOEXEC);
    if (fd < 0) { errno = ENOENT; return EAI_SYSTEM; }
    volatile unsigned char *mode = mmap(NULL, 4096, PROT_READ, MAP_SHARED, fd, 0);
    close(fd);
    if (mode == MAP_FAILED) return EAI_SYSTEM;
    unsigned long id = atomic_fetch_add(&sequence, 1) + 1;
    int count = atomic_fetch_add(&active, 1) + 1;
    event("start", id, node, count, 0);
    double started = now();
    const struct timespec pause = { .tv_sec = 0, .tv_nsec = 20000000 };
    while (*mode == 'b' && now() - started < 30.0) nanosleep(&pause, NULL);
    int rc;
    if (*mode == 'b') {
        event("guard", id, node, atomic_load(&active), EAI_AGAIN);
        *result = NULL;
        rc = EAI_AGAIN;
    } else if (*mode == 'n') {
        *result = NULL;
        rc = EAI_NONAME;
    } else {
        rc = real_lookup(node, service, hints, result);
    }
    munmap((void *)mode, 4096);
    count = atomic_fetch_sub(&active, 1) - 1;
    event("end", id, node, count, rc);
    return rc;
}
