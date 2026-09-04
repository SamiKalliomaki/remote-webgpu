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

/* FPS and protocol traffic: accumulated per presented frame, displayed
 * once a second (messages/frame and bandwidth are averages over frames). */
let frameCount = 0;
let messageCount = 0;
let packetCount = 0;
let byteCount = 0;
setInterval(() => {
  const perFrame = frameCount > 0 ? Math.round(messageCount / frameCount) : 0;
  const pktsPerFrame = frameCount > 0 ? Math.round(packetCount / frameCount) : 0;
  const kib = byteCount / 1024;
  fpsBox.textContent =
    `${frameCount} fps · ${perFrame} msgs in ${pktsPerFrame} pkts/frame · ${
      kib >= 1024 ? (kib / 1024).toFixed(1) + " MiB/s" : Math.round(kib) + " KiB/s"}`;
  frameCount = 0;
  messageCount = 0;
  packetCount = 0;
  byteCount = 0;
}, 1000);

/* The server to attach to; override with ?server=ws://host:port.  By
 * default the page assumes the server is what served it (bevy_pbr_game hosts
 * this page on its websocket port), using wss:// when the page itself was
 * served over HTTPS (e.g. behind a TLS-terminating reverse proxy), and
 * falling back to localhost:8080 when the page was opened from disk. */
const params = new URLSearchParams(location.search);
const url = params.get("server") ??
  (location.protocol.startsWith("http") && location.host
    ? `${location.protocol === "https:" ? "wss" : "ws"}://${location.host}`
    : "ws://localhost:8080");

try {
  const client = await RemoteGpuClient.connect(url, {
    canvas,
    onStatus: log,
    onFrame: (stats) => {
      frameCount += 1;
      messageCount += stats.messages;
      packetCount += stats.packets;
      byteCount += stats.bytes;
    },
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

  /* Forward key presses as user-defined events: "keydown"/"keyup" with a
   * UTF-8 payload of "<code>\n<key>\n<repeat 0|1>" (the browser's
   * KeyboardEvent.code / .key names).  Keys that would scroll the page are
   * suppressed locally; the server application is the one looking at them. */
  const encoder = new TextEncoder();
  for (const type of ["keydown", "keyup"] as const) {
    window.addEventListener(type, (event) => {
      if (["Space", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Tab"]
          .includes(event.code)) {
        event.preventDefault();
      }
      const payload = `${event.code}\n${event.key}\n${event.repeat ? 1 : 0}`;
      client.sendEvent(type, encoder.encode(payload));
    });
  }
} catch (error) {
  log(`failed: ${error instanceof Error ? error.message : String(error)}`);
}
