/*
 * The built-in fuzzing driver for fuzz_target.c.
 *
 * libFuzzer and AFL++ both want clang; this driver exists so the fuzzer
 * runs anywhere the rest of the test suite does -- in CI, on a plain gcc
 * box, as one more stage of e2e/run.sh -- with no extra tooling.  It is a
 * generator, not a coverage-guided fuzzer: every input is built from a seed
 * corpus of valid protocol messages that it mutates and reframes, plus a
 * share of pure noise.  Everything derives from --seed, so a failing run
 * replays exactly.
 *
 * With clang, build fuzz_target.c with -fsanitize=fuzzer,address instead
 * and leave this file out; both drive the same LLVMFuzzerTestOneInput().
 *
 *   fuzz_receive --seconds 30 --seed 1234     generate and run
 *   fuzz_receive crash.bin                    replay saved inputs
 *   fuzz_receive --quiet --seconds 30         without the library's chatter
 *   fuzz_receive --dump-corpus DIR            write the seed corpus out
 *                                             (this is how run_libfuzzer.sh
 *                                             seeds libFuzzer's corpus)
 *
 * A crashing input is written to --artifact (default fuzz-crash.bin) before
 * the process dies, so it can be replayed under a debugger or a sanitizer.
 */

#include <fcntl.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#include "proto_writer.h"

int LLVMFuzzerTestOneInput(const uint8_t *data, size_t size);

#define MAX_INPUT (64 * 1024)

/* ------------------------------------------------------------------ */
/* deterministic randomness                                           */
/* ------------------------------------------------------------------ */

static uint64_t rng_state;

static uint64_t rng_next(void)
{
    /* splitmix64: tiny, decent, and reproducible across platforms. */
    uint64_t z = (rng_state += 0x9E3779B97F4A7C15ull);
    z = (z ^ (z >> 30)) * 0xBF58476D1CE4E5B9ull;
    z = (z ^ (z >> 27)) * 0x94D049BB133111EBull;
    return z ^ (z >> 31);
}

static uint32_t rng_below(uint32_t bound)
{
    return bound ? (uint32_t)(rng_next() % bound) : 0;
}

/* ------------------------------------------------------------------ */
/* seed corpus: one well-formed envelope of every client -> server kind */
/* ------------------------------------------------------------------ */

/* Envelope field numbers, from proto/remote_webgpu.proto. */
enum {
    F_ERROR = 3, F_CLIENT_HELLO = 2,
    F_PRESENT_DONE = 50, F_MAP_BUFFER_DATA = 51, F_EVENT = 52,
    F_ERROR_SCOPE_RESULT = 90, F_WORK_DONE = 91, F_COMPILATION_INFO = 92,
    F_UNCAPTURED_ERROR = 93, F_DEVICE_LOST = 94, F_TEXTURE_LOADED = 95,
};

typedef struct {
    uint8_t bytes[4096];
    size_t len;
} Message;

static Message corpus[16];
static size_t corpus_len;

static Message *corpus_add(void)
{
    Message *message = &corpus[corpus_len++];
    message->len = 0;
    return message;
}

static void build_corpus(void)
{
    uint8_t body[2048];
    size_t n;
    Message *m;

    /* ClientHello, well-formed. */
    {
        uint8_t limits[256], info[64], hello[512];
        size_t l = 0, i = 0, h = 0;
        l += pw_field(limits + l, 1, 8192);
        l += pw_field(limits + l, 2, 8192);
        l += pw_field(limits + l, 5, 4);
        l += pw_field(limits + l, 17, 256);
        l += pw_field(limits + l, 18, 256);
        l += pw_field(limits + l, 20, 268435456);
        l += pw_field(limits + l, 31, 65535);   /* max_compute_workgroups_per_dimension */
        i += pw_string(info + i, 1, "fuzz vendor");
        i += pw_string(info + i, 3, "fuzz device");
        h += pw_field(hello + h, 1, 6);
        h += pw_bytes(hello + h, 2, info, i);
        h += pw_field(hello + h, 3, 1280);
        h += pw_field(hello + h, 4, 720);
        h += pw_bytes(hello + h, 5, limits, l);
        for (int f = 0; f < 8; ++f)
            h += pw_field(hello + h, 6, (uint64_t)f);
        m = corpus_add();
        m->len = pw_bytes(m->bytes, F_CLIENT_HELLO, hello, h);
    }

    /* Event: canvas resize. */
    n = 0;
    n += pw_field(body + n, 1, 1920);
    n += pw_field(body + n, 2, 1080);
    m = corpus_add();
    {
        uint8_t event[64];
        size_t e = pw_bytes(event, 1, body, n);
        m->len = pw_bytes(m->bytes, F_EVENT, event, e);
    }

    /* Event: user-defined (name + payload). */
    n = 0;
    n += pw_string(body + n, 1, "keydown");
    n += pw_bytes(body + n, 2, "KeyW\nw\n1", 8);
    m = corpus_add();
    {
        uint8_t event[128];
        size_t e = pw_bytes(event, 2, body, n);
        m->len = pw_bytes(m->bytes, F_EVENT, event, e);
    }

    /* PresentDone (empty). */
    m = corpus_add();
    m->len = pw_bytes(m->bytes, F_PRESENT_DONE, NULL, 0);

    /* MapBufferData: a full 256-byte read map for request 1. */
    {
        uint8_t payload[256];
        memset(payload, 0xA5, sizeof payload);
        n = 0;
        n += pw_field(body + n, 1, 1);
        n += pw_bytes(body + n, 2, payload, sizeof payload);
        m = corpus_add();
        m->len = pw_bytes(m->bytes, F_MAP_BUFFER_DATA, body, n);
    }

    /* MapBufferData: a failed map. */
    n = 0;
    n += pw_field(body + n, 1, 1);
    n += pw_field(body + n, 3, 1);
    n += pw_string(body + n, 4, "denied");
    m = corpus_add();
    m->len = pw_bytes(m->bytes, F_MAP_BUFFER_DATA, body, n);

    /* WorkDone. */
    n = pw_field(body, 1, 2);
    m = corpus_add();
    m->len = pw_bytes(m->bytes, F_WORK_DONE, body, n);

    /* ErrorScopeResult. */
    n = 0;
    n += pw_field(body + n, 1, 3);
    n += pw_field(body + n, 2, 2);
    n += pw_string(body + n, 3, "validation failed");
    m = corpus_add();
    m->len = pw_bytes(m->bytes, F_ERROR_SCOPE_RESULT, body, n);

    /* CompilationInfoResult with two messages. */
    {
        uint8_t entry[256];
        size_t e = 0;
        e += pw_string(entry + e, 1, "expected ';'");
        e += pw_field(entry + e, 2, 1);
        e += pw_field(entry + e, 3, 12);
        e += pw_field(entry + e, 4, 4);
        n = 0;
        n += pw_field(body + n, 1, 4);
        n += pw_bytes(body + n, 2, entry, e);
        n += pw_bytes(body + n, 2, entry, e);
        m = corpus_add();
        m->len = pw_bytes(m->bytes, F_COMPILATION_INFO, body, n);
    }

    /* TextureLoaded. */
    n = 0;
    n += pw_field(body + n, 1, 5);
    n += pw_field(body + n, 4, 1024);
    n += pw_field(body + n, 5, 1024);
    m = corpus_add();
    m->len = pw_bytes(m->bytes, F_TEXTURE_LOADED, body, n);

    /* UncapturedError. */
    n = 0;
    n += pw_field(body + n, 1, 2);
    n += pw_string(body + n, 2, "fabricated validation error");
    m = corpus_add();
    m->len = pw_bytes(m->bytes, F_UNCAPTURED_ERROR, body, n);

    /* DeviceLost. */
    n = 0;
    n += pw_field(body + n, 1, 1);
    n += pw_string(body + n, 2, "fabricated device loss");
    m = corpus_add();
    m->len = pw_bytes(m->bytes, F_DEVICE_LOST, body, n);

    /* Error (a protocol error reported by the client). */
    n = pw_string(body, 1, "client says goodbye");
    m = corpus_add();
    m->len = pw_bytes(m->bytes, F_ERROR, body, n);
}

/* ------------------------------------------------------------------ */
/* mutation                                                           */
/* ------------------------------------------------------------------ */

static void mutate(uint8_t *buf, size_t *len, size_t capacity)
{
    if (*len == 0)
        return;
    switch (rng_below(8)) {
    case 0:  /* flip a bit */
        buf[rng_below((uint32_t)*len)] ^= (uint8_t)(1u << rng_below(8));
        break;
    case 1:  /* set a byte to an interesting value */
    {
        static const uint8_t interesting[] = { 0x00, 0x01, 0x7F, 0x80, 0xFF };
        buf[rng_below((uint32_t)*len)] = interesting[rng_below(sizeof interesting)];
        break;
    }
    case 2:  /* truncate */
        *len = rng_below((uint32_t)*len);
        break;
    case 3:  /* append noise */
    {
        size_t extra = rng_below(32);
        while (extra-- && *len < capacity)
            buf[(*len)++] = (uint8_t)rng_next();
        break;
    }
    case 4:  /* overwrite a run */
    {
        size_t pos = rng_below((uint32_t)*len);
        size_t run = rng_below((uint32_t)(*len - pos)) + 1;
        for (size_t i = 0; i < run; ++i)
            buf[pos + i] = (uint8_t)rng_next();
        break;
    }
    case 5:  /* splice a varint-sized field length to something absurd */
        buf[rng_below((uint32_t)*len)] = 0xFF;
        break;
    case 6:  /* duplicate the message onto itself */
    {
        size_t copy = *len < capacity - *len ? *len : capacity - *len;
        memcpy(buf + *len, buf, copy);
        *len += copy;
        break;
    }
    default: /* leave it alone: the unmutated seed is worth running too */
        break;
    }
}

/* Build one input: a mode byte, then framed (or deliberately misframed)
 * envelopes drawn from the corpus, mutated, or made of pure noise. */
static size_t generate(uint8_t *out, size_t capacity)
{
    size_t len = 0;
    out[len++] = (uint8_t)rng_next();          /* session mode */

    uint32_t envelopes = 1 + rng_below(8);
    for (uint32_t i = 0; i < envelopes && len + 8 < capacity; ++i) {
        uint8_t body[MAX_INPUT];
        size_t body_len;

        if (rng_below(10) == 0) {              /* pure noise */
            body_len = rng_below(128);
            for (size_t b = 0; b < body_len; ++b)
                body[b] = (uint8_t)rng_next();
        } else {                               /* a seed message, mutated */
            const Message *seed = &corpus[rng_below((uint32_t)corpus_len)];
            body_len = seed->len;
            memcpy(body, seed->bytes, body_len);
            uint32_t rounds = rng_below(4);
            for (uint32_t r = 0; r < rounds; ++r)
                mutate(body, &body_len, sizeof body);
        }
        if (body_len > capacity - len - 4)
            body_len = capacity - len - 4;

        /* The size prefix, occasionally a lie. */
        uint32_t prefix = (uint32_t)body_len;
        if (rng_below(16) == 0)
            prefix = rng_below(2) ? 0xFFFFFFF0u : (uint32_t)rng_next();
        out[len++] = (uint8_t)prefix;
        out[len++] = (uint8_t)(prefix >> 8);
        out[len++] = (uint8_t)(prefix >> 16);
        out[len++] = (uint8_t)(prefix >> 24);
        memcpy(out + len, body, body_len);
        len += body_len;
    }
    return len;
}

/* ------------------------------------------------------------------ */
/* crash artifacts                                                    */
/* ------------------------------------------------------------------ */

static uint8_t current_input[MAX_INPUT];
static volatile size_t current_len;
static const char *artifact_path = "fuzz-crash.bin";
static uint64_t current_seed;
static uint64_t iteration;

static void write_artifact(void)
{
    int fd = open(artifact_path, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (fd < 0)
        return;
    ssize_t written = write(fd, current_input, current_len);
    (void)written;
    close(fd);
}

static void on_crash(int signal_number)
{
    /* Async-signal-safe: a fixed message, then the input, then let the
     * default handler (or the sanitizer's) produce the real report. */
    static const char message[] = "\nfuzz: crashed; input written to the artifact file "
                                 "(replay it without --quiet to see why)\n";
    ssize_t written = write(1, message, sizeof message - 1);
    (void)written;
    write_artifact();
    signal(signal_number, SIG_DFL);
    raise(signal_number);
}

static void install_handlers(void)
{
    signal(SIGABRT, on_crash);
    signal(SIGSEGV, on_crash);
    signal(SIGBUS, on_crash);
    signal(SIGILL, on_crash);
    signal(SIGFPE, on_crash);
}

/* ------------------------------------------------------------------ */

/*
 * Write the seed messages out as ready-to-run inputs (mode byte + framing),
 * one file each, for a coverage-guided fuzzer to start from.
 */
static int dump_corpus(const char *dir)
{
    /*
     * Two files per message, one for each side of the mode byte: an
     * established session (where the interesting handlers live) and a bare
     * adapter still waiting for its hello (where the handshake itself,
     * and the clamping of everything a hello reports, lives).  A mutated
     * hello is only meaningful in the second.
     */
    for (size_t i = 0; i < corpus_len; ++i) {
        for (unsigned mode = 0; mode < 2; ++mode) {
            char path[512];
            snprintf(path, sizeof path, "%s/seed-%02zu-%s.bin", dir, i,
                     mode ? "handshake" : "session");
            FILE *file = fopen(path, "wb");
            if (!file) {
                fprintf(stdout, "fuzz: cannot write %s\n", path);
                return 1;
            }
            const Message *message = &corpus[i];
            uint8_t header[5] = {
                (uint8_t)mode,
                (uint8_t)message->len,
                (uint8_t)(message->len >> 8),
                (uint8_t)(message->len >> 16),
                (uint8_t)(message->len >> 24),
            };
            fwrite(header, 1, sizeof header, file);
            fwrite(message->bytes, 1, message->len, file);
            fclose(file);
        }
    }
    fprintf(stdout, "fuzz: wrote %zu seed inputs to %s\n", corpus_len * 2, dir);
    return 0;
}

static int run_file(const char *path)
{
    FILE *file = fopen(path, "rb");
    if (!file) {
        fprintf(stdout, "fuzz: cannot open %s\n", path);
        return 1;
    }
    current_len = fread(current_input, 1, sizeof current_input, file);
    fclose(file);
    fprintf(stdout, "fuzz: replaying %s (%zu bytes)\n", path, current_len);
    LLVMFuzzerTestOneInput(current_input, current_len);
    return 0;
}

int main(int argc, char **argv)
{
    double seconds = 10.0;
    uint64_t runs = 0;                 /* 0 = bounded by time instead */
    uint64_t seed = 1;
    const char *corpus_dir = NULL;
    int quiet = 0;
    int first_file = argc;

    for (int i = 1; i < argc; ++i) {
        if (!strcmp(argv[i], "--seconds") && i + 1 < argc)
            seconds = atof(argv[++i]);
        else if (!strcmp(argv[i], "--runs") && i + 1 < argc)
            runs = strtoull(argv[++i], NULL, 10);
        else if (!strcmp(argv[i], "--seed") && i + 1 < argc)
            seed = strtoull(argv[++i], NULL, 10);
        else if (!strcmp(argv[i], "--artifact") && i + 1 < argc)
            artifact_path = argv[++i];
        else if (!strcmp(argv[i], "--dump-corpus") && i + 1 < argc)
            corpus_dir = argv[++i];
        else if (!strcmp(argv[i], "--quiet"))
            quiet = 1;
        else if (argv[i][0] == '-') {
            fprintf(stderr,
                    "usage: %s [--seconds N] [--runs N] [--seed N] "
                    "[--artifact PATH] [--dump-corpus DIR] [--quiet] "
                    "[input files...]\n", argv[0]);
            return 2;
        } else {
            first_file = i;
            break;
        }
    }

    install_handlers();
    build_corpus();

    /*
     * The library narrates every rejected message on stderr, which at a few
     * hundred thousand inputs a second is its own denial of service.
     * --quiet drops it (libFuzzer's -close_fd_mask does the same); the
     * driver's own reporting goes to stdout and survives.
     */
    if (quiet && !freopen("/dev/null", "w", stderr))
        printf("fuzz: could not silence stderr\n");

    if (corpus_dir)
        return dump_corpus(corpus_dir);

    /* Replay mode: run the given inputs once each and stop. */
    if (first_file < argc) {
        int rc = 0;
        for (int i = first_file; i < argc; ++i)
            rc |= run_file(argv[i]);
        printf("fuzz: replayed %d input(s), all survived\n", argc - first_file);
        return rc;
    }

    current_seed = seed;
    rng_state = seed;
    printf("fuzz: seed %llu, %.1fs budget, %zu seed messages\n",
           (unsigned long long)seed, seconds, corpus_len);
    fflush(stdout);

    struct timespec start;
    clock_gettime(CLOCK_MONOTONIC, &start);
    double elapsed = 0.0;
    for (iteration = 0; runs ? iteration < runs : elapsed < seconds; ++iteration) {
        current_len = generate(current_input, sizeof current_input);
        LLVMFuzzerTestOneInput(current_input, current_len);
        if ((iteration & 0x3F) == 0) {
            struct timespec now;
            clock_gettime(CLOCK_MONOTONIC, &now);
            elapsed = (double)(now.tv_sec - start.tv_sec)
                    + (double)(now.tv_nsec - start.tv_nsec) / 1e9;
        }
    }

    printf("fuzz: %llu inputs, no crash (seed %llu)\n",
           (unsigned long long)iteration, (unsigned long long)current_seed);
    return 0;
}
