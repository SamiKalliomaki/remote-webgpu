#include "connection.h"

#include <stdio.h>
#include <stdlib.h>
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
    if (conn->fd >= 0 && ws_send_binary(conn->fd, data, size) != 0) {
        /* Sends after a disconnect are harmless (the render loop notices
         * via connection_pump); don't spam about them. */
    }
}

int connection_pump(Connection *conn)
{
    if (conn->fd < 0)
        return -1;

    uint8_t *data;
    size_t size;
    if (ws_recv_binary(conn->fd, &data, &size) != 0)
        return -1;

    if (conn->adapter)
        wgpuRemoteAdapterReceiveData(conn->adapter, data, size);
    else
        fprintf(stderr, "connection: dropping message, no adapter yet\n");
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
