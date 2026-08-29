#ifndef WS_TRANSPORT_H
#define WS_TRANSPORT_H

#include <stddef.h>
#include <stdint.h>

/*
 * Server-side websocket framing (RFC 6455) on an fd whose HTTP handshake is
 * already complete.  Only binary messages are supported; pings are answered
 * transparently.  Every protocol message is one Envelope (see
 * proto/remote_webgpu.proto) per binary websocket message.
 */

/* Send `len` bytes as a single binary message.  Returns 0 on success. */
int ws_send_binary(int fd, const uint8_t *data, size_t len);

/*
 * Block until a complete binary message arrives and return it in a
 * malloc()ed buffer (*out, *out_len).  Control frames (ping/pong) are
 * handled internally.  Returns 0 on success, -1 on close/error.
 */
int ws_recv_binary(int fd, uint8_t **out, size_t *out_len);

#endif /* WS_TRANSPORT_H */
