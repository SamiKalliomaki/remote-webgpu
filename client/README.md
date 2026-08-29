# remote-webgpu-client

TypeScript client library for the remote WebGPU protocol
(`../proto/remote_webgpu.proto`).  The *server* is a native app written
against `webgpu.h`; the *client* (this library) runs where the GPU is and
lends it out.

```ts
import { RemoteGpuClient } from "remote-webgpu-client";

const client = await RemoteGpuClient.connect("ws://localhost:8080", {
  canvas,                      // frames are presented here, at its CSS size
  onStatus: (m) => console.log(m),
  onFrame: () => {},           // once per presented frame (FPS counters)
  onClose: (reason) => console.warn(reason),
});
client.adapter;                // the local GPUAdapter backing the connection
client.device;                 // the local GPUDevice the server will drive
```

`connect()` requests a local adapter/device from `navigator.gpu`, opens the
websocket, waits for the server's `ServerHello` and answers with a
`ClientHello` carrying the adapter's info.  After the handshake the server
streams GPU commands; `src/executor.ts` executes them against the local
device in order (an id -> object map mirrors the server's handles), presents
to the `canvas` passed to `connect()`, answers `Present` with `PresentDone`
on the next animation frame, and serves buffer readbacks for the server's
screenshots.  The canvas size (in device pixels) is reported in the
`ClientHello` and re-reported as a canvas-resize `Event` whenever the
element changes size; the canvas backing store itself is only resized when
the server reconfigures the surface, so its pixel size always matches what
the server configured.  `sendEvent(name, payload)` sends a user-defined
event for the server application to consume (the example client streams
the pointer position this way).

Commands:

```sh
npm install
npm run gen        # regenerate src/gen/ from ../proto (needs protoc)
npm run gen:enums  # regenerate src/gen/enums.ts from webgpu.h
npm run check      # typecheck
```

The package is consumed as TypeScript source (see `exports`); bundle it with
esbuild/vite/etc., as `../example_client` does.
