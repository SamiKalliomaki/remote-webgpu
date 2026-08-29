#include "ws_transport.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/uio.h>
#include <unistd.h>

/* Frame size we are willing to accept before assuming a broken peer. */
#define WS_MAX_PAYLOAD (64u * 1024u * 1024u)

#define WS_OP_CONT   0x0
#define WS_OP_TEXT   0x1
#define WS_OP_BINARY 0x2
#define WS_OP_CLOSE  0x8
#define WS_OP_PING   0x9
#define WS_OP_PONG   0xA

static int read_exact(int fd, void *buf, size_t len)
{
    unsigned char *p = buf;
    while (len) {
        ssize_t n = read(fd, p, len);
        if (n <= 0)
            return -1;
        p += n;
        len -= (size_t)n;
    }
    return 0;
}

/* Server-to-client frames are never masked. */
static int send_frame(int fd, unsigned opcode, const uint8_t *data, size_t len)
{
    uint8_t header[10];
    size_t header_len = 2;

    header[0] = 0x80u | (uint8_t)opcode; /* FIN + opcode */
    if (len < 126) {
        header[1] = (uint8_t)len;
    } else if (len <= 0xFFFF) {
        header[1] = 126;
        header[2] = (uint8_t)(len >> 8);
        header[3] = (uint8_t)len;
        header_len = 4;
    } else {
        header[1] = 127;
        for (int i = 0; i < 8; ++i)
            header[2 + i] = (uint8_t)((uint64_t)len >> (8 * (7 - i)));
        header_len = 10;
    }

    /* One writev so header and payload leave as a single segment; split
     * writes interact badly with Nagle's algorithm + delayed ACKs. */
    struct iovec iov[2] = {
        { header, header_len },
        { (void *)data, len },
    };
    size_t total = header_len + len;
    struct msghdr msg;
    memset(&msg, 0, sizeof msg);
    msg.msg_iov = iov;
    msg.msg_iovlen = len ? 2 : 1;
    while (total) {
        /* MSG_NOSIGNAL: teardown-time sends after a disconnect must fail
         * with EPIPE, not kill the process. */
        ssize_t n = sendmsg(fd, &msg, MSG_NOSIGNAL);
        if (n <= 0)
            return -1;
        total -= (size_t)n;
        for (size_t i = 0; i < (size_t)msg.msg_iovlen && n; ++i) {
            size_t take = (size_t)n < iov[i].iov_len ? (size_t)n : iov[i].iov_len;
            iov[i].iov_base = (char *)iov[i].iov_base + take;
            iov[i].iov_len -= take;
            n -= (ssize_t)take;
        }
    }
    return 0;
}

int ws_send_binary(int fd, const uint8_t *data, size_t len)
{
    return send_frame(fd, WS_OP_BINARY, data, len);
}

/* Read one frame; on success *payload is malloc()ed (or NULL when empty). */
static int read_frame(int fd, unsigned *opcode, int *fin,
                      uint8_t **payload, size_t *payload_len)
{
    uint8_t header[2];
    if (read_exact(fd, header, 2) != 0)
        return -1;

    *fin = (header[0] & 0x80u) != 0;
    *opcode = header[0] & 0x0Fu;
    const int masked = (header[1] & 0x80u) != 0;
    uint64_t len = header[1] & 0x7Fu;

    if (len == 126) {
        uint8_t ext[2];
        if (read_exact(fd, ext, 2) != 0)
            return -1;
        len = (uint64_t)ext[0] << 8 | ext[1];
    } else if (len == 127) {
        uint8_t ext[8];
        if (read_exact(fd, ext, 8) != 0)
            return -1;
        len = 0;
        for (int i = 0; i < 8; ++i)
            len = len << 8 | ext[i];
    }
    if (len > WS_MAX_PAYLOAD) {
        fprintf(stderr, "ws_transport: oversized frame (%llu bytes)\n",
                (unsigned long long)len);
        return -1;
    }

    uint8_t mask[4] = {0};
    if (masked && read_exact(fd, mask, 4) != 0)
        return -1;

    uint8_t *data = NULL;
    if (len) {
        data = malloc((size_t)len);
        if (!data || read_exact(fd, data, (size_t)len) != 0) {
            free(data);
            return -1;
        }
        if (masked)
            for (uint64_t i = 0; i < len; ++i)
                data[i] ^= mask[i & 3];
    }

    *payload = data;
    *payload_len = (size_t)len;
    return 0;
}

int ws_recv_binary(int fd, uint8_t **out, size_t *out_len)
{
    /* Reassembly buffer for a fragmented message in progress. */
    uint8_t *message = NULL;
    size_t message_len = 0;

    for (;;) {
        unsigned opcode;
        int fin;
        uint8_t *payload;
        size_t len;
        if (read_frame(fd, &opcode, &fin, &payload, &len) != 0) {
            free(message);
            return -1;
        }

        switch (opcode) {
        case WS_OP_BINARY:
        case WS_OP_CONT: {
            if ((opcode == WS_OP_BINARY) != (message == NULL && message_len == 0)) {
                fprintf(stderr, "ws_transport: bad fragmentation sequence\n");
                free(payload);
                free(message);
                return -1;
            }
            if (message_len + len > WS_MAX_PAYLOAD) {
                fprintf(stderr, "ws_transport: oversized fragmented message\n");
                free(payload);
                free(message);
                return -1;
            }
            uint8_t *grown = realloc(message, message_len + len + 1);
            if (!grown) {
                free(payload);
                free(message);
                return -1;
            }
            message = grown;
            if (len)
                memcpy(message + message_len, payload, len);
            message_len += len;
            free(payload);
            if (fin) {
                *out = message;
                *out_len = message_len;
                return 0;
            }
            break;
        }

        case WS_OP_PING:
            send_frame(fd, WS_OP_PONG, payload, len);
            free(payload);
            break;

        case WS_OP_PONG:
            free(payload);
            break;

        case WS_OP_CLOSE:
            send_frame(fd, WS_OP_CLOSE, payload, len);
            free(payload);
            free(message);
            return -1;

        default:
            fprintf(stderr, "ws_transport: unexpected opcode 0x%X\n", opcode);
            free(payload);
            free(message);
            return -1;
        }
    }
}
