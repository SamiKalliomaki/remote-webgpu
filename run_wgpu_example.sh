#!/usr/bin/env bash
# Run any demo from the upstream wgpu examples (examples/features/src) with
# the GPU living in a connected browser tab: the workspace's `wgpu` and
# `winit` crates are replaced by the remote-webgpu compatible crates in
# ./rust, which serve a websocket that the web client (example_client/)
# connects to.
#
# Usage:
#   ./run_wgpu_example.sh                 # list available demos
#   ./run_wgpu_example.sh cube            # wait for a client, then run "cube"
#   REMOTE_WEBGPU_PORT=9000 ./run_wgpu_example.sh boids
#
# Then open the web client (see example_client/README.md):
#   cd client && npm install && cd ../example_client && npm install && npm run serve
#   -> http://127.0.0.1:8000/?server=ws://127.0.0.1:8080
set -euo pipefail

REPO_DIR="$(cd "$(dirname "$0")" && pwd)"
CHECKOUT_DIR="${WGPU_CHECKOUT_DIR:-$REPO_DIR/rust/.wgpu-examples}"

# The upstream commit the compatible crates were written against.
WGPU_REPO=https://github.com/gfx-rs/wgpu.git
WGPU_REV=e9a873e5d2c7e74ea6b19bd20280acc01db6d4f5

# Demos that cannot work over browser WebGPU (ray tracing, mesh shaders,
# cooperative matrices, multiview) are removed from the build entirely.
UNSUPPORTED="ray_aabb_compute ray_cube_compute ray_cube_fragment ray_cube_normals ray_scene ray_shadows ray_traced_triangle mesh_shader cooperative_matrix multiview"

if [ ! -e "$CHECKOUT_DIR/.remote-webgpu-prepared" ]; then
    echo "Preparing wgpu checkout in $CHECKOUT_DIR ..."
    if [ ! -d "$CHECKOUT_DIR/.git" ]; then
        mkdir -p "$CHECKOUT_DIR"
        git init -q "$CHECKOUT_DIR"
        git -C "$CHECKOUT_DIR" remote add origin "$WGPU_REPO" 2>/dev/null || true
    fi
    git -C "$CHECKOUT_DIR" fetch -q --depth 1 origin "$WGPU_REV"
    git -C "$CHECKOUT_DIR" checkout -q -f "$WGPU_REV"

    UNSUPPORTED="$UNSUPPORTED" RUST_DIR="$REPO_DIR/rust" CHECKOUT_DIR="$CHECKOUT_DIR" \
    python3 - <<'PYEOF'
import os, re

checkout = os.environ["CHECKOUT_DIR"]
rust_dir = os.environ["RUST_DIR"]
unsupported = os.environ["UNSUPPORTED"].split()

# 1. Point the workspace's wgpu and winit dependencies at our crates.
manifest_path = os.path.join(checkout, "Cargo.toml")
manifest = open(manifest_path).read()
lines = manifest.splitlines(keepends=True)
start = next(i for i, l in enumerate(lines)
             if l.startswith('wgpu = {') and '"./wgpu"' in l)
end = start
while not lines[end].rstrip().endswith('}'):
    end += 1
lines[start:end + 1] = ['wgpu = { path = "%s/wgpu" }\n' % rust_dir]
manifest = "".join(lines)
manifest, n = re.subn(
    r"^winit = \{[^\n]*\}\n",
    'winit = { path = "%s/winit" }\n' % rust_dir,
    manifest, count=1, flags=re.M)
assert n == 1, "failed to rewrite the winit workspace dependency"
open(manifest_path, "w").write(manifest)

# 2. Remove the demos that cannot work over browser WebGPU.
src = os.path.join(checkout, "examples", "features", "src")
lib_path = os.path.join(src, "lib.rs")
lib = open(lib_path).read()
for name in unsupported:
    lib = lib.replace("pub mod %s;\n" % name, "")
open(lib_path, "w").write(lib)

main_path = os.path.join(src, "main.rs")
main = open(main_path).read()
for name in unsupported:
    main, n = re.subn(
        r"    ExampleDesc \{\n        name: \"%s\",\n(?:.*?\n)*?    \},\n" % name,
        "", main, count=1)
    assert n == 1, "failed to remove example %s from main.rs" % name
open(main_path, "w").write(main)
PYEOF

    touch "$CHECKOUT_DIR/.remote-webgpu-prepared"
fi

cd "$CHECKOUT_DIR"
exec cargo run -p wgpu-examples --bin wgpu-examples -- "$@"
