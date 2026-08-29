import { RemoteGpuClient } from "remote-webgpu-client";

const canvas = document.getElementById("canvas") as HTMLCanvasElement;
const statusBox = document.getElementById("status") as HTMLDivElement;
const fpsBox = document.getElementById("fps") as HTMLDivElement;

const lines: string[] = [];
function log(message: string): void {
  console.log(message);
  lines.push(message);
  statusBox.textContent = lines.slice(-8).join("\n");
}

/* The canvas fills the window via CSS.  Its backing-store size is set by
 * the library when the server configures the surface; the library reports
 * CSS-size changes to the server, which reconfigures to match. */

/* FPS: counted per presented frame, displayed once a second. */
let frameCount = 0;
setInterval(() => {
  fpsBox.textContent = `${frameCount} fps`;
  frameCount = 0;
}, 1000);

/* The server to attach to; override with ?server=ws://host:port */
const params = new URLSearchParams(location.search);
const url = params.get("server") ?? `ws://${location.hostname || "localhost"}:8080`;

try {
  const client = await RemoteGpuClient.connect(url, {
    canvas,
    onStatus: log,
    onFrame: () => { frameCount += 1; },
    onClose: (reason) => log(`disconnected: ${reason}`),
  });
  const info = client.adapter?.info;
  log(info
    ? `local GPU: ${[info.vendor, info.architecture, info.device, info.description]
        .filter(Boolean).join(" / ") || "(no details exposed)"}`
    : "connected without a local GPU");
  log("connected; the server is driving the local GPU");

  /* Stream the pointer position to the server as a user-defined event:
   * "mousemove" with the coordinates in device pixels (matching the frame
   * the server renders) as two little-endian float32s. */
  canvas.addEventListener("pointermove", (event) => {
    const rect = canvas.getBoundingClientRect();
    const scale = devicePixelRatio;
    const position = new Float32Array([
      (event.clientX - rect.left) * scale,
      (event.clientY - rect.top) * scale,
    ]);
    client.sendEvent("mousemove", new Uint8Array(position.buffer));
  });
} catch (error) {
  log(`failed: ${error instanceof Error ? error.message : String(error)}`);
}
