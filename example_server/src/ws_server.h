#ifndef WS_SERVER_H
#define WS_SERVER_H

#include <stdint.h>

/*
 * Listen on `port`, block until one client connects and completes the
 * websocket opening handshake (RFC 6455), and return the connected fd.
 * After this the socket carries websocket frames.
 *
 * Returns the fd on success, -1 on failure.  The listening socket is closed
 * either way; only a single connection is accepted.
 */
int ws_server_accept_one(uint16_t port);

#endif /* WS_SERVER_H */
