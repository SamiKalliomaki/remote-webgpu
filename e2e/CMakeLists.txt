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
set(E2E_TESTS limits buffers compute render queries async image)

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
