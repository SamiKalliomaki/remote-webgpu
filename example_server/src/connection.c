#include "connection.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "ws_server.h"
#include "ws_transport.h"

int connection_accept(Connection *conn, uint16_t port)
{
    conn->adapter = NULL;
    conn->fd = ws_server_accept_one(port);
    return conn->fd < 0 ? -1 : 0;
}

void connection_send(void const *data, size_t size, void *userdata)
{
    Connection *conn = userdata;
    if (conn->fd < 0)
        return;

    /* A websocket message carries size-prefixed envelopes (see the
     * transport notes in remote_webgpu.proto); this simple server sends
     * one envelope per message. */
    uint8_t *framed = malloc(4 + size);
    if (!framed)
        return;
    framed[0] = (uint8_t)(size);
    framed[1] = (uint8_t)(size >> 8);
    framed[2] = (uint8_t)(size >> 16);
    framed[3] = (uint8_t)(size >> 24);
    memcpy(framed + 4, data, size);
    if (ws_send_binary(conn->fd, framed, 4 + size) != 0) {
        /* Sends after a disconnect are harmless (the render loop notices
         * via connection_pump); don't spam about them. */
    }
    free(framed);
}

int connection_pump(Connection *conn)
{
    if (conn->fd < 0)
        return -1;

    uint8_t *data;
    size_t size;
    if (ws_recv_binary(conn->fd, &data, &size) != 0)
        return -1;

    if (conn->adapter) {
        /* Split the message into its size-prefixed envelopes.  A malformed
         * prefix ends the loop; the library's parser flags the garbage. */
        size_t offset = 0;
        while (offset + 4 <= size) {
            size_t len = (size_t)data[offset] | (size_t)data[offset + 1] << 8 |
                         (size_t)data[offset + 2] << 16 |
                         (size_t)data[offset + 3] << 24;
            offset += 4;
            if (offset + len > size)
                break;
            wgpuRemoteAdapterReceiveData(conn->adapter, data + offset, len);
            offset += len;
        }
    } else {
        fprintf(stderr, "connection: dropping message, no adapter yet\n");
    }
    free(data);
    return 0;
}

void connection_close(Connection *conn)
{
    if (conn->fd >= 0)
        close(conn->fd);
    conn->fd = -1;
    conn->adapter = NULL;
}
