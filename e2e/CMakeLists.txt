cmake_minimum_required(VERSION 3.20)
project(remote_webgpu_e2e C)

set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)

# The library under test.
add_subdirectory(${CMAKE_CURRENT_SOURCE_DIR}/../remote_webgpu remote_webgpu)

# The tests reuse the example server's websocket plumbing; the transport is
# the app's business and that app already has one that works.
set(EXAMPLE_SRC ${CMAKE_CURRENT_SOURCE_DIR}/../example_server/src)

# One executable per feature: src/test_<name>.c defines run_test(), the
# shared test_main.c owns the accept / device bring-up.  run.sh runs each
# against a fresh browser session.
set(E2E_TESTS limits buffers compute render queries async image hostile)

# Optional sanitizers for the whole tree (the fuzzer wants them):
#   cmake -S . -B build -DE2E_SANITIZE=address,undefined
set(E2E_SANITIZE "" CACHE STRING "sanitizers to build the tests with")
if(E2E_SANITIZE)
    add_compile_options(-fsanitize=${E2E_SANITIZE} -fno-omit-frame-pointer -g)
    add_link_options(-fsanitize=${E2E_SANITIZE})
endif()

foreach(test ${E2E_TESTS})
    add_executable(test_${test}
        src/test_${test}.c
        src/test_main.c
        src/test_util.c
        ${EXAMPLE_SRC}/gpu_setup.c
        ${EXAMPLE_SRC}/connection.c
        ${EXAMPLE_SRC}/ws_transport.c
        ${EXAMPLE_SRC}/ws_server.c)
    target_include_directories(test_${test} PRIVATE src ${EXAMPLE_SRC})
    target_link_libraries(test_${test} PRIVATE remote_webgpu m)
endforeach()

# The fuzz target (fuzz/fuzz_target.c) is written against libFuzzer's entry
# point; fuzz/run_libfuzzer.sh builds it with clang for real coverage-guided
# fuzzing.  This binary is the portable fallback: the same target driven by
# the seeded generator in fuzz/fuzz_main.c, so `run.sh fuzz` works on a box
# with no clang, and so saved crash inputs can be replayed anywhere.
add_executable(fuzz_receive
    fuzz/fuzz_target.c
    fuzz/fuzz_main.c)
target_include_directories(fuzz_receive PRIVATE src fuzz ${EXAMPLE_SRC})
target_link_libraries(fuzz_receive PRIVATE remote_webgpu m)
