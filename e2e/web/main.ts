/*
 * Minimal page for the end-to-end tests: connect the local GPU to the
 * server named in ?server=... and let it drive.  All progress goes to the
 * console, where run.sh can read it via chromium's --enable-logging.
 */

import { RemoteGpuClient } from "remote-webgpu-client";

const params = new URLSearchParams(location.search);
const server = params.get("server") ?? "ws://127.0.0.1:8080";
const canvas = document.getElementById("canvas") as HTMLCanvasElement;

RemoteGpuClient.connect(server, {
  canvas,
  onStatus: (message) => console.log(`e2e: ${message}`),
  onClose: (reason) => console.log(`e2e: closed: ${reason}`),
}).catch((error) => {
  console.error(`e2e: connect failed: ${error instanceof Error ? error.message : error}`);
});
