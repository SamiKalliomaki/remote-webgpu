#!/usr/bin/env bash
#
# Coverage-guided fuzzing of the untrusted half of the protocol.
#
# Builds fuzz_target.c and the library it exercises with clang's libFuzzer
# plus AddressSanitizer and UndefinedBehaviorSanitizer, seeds the corpus
# from the driver's built-in seed messages, and runs.  This is the good
# tool; e2e/fuzz/fuzz_main.c is the fallback for machines without clang.
#
#   ./run_libfuzzer.sh                    build and fuzz for 60s
#   ./run_libfuzzer.sh -max_total_time=600 -jobs=8
#   ./run_libfuzzer.sh crash-abc123       reproduce a saved crash
#
# Anything on the command line is passed through to the fuzzer, so all of
# libFuzzer's own flags work.  The corpus persists in fuzz/corpus/ between
# runs, which is what makes a long campaign worth more than a short one;
# crashes land in fuzz/artifacts/.  regressions/ is passed as a second,
# read-only corpus so every input the fuzzer has ever found is re-run (and
# re-leak-checked) at the start of every campaign; libFuzzer only writes new
# units into the first corpus directory.
#
# Note: libprotobuf-c comes from the system and is therefore not
# instrumented, so the fuzzer gets no coverage signal from inside the
# protobuf parser itself (ASan still checks it).  The dictionary and the
# seed corpus are what get past the parser and into the handlers.

set -euo pipefail
cd "$(dirname "$0")"

CLANG=${CLANG:-clang}
command -v "$CLANG" > /dev/null || {
    echo "fuzz: $CLANG not found; install clang or use build/fuzz_receive" >&2
    exit 2
}

REPO=../..
BUILD=build
CORPUS=corpus
ARTIFACTS=artifacts
mkdir -p "$BUILD" "$CORPUS" "$ARTIFACTS"

# The wire-protocol code, generated from the same schema as everything else.
protoc --c_out="$BUILD" -I "$REPO/proto" "$REPO/proto/remote_webgpu.proto"

# -fsanitize=fuzzer covers the coverage instrumentation; address and
# undefined turn latent corruption into an immediate, attributable failure.
# UBSan findings must be fatal or the fuzzer would run straight past them.
CFLAGS=(
    -g -O1 -std=gnu11   # the library's CMake builds it as gnu11 too
    -fsanitize=fuzzer,address,undefined
    -fno-sanitize-recover=undefined
    -fno-omit-frame-pointer
    -I "$REPO/remote_webgpu/include"
    -I "$REPO/remote_webgpu/src"
    -I ../src
    -I "$BUILD"
    -I .
)

echo "fuzz: building with $($CLANG --version | head -1)"
"$CLANG" "${CFLAGS[@]}" \
    fuzz_target.c \
    "$REPO/remote_webgpu/src/remote_webgpu.c" \
    "$REPO/remote_webgpu/src/remote_methods.c" \
    "$REPO/remote_webgpu/src/stubs.c" \
    "$BUILD/remote_webgpu.pb-c.c" \
    $(pkg-config --cflags --libs libprotobuf-c) \
    -o "$BUILD/fuzz_libfuzzer"

# Seed the corpus with one well-formed message of every kind the client may
# send; without them the mutator spends its life failing to parse.
if [ -z "$(ls -A "$CORPUS" 2>/dev/null)" ]; then
    if [ -x ../build/fuzz_receive ]; then
        ../build/fuzz_receive --dump-corpus "$CORPUS"
    else
        echo "fuzz: build/fuzz_receive not built; starting from an empty corpus" >&2
    fi
fi

# -close_fd_mask=3: the library narrates every rejected message, which at
# fuzzing rates costs more than the fuzzing does.
exec "$BUILD/fuzz_libfuzzer" \
    -close_fd_mask=3 \
    -dict=remote_webgpu.dict \
    -artifact_prefix="$ARTIFACTS/" \
    -max_len=65536 \
    -max_total_time=60 \
    "$@" "$CORPUS" regressions
