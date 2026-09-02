#ifndef E2E_FUZZ_PROTO_WRITER_H
#define E2E_FUZZ_PROTO_WRITER_H

/*
 * A hand-rolled protobuf writer, used to build the fuzzer's seed messages
 * and the canonical ClientHello the target hands the library.
 *
 * Deliberately not protobuf-c: the point of a fuzzer is to emit encodings a
 * generated writer would refuse to produce, so the wire format is written
 * out by hand and the mutator is free to corrupt it afterwards.  Callers
 * are responsible for the buffer being large enough; every use here writes
 * into a fixed buffer with a generous margin.
 */

#include <stdint.h>
#include <string.h>

static inline size_t pw_varint(uint8_t *out, uint64_t value)
{
    size_t n = 0;
    do {
        out[n] = (uint8_t)(value & 0x7F);
        value >>= 7;
        if (value)
            out[n] |= 0x80;
        ++n;
    } while (value);
    return n;
}

/* field: varint */
static inline size_t pw_field(uint8_t *out, uint32_t field, uint64_t value)
{
    size_t n = pw_varint(out, ((uint64_t)field << 3) | 0);
    return n + pw_varint(out + n, value);
}

/* field: length-delimited (bytes, string, or a nested message) */
static inline size_t pw_bytes(uint8_t *out, uint32_t field,
                              const void *data, size_t len)
{
    size_t n = pw_varint(out, ((uint64_t)field << 3) | 2);
    n += pw_varint(out + n, len);
    if (len)
        memcpy(out + n, data, len);
    return n + len;
}

static inline size_t pw_string(uint8_t *out, uint32_t field, const char *text)
{
    return pw_bytes(out, field, text, strlen(text));
}

#endif /* E2E_FUZZ_PROTO_WRITER_H */
