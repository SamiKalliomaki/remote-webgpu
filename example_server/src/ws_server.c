#include "ws_server.h"

#include <arpa/inet.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

/* ------------------------------------------------------------------ */
/* SHA-1 (RFC 3174) -- needed only for the websocket accept key        */
/* ------------------------------------------------------------------ */

static uint32_t rol32(uint32_t x, int n) { return (x << n) | (x >> (32 - n)); }

static void sha1(const unsigned char *data, size_t len, unsigned char out[20])
{
    uint32_t h[5] = { 0x67452301u, 0xEFCDAB89u, 0x98BADCFEu, 0x10325476u, 0xC3D2E1F0u };

    /* message + 0x80 + padding + 64-bit bit length, in 64-byte blocks */
    const uint64_t total_bits = (uint64_t)len * 8;
    size_t padded = ((len + 8) / 64 + 1) * 64;

    for (size_t block = 0; block < padded; block += 64) {
        unsigned char chunk[64];
        for (size_t i = 0; i < 64; ++i) {
            size_t pos = block + i;
            if (pos < len)
                chunk[i] = data[pos];
            else if (pos == len)
                chunk[i] = 0x80;
            else if (pos >= padded - 8)
                chunk[i] = (unsigned char)(total_bits >> (8 * (padded - 1 - pos)));
            else
                chunk[i] = 0;
        }

        uint32_t w[80];
        for (int i = 0; i < 16; ++i)
            w[i] = (uint32_t)chunk[4 * i] << 24 | (uint32_t)chunk[4 * i + 1] << 16
                 | (uint32_t)chunk[4 * i + 2] << 8 | chunk[4 * i + 3];
        for (int i = 16; i < 80; ++i)
            w[i] = rol32(w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16], 1);

        uint32_t a = h[0], b = h[1], c = h[2], d = h[3], e = h[4];
        for (int i = 0; i < 80; ++i) {
            uint32_t f, k;
            if (i < 20)      { f = (b & c) | (~b & d);           k = 0x5A827999u; }
            else if (i < 40) { f = b ^ c ^ d;                    k = 0x6ED9EBA1u; }
            else if (i < 60) { f = (b & c) | (b & d) | (c & d);  k = 0x8F1BBCDCu; }
            else             { f = b ^ c ^ d;                    k = 0xCA62C1D6u; }
            uint32_t t = rol32(a, 5) + f + e + k + w[i];
            e = d; d = c; c = rol32(b, 30); b = a; a = t;
        }
        h[0] += a; h[1] += b; h[2] += c; h[3] += d; h[4] += e;
    }

    for (int i = 0; i < 5; ++i) {
        out[4 * i]     = (unsigned char)(h[i] >> 24);
        out[4 * i + 1] = (unsigned char)(h[i] >> 16);
        out[4 * i + 2] = (unsigned char)(h[i] >> 8);
        out[4 * i + 3] = (unsigned char)h[i];
    }
}

static void base64(const unsigned char *in, size_t len, char *out)
{
    static const char tbl[] =
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    size_t o = 0;
    for (size_t i = 0; i < len; i += 3) {
        uint32_t v = (uint32_t)in[i] << 16;
        if (i + 1 < len) v |= (uint32_t)in[i + 1] << 8;
        if (i + 2 < len) v |= in[i + 2];
        out[o++] = tbl[(v >> 18) & 63];
        out[o++] = tbl[(v >> 12) & 63];
        out[o++] = i + 1 < len ? tbl[(v >> 6) & 63] : '=';
        out[o++] = i + 2 < len ? tbl[v & 63] : '=';
    }
    out[o] = '\0';
}

/* ------------------------------------------------------------------ */
/* handshake                                                          */
/* ------------------------------------------------------------------ */

/* Case-insensitive "Sec-WebSocket-Key" header lookup; key is base64, 24 chars. */
static int find_ws_key(const char *request, char key[64])
{
    static const char name[] = "sec-websocket-key:";
    for (const char *line = request; line; line = strchr(line, '\n'), line = line ? line + 1 : NULL) {
        if (strncasecmp(line, name, sizeof name - 1) != 0)
            continue;
        const char *p = line + sizeof name - 1;
        while (*p == ' ' || *p == '\t')
            ++p;
        size_t n = strcspn(p, " \t\r\n");
        if (n == 0 || n >= 64)
            return -1;
        memcpy(key, p, n);
        key[n] = '\0';
        return 0;
    }
    return -1;
}

static int do_handshake(int fd)
{
    char request[4096];
    size_t got = 0;

    /* Read until the end of the HTTP header block. */
    while (got < sizeof request - 1) {
        ssize_t n = read(fd, request + got, sizeof request - 1 - got);
        if (n <= 0) {
            fprintf(stderr, "ws_server: client hung up during handshake\n");
            return -1;
        }
        got += (size_t)n;
        request[got] = '\0';
        if (strstr(request, "\r\n\r\n"))
            break;
    }

    char key[64];
    if (find_ws_key(request, key) != 0) {
        fprintf(stderr, "ws_server: no Sec-WebSocket-Key in request\n");
        static const char bad[] = "HTTP/1.1 400 Bad Request\r\n\r\n";
        (void)!write(fd, bad, sizeof bad - 1);
        return -1;
    }

    /* accept = base64(sha1(key + magic GUID)) */
    char keyed[64 + 40];
    snprintf(keyed, sizeof keyed, "%s258EAFA5-E914-47DA-95CA-C5AB0DC85B11", key);
    unsigned char digest[20];
    sha1((const unsigned char *)keyed, strlen(keyed), digest);
    char accept_key[32];
    base64(digest, sizeof digest, accept_key);

    char response[256];
    int len = snprintf(response, sizeof response,
                       "HTTP/1.1 101 Switching Protocols\r\n"
                       "Upgrade: websocket\r\n"
                       "Connection: Upgrade\r\n"
                       "Sec-WebSocket-Accept: %s\r\n"
                       "\r\n",
                       accept_key);
    if (write(fd, response, (size_t)len) != len) {
        perror("ws_server: write");
        return -1;
    }
    return 0;
}

/* ------------------------------------------------------------------ */
/* public API                                                         */
/* ------------------------------------------------------------------ */

int ws_server_accept_one(uint16_t port)
{
    int listener = socket(AF_INET, SOCK_STREAM, 0);
    if (listener < 0) {
        perror("ws_server: socket");
        return -1;
    }

    int one = 1;
    setsockopt(listener, SOL_SOCKET, SO_REUSEADDR, &one, sizeof one);

    struct sockaddr_in addr;
    memset(&addr, 0, sizeof addr);
    addr.sin_family = AF_INET;
    addr.sin_addr.s_addr = htonl(INADDR_ANY);
    addr.sin_port = htons(port);
    if (bind(listener, (struct sockaddr *)&addr, sizeof addr) < 0) {
        perror("ws_server: bind");
        close(listener);
        return -1;
    }
    if (listen(listener, 1) < 0) {
        perror("ws_server: listen");
        close(listener);
        return -1;
    }

    printf("ws_server: waiting for a connection on port %u...\n", (unsigned)port);
    fflush(stdout);
    int fd = accept(listener, NULL, NULL);
    close(listener);
    if (fd < 0) {
        perror("ws_server: accept");
        return -1;
    }

    if (do_handshake(fd) != 0) {
        close(fd);
        return -1;
    }

    /* The protocol is request/response per frame (Present -> PresentDone);
     * Nagle's algorithm turns that into ~40ms delayed-ACK stalls. */
    int nodelay = 1;
    setsockopt(fd, IPPROTO_TCP, TCP_NODELAY, &nodelay, sizeof nodelay);

    printf("ws_server: client connected\n");
    fflush(stdout);
    return fd;
}
