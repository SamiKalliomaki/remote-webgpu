#!/usr/bin/env bash
#
# End-to-end tests for remote WebGPU: build everything, start a native
# server per test, point a headless WebGPU browser at the web client and
# verify the results the server reads back over the websocket.
#
#   ./run.sh                  build + run all tests
#   ./run.sh compute queries  run only these feature tests (plus no golden)
#   UPDATE_GOLDEN=1 ./run.sh  re-record golden/triangle.ppm
#   CHROMIUM=... PORT=...     override the browser binary / base port
#
# Requires: cmake, protoc + libprotobuf-c, node/npm, python3, and a
# chromium with WebGPU (SwiftShader is fine; no GPU or display needed).

set -u
cd "$(dirname "$0")"

CHROMIUM=${CHROMIUM:-$(command -v chromium || command -v chromium-browser \
                       || command -v google-chrome || true)}
if [ -z "$CHROMIUM" ]; then
    echo "e2e: no chromium found; set CHROMIUM=/path/to/browser" >&2
    exit 2
fi
# The golden screenshot is deterministic because web/index.html pins the
# canvas CSS size (and headless devicePixelRatio is 1); the fixed window
# size just keeps the rest of the environment stable.
CHROMIUM_FLAGS=(--headless=new --no-sandbox --disable-gpu-sandbox
                --enable-unsafe-webgpu --enable-features=Vulkan
                --use-angle=vulkan --window-size=400,300)

FEATURE_TESTS=(limits buffers compute render queries async image)
RUN_GOLDEN=1
RUN_HOSTILE=1
RUN_FUZZ=1
if [ "$#" -gt 0 ]; then
    FEATURE_TESTS=()
    RUN_GOLDEN=0
    RUN_HOSTILE=0
    RUN_FUZZ=0
    # `./run.sh hostile fuzz` runs just those; anything else is a feature test.
    for arg in "$@"; do
        case "$arg" in
            hostile) RUN_HOSTILE=1 ;;
            fuzz)    RUN_FUZZ=1 ;;
            *)       FEATURE_TESTS+=("$arg") ;;
        esac
    done
fi

PORT=${PORT:-8210}          # websocket ports: PORT, PORT+1, ...
HTTP_PORT=$((PORT + 100))   # the web client is served here
TIMEOUT=${TIMEOUT:-60}      # per test, seconds
FUZZ_SECONDS=${FUZZ_SECONDS:-20}  # fuzzing budget in the suite; raise it for
                                  # a real campaign, or use fuzz/run_libfuzzer.sh
GOLDEN=golden/triangle.ppm

WORK=$(mktemp -d /tmp/remote-webgpu-e2e.XXXXXX)
PIDS=()
FAILED=0

cleanup() {
    for pid in "${PIDS[@]:-}"; do kill "$pid" 2>/dev/null; done
    wait 2>/dev/null
}
trap cleanup EXIT

log() { echo "e2e: $*"; }

# ------------------------------------------------------------------ build

log "building native targets..."
cmake -S . -B build > "$WORK/cmake.log" 2>&1 \
    && cmake --build build -j"$(nproc)" >> "$WORK/cmake.log" 2>&1 \
    || { cat "$WORK/cmake.log"; exit 1; }
cmake -S ../example_server -B ../example_server/build > "$WORK/cmake2.log" 2>&1 \
    && cmake --build ../example_server/build -j"$(nproc)" >> "$WORK/cmake2.log" 2>&1 \
    || { cat "$WORK/cmake2.log"; exit 1; }

log "bundling web client..."
[ -d node_modules ] || npm install --no-fund --no-audit > "$WORK/npm.log" 2>&1 \
    || { cat "$WORK/npm.log"; exit 1; }
npm run --silent bundle > "$WORK/esbuild.log" 2>&1 \
    || { cat "$WORK/esbuild.log"; exit 1; }

python3 -m http.server "$HTTP_PORT" -d web > /dev/null 2>&1 &
PIDS+=($!)

# ------------------------------------------------------------------ tests

next_port=$PORT

# run_server NAME SERVER_CMD... : start the server, connect a headless
# browser to it, and use the server's exit code as the verdict.
run_server() {
    local name=$1 port=$next_port
    next_port=$((next_port + 1))
    shift
    log "[$name] running..."
    timeout "$TIMEOUT" "$@" --port "$port" > "$WORK/$name.server.log" 2>&1 &
    local server=$!
    sleep 0.5
    "$CHROMIUM" "${CHROMIUM_FLAGS[@]}" --enable-logging=stderr \
        --user-data-dir="$WORK/$name.profile" \
        "http://127.0.0.1:$HTTP_PORT/?server=ws://127.0.0.1:$port" \
        > "$WORK/$name.browser.log" 2>&1 &
    local browser=$!
    wait "$server"
    local rc=$?
    kill "$browser" 2>/dev/null
    wait "$browser" 2>/dev/null
    if [ "$rc" -ne 0 ]; then
        FAILED=1
        log "[$name] FAILED (server exit $rc); server log:"
        cat "$WORK/$name.server.log"
        log "[$name] browser console:"
        grep -i console "$WORK/$name.browser.log" | tail -20
    else
        log "[$name] OK"
    fi
    return "$rc"
}

# One feature per binary; see src/test_*.c.
for test in "${FEATURE_TESTS[@]}"; do
    if [ ! -x "build/test_$test" ]; then
        log "[$test] FAILED: no such test (build/test_$test)"
        FAILED=1
        continue
    fi
    run_server "$test" "./build/test_$test"
done

# The untrusted-client test: the browser is replaced by a hostile peer that
# completes the handshake with absurd values and then abuses the protocol.
# The server binary checks the invariants and exits 0 if it survived.
if [ "$RUN_HOSTILE" -eq 1 ]; then
    port=$next_port
    next_port=$((next_port + 1))
    log "[hostile] running..."
    timeout "$TIMEOUT" ./build/test_hostile --port "$port" \
        > "$WORK/hostile.server.log" 2>&1 &
    server=$!
    sleep 0.5
    python3 tools/hostile_client.py --port "$port" \
        > "$WORK/hostile.client.log" 2>&1 &
    client=$!
    wait "$server"
    rc=$?
    kill "$client" 2>/dev/null
    wait "$client" 2>/dev/null
    if [ "$rc" -ne 0 ]; then
        FAILED=1
        log "[hostile] FAILED (server exit $rc); server log:"
        tail -30 "$WORK/hostile.server.log"
        log "[hostile] client log:"
        tail -10 "$WORK/hostile.client.log"
    else
        log "[hostile] OK"
    fi
fi

# Fuzzing the client -> server direction.  The good tool is libFuzzer with
# ASan and UBSan (fuzz/run_libfuzzer.sh, which needs clang); the fallback is
# the same fuzz target driven by the seeded generator in fuzz/fuzz_main.c,
# so this stage runs everywhere.  Either way the budget is small enough to
# belong in a test suite -- it is a regression check, not a campaign.
if [ "$RUN_FUZZ" -eq 1 ]; then
    log "[fuzz] running (${FUZZ_SECONDS}s)..."
    # Everything the fuzzer has ever found, replayed first: a regression
    # fails here in a second instead of waiting to be rediscovered.
    if ! ./build/fuzz_receive fuzz/regressions/*.bin > "$WORK/fuzz.regressions.log" 2>&1; then
        FAILED=1
        log "[fuzz] FAILED replaying the saved regressions:"
        tail -20 "$WORK/fuzz.regressions.log"
    fi
    if command -v "${CLANG:-clang}" > /dev/null; then
        log "[fuzz] using libFuzzer + ASan/UBSan"
        ( cd fuzz && ./run_libfuzzer.sh -max_total_time="$FUZZ_SECONDS" ) \
            > "$WORK/fuzz.log" 2>&1
        rc=$?
    else
        log "[fuzz] no clang; using the built-in generator"
        ./build/fuzz_receive --quiet --seconds "$FUZZ_SECONDS" --seed "${FUZZ_SEED:-1}" \
            --artifact "$WORK/fuzz-crash.bin" > "$WORK/fuzz.log" 2>&1
        rc=$?
    fi
    if [ "$rc" -ne 0 ]; then
        FAILED=1
        log "[fuzz] FAILED (exit $rc):"
        tail -40 "$WORK/fuzz.log"
        log "[fuzz] reproduce with: e2e/build/fuzz_receive <input>"
    else
        log "[fuzz] OK ($(tail -1 "$WORK/fuzz.log"))"
    fi
fi

# The rendering/present path, compared against a golden screenshot: the
# triangle rotates a fixed amount per frame and web/index.html pins the
# canvas size, so the final frame is bit-for-bit reproducible.
if [ "$RUN_GOLDEN" -eq 1 ]; then
    SHOT="$WORK/triangle.ppm"
    run_server golden ../example_server/build/spinning_triangle \
        --frames 30 --screenshot "$SHOT"
    if [ ! -f "$SHOT" ]; then
        log "[golden] FAILED: no screenshot written"
        FAILED=1
    elif [ "${UPDATE_GOLDEN:-0}" = "1" ]; then
        mkdir -p golden
        cp "$SHOT" "$GOLDEN"
        log "[golden] updated $GOLDEN ($(stat -c%s "$GOLDEN") bytes)"
    elif [ ! -f "$GOLDEN" ]; then
        log "[golden] FAILED: $GOLDEN missing; record it with UPDATE_GOLDEN=1 ./run.sh"
        FAILED=1
    else
        python3 - "$GOLDEN" "$SHOT" <<'EOF' || FAILED=1
import sys

def read_ppm(path):
    with open(path, "rb") as f:
        assert f.readline().strip() == b"P6", f"{path}: not a binary PPM"
        size = tuple(map(int, f.readline().split()))
        assert int(f.readline()) == 255
        return size, f.read(size[0] * size[1] * 3)

(gw, gh), golden = read_ppm(sys.argv[1])
(sw, sh), shot = read_ppm(sys.argv[2])
if (gw, gh) != (sw, sh):
    sys.exit(f"e2e: [golden] FAILED: size {sw}x{sh}, golden is {gw}x{gh} "
             "(browser window size changed?)")
if golden == shot:
    print(f"e2e: [golden] screenshot matches golden exactly ({gw}x{gh})")
    sys.exit(0)
diffs = [i for i in range(0, len(golden), 3)
         if golden[i:i+3] != shot[i:i+3]]
worst = max(abs(golden[i+c] - shot[i+c]) for i in diffs for c in range(3))
sys.exit(f"e2e: [golden] FAILED: {len(diffs)} of {gw*gh} pixels differ "
         f"(max channel delta {worst}); re-record with UPDATE_GOLDEN=1 "
         "./run.sh if the change is intended")
EOF
    fi
fi

# ------------------------------------------------------------------ result

if [ "$FAILED" -ne 0 ]; then
    log "FAIL (logs kept in $WORK)"
    exit 1
fi
log "PASS"
rm -rf "$WORK"
