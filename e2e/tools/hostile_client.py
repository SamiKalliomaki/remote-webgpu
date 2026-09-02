#!/usr/bin/env python3
"""A deliberately hostile remote-webgpu client, for e2e/src/test_hostile.c.

Stands in for the browser: speaks just enough of the protocol to complete
the handshake, then feeds the server everything the wire format allows but
no honest client would send -- saturated and zeroed capabilities, repeated
hellos, resize storms, event floods, unsolicited replies, truncated
envelopes -- and answers every buffer map with fewer bytes than were asked
for.  The server is expected to survive all of it; see test_hostile.c for
the invariants that are checked on the other side.

Usage: hostile_client.py --port PORT [--seconds N]
"""
import argparse
import base64
import os
import socket
import struct
import time

# --------------------------------------------------------------- protobuf

def varint(n):
    out = bytearray()
    while True:
        byte = n & 0x7F
        n >>= 7
        out.append(byte | (0x80 if n else 0))
        if not n:
            return bytes(out)

def tag(field, wire):     return varint((field << 3) | wire)
def vfield(field, value): return tag(field, 0) + varint(value)
def bfield(field, data):  return tag(field, 2) + varint(len(data)) + data
def sfield(field, text):  return bfield(field, text.encode())

U32_MAX = 0xFFFFFFFF
U64_MAX = 0xFFFFFFFFFFFFFFFF

# Envelope field numbers (see proto/remote_webgpu.proto).
F_ERROR, F_HELLO = 3, 2
F_MAP_BUFFER = 31
F_PRESENT_DONE, F_MAP_DATA, F_EVENT = 50, 51, 52
F_ERROR_SCOPE, F_WORK_DONE, F_COMPILATION = 90, 91, 92
F_UNCAPTURED, F_DEVICE_LOST, F_TEXTURE_LOADED = 93, 94, 95

def limits_absurd():
    """Every limit either 0 (division-by-zero bait) or saturated."""
    msg = b"".join(vfield(f, 0) for f in range(1, 15))
    msg += vfield(15, U64_MAX)   # max_uniform_buffer_binding_size
    msg += vfield(16, U64_MAX)   # max_storage_buffer_binding_size
    msg += vfield(17, 0)         # min_uniform_buffer_offset_alignment
    msg += vfield(18, 3)         # min_storage_buffer_offset_alignment (not a power of two)
    msg += vfield(19, 0)
    msg += vfield(20, U64_MAX)   # max_buffer_size
    msg += b"".join(vfield(f, U32_MAX) for f in range(21, 33))
    return msg

def client_hello(version=6, features=50000):
    info = b"".join(sfield(f, "hostile") for f in (1, 2, 3, 4))
    hello = vfield(1, version) + bfield(2, info)
    hello += vfield(3, U32_MAX) + vfield(4, U32_MAX)     # canvas size
    hello += bfield(5, limits_absurd())
    hello += b"".join(vfield(6, U32_MAX) for _ in range(features))
    return bfield(F_HELLO, hello)

def event_resize(w, h):   return bfield(F_EVENT, bfield(1, vfield(1, w) + vfield(2, h)))
def event_user(n, p):     return bfield(F_EVENT, bfield(2, sfield(1, n) + bfield(2, p)))
def present_done():       return bfield(F_PRESENT_DONE, b"")
def work_done(rid):       return bfield(F_WORK_DONE, vfield(1, rid))
def map_data(rid, data):  return bfield(F_MAP_DATA, vfield(1, rid) + bfield(2, data))
def error_scope(rid):     return bfield(F_ERROR_SCOPE, vfield(1, rid) + vfield(2, U32_MAX))
def compilation(rid):     return bfield(F_COMPILATION, vfield(1, rid))
def texture_loaded(rid):  return bfield(F_TEXTURE_LOADED,
                                        vfield(1, rid) + vfield(4, U32_MAX) + vfield(5, U32_MAX))
def uncaptured(msg):      return bfield(F_UNCAPTURED, vfield(1, 2) + sfield(2, msg))
def device_lost(msg):     return bfield(F_DEVICE_LOST, vfield(1, 1) + sfield(2, msg))

def read_varint(buf, pos):
    value = shift = 0
    while pos < len(buf):
        byte = buf[pos]
        pos += 1
        value |= (byte & 0x7F) << shift
        if not byte & 0x80:
            return value, pos
        shift += 7
    raise ValueError("truncated varint")

def map_request_id(envelope):
    """The request id of a MapBuffer envelope, or None for anything else."""
    try:
        key, pos = read_varint(envelope, 0)
        if key >> 3 != F_MAP_BUFFER or key & 7 != 2:
            return None
        length, pos = read_varint(envelope, pos)
        body = envelope[pos:pos + length]
        key, pos = read_varint(body, 0)
        if key >> 3 != 1:            # MapBuffer.request_id
            return None
        return read_varint(body, pos)[0]
    except ValueError:
        return None

# -------------------------------------------------------------- websocket

class WebSocket:
    """The tiny subset of RFC 6455 this needs: a masked binary client."""

    def __init__(self, host, port, path="/"):
        self.sock = socket.create_connection((host, port), timeout=15)
        key = base64.b64encode(os.urandom(16)).decode()
        self.sock.sendall(
            f"GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nUpgrade: websocket\r\n"
            f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\n"
            f"Sec-WebSocket-Version: 13\r\n\r\n".encode())
        head = b""
        while b"\r\n\r\n" not in head:
            chunk = self.sock.recv(4096)
            if not chunk:
                raise RuntimeError("server closed during the websocket handshake")
            head += chunk
        status = head.split(b"\r\n", 1)[0]
        if b"101" not in status:
            raise RuntimeError(f"websocket upgrade refused: {status!r}")
        self.buf = head.split(b"\r\n\r\n", 1)[1]

    def send(self, *envelopes):
        """One binary message carrying size-prefixed envelopes."""
        self.send_raw(b"".join(struct.pack("<I", len(e)) + e for e in envelopes))

    def send_raw(self, body):
        header = bytearray([0x82])
        mask = os.urandom(4)
        if len(body) < 126:
            header.append(0x80 | len(body))
        elif len(body) <= 0xFFFF:
            header.append(0x80 | 126)
            header += struct.pack(">H", len(body))
        else:
            header.append(0x80 | 127)
            header += struct.pack(">Q", len(body))
        header += mask
        self.sock.sendall(bytes(header)
                          + bytes(b ^ mask[i & 3] for i, b in enumerate(body)))

    def _fill(self, n):
        while len(self.buf) < n:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise EOFError
            self.buf += chunk

    def recv(self):
        """The payload of one (unmasked, server-sent) binary frame."""
        self._fill(2)
        length = self.buf[1] & 0x7F
        pos = 2
        if length == 126:
            self._fill(4)
            length = struct.unpack(">H", self.buf[2:4])[0]
            pos = 4
        elif length == 127:
            self._fill(10)
            length = struct.unpack(">Q", self.buf[2:10])[0]
            pos = 10
        self._fill(pos + length)
        payload = self.buf[pos:pos + length]
        self.buf = self.buf[pos + length:]
        return payload

    def envelopes(self):
        """The size-prefixed envelopes in the next binary message."""
        data = self.recv()
        out, pos = [], 0
        while pos + 4 <= len(data):
            size = struct.unpack("<I", data[pos:pos + 4])[0]
            pos += 4
            if pos + size > len(data):
                break
            out.append(data[pos:pos + size])
            pos += size
        return out

    def close(self):
        try:
            self.sock.close()
        except OSError:
            pass

# ------------------------------------------------------------------ abuse

def abuse(ws):
    for _ in range(200):
        ws.send(client_hello(features=1000))                   # repeated hellos
    for i in range(2000):
        ws.send(event_resize(U32_MAX - i, U32_MAX),            # resize storm
                event_resize(0, 0), event_resize(1, U32_MAX))
    for _ in range(5000):
        ws.send(event_user("keydown", b"KeyW\nw\n1"),          # event flood
                event_user("mousemove", b"\x01\x02"),          # (truncated payload)
                event_user("ÿþ", os.urandom(32)))
    for i in range(2000):
        ws.send(present_done(), work_done(i), error_scope(i),  # unsolicited replies
                compilation(i), texture_loaded(i))
    for _ in range(200):
        ws.send(uncaptured("fabricated validation error"),     # fabricated errors
                device_lost("fabricated device loss"))
    ws.send(bfield(F_HELLO, b"\x08"))                          # truncated envelope
    ws.send_raw(struct.pack("<I", 0xFFFFFFF0) + b"\x01\x02")   # lying size prefix

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--seconds", type=float, default=15.0,
                        help="how long to keep answering map requests")
    args = parser.parse_args()

    ws = WebSocket(args.host, args.port)
    ws.envelopes()                    # the ServerHello
    ws.send(client_hello())
    abuse(ws)

    # Answer every buffer map with a reply too short to satisfy it, for as
    # long as the server keeps talking, then hang up.
    ws.sock.settimeout(1.0)
    deadline = time.time() + args.seconds
    while time.time() < deadline:
        try:
            for envelope in ws.envelopes():
                request = map_request_id(envelope)
                if request is not None:
                    ws.send(map_data(request, b"\x00" * 16))
        except (socket.timeout, TimeoutError):
            continue
        except (EOFError, OSError):
            break
    ws.close()

if __name__ == "__main__":
    main()
