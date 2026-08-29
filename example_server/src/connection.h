#ifndef CONNECTION_H
#define CONNECTION_H

#include <stdint.h>

#include <webgpu/remote.h>

/*
 * The application's side of the transport: a connected websocket to the
 * remote GPU client.  The remote_webgpu library is transport-agnostic; this
 * module adapts it to a websocket by
 *   - passing connection_send() as the library's send callback, and
 *   - feeding received messages into wgpuRemoteAdapterReceiveData() from
 *     connection_pump().
 */
typedef struct {
    int fd;
    /* Set by gpu_setup() once the adapter exists; pumped messages go here. */
    WGPUAdapter adapter;
} Connection;

/*
 * Listen on `port`, block until one client connects and completes the
 * websocket handshake.  Returns 0 on success, -1 on failure.
 */
int connection_accept(Connection *conn, uint16_t port);

/*
 * WGPURemoteSendCallback: sends one protocol message as a binary websocket
 * message.  `userdata` is the Connection.
 */
void connection_send(void const *data, size_t size, void *userdata);

/*
 * Block until one message arrives and feed it to the adapter.  Returns 0 on
 * success, -1 when the client disconnected (or on a transport error).
 */
int connection_pump(Connection *conn);

void connection_close(Connection *conn);

#endif /* CONNECTION_H */
