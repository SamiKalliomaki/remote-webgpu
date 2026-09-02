# syntax=docker/dockerfile:1.7
#
# Hosts bevy_pbr_game: the multiplayer bevy_pbr demo that renders on each
# connected browser tab's GPU.  The image serves the web client and the
# websocket protocol on one port (8000 by default).
#
# The build needs the whole repository (the crate embeds ../../example_client
# and remote-wgpu-sys compiles ../../remote_webgpu), so build from the repo
# root:
#
#     docker build -f rust/bevy_pbr_game/Dockerfile -t bevy_pbr_game .
#     docker run --rm -p 8000:8000 bevy_pbr_game
#
# then open http://localhost:8000/ in a WebGPU-capable browser.

# ---------------------------------------------------------------- build ---
FROM rust:1-bookworm AS build

# protoc + protoc-gen-c + libprotobuf-c: remote-wgpu-sys generates and compiles the C wire
# protocol.  node/npm: build.rs bundles example_client/ into the binary.
RUN apt-get update && apt-get install -y --no-install-recommends \
        protobuf-compiler protobuf-c-compiler libprotobuf-c-dev pkg-config nodejs npm \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY proto proto
COPY remote_webgpu remote_webgpu
COPY client client
COPY example_client example_client
COPY rust rust

WORKDIR /src/rust
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/rust/target \
    cargo build --release --locked -p bevy_pbr_game \
    && cp target/release/bevy_pbr_game /bevy_pbr_game

# -------------------------------------------------------------- runtime ---
FROM debian:bookworm-slim

# The only shared library the binary links besides libc.
RUN apt-get update && apt-get install -y --no-install-recommends libprotobuf-c1 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --no-create-home game

COPY --from=build /bevy_pbr_game /usr/local/bin/bevy_pbr_game

USER game
# The game listens on 0.0.0.0:8000; REMOTE_WEBGPU_PORT overrides the port.
EXPOSE 8000
ENTRYPOINT ["/usr/local/bin/bevy_pbr_game"]
