cmake_minimum_required(VERSION 3.20)
project(remote_webgpu C)

set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)

find_program(PROTOC_EXECUTABLE protoc REQUIRED)
find_package(PkgConfig REQUIRED)
pkg_check_modules(PROTOBUF_C REQUIRED IMPORTED_TARGET libprotobuf-c)

# Generate the wire-protocol code from the shared schema in ../proto.
set(PROTO_DIR ${CMAKE_CURRENT_SOURCE_DIR}/../proto)
set(PROTO_GEN_DIR ${CMAKE_CURRENT_BINARY_DIR}/proto_gen)
add_custom_command(
    OUTPUT ${PROTO_GEN_DIR}/remote_webgpu.pb-c.c ${PROTO_GEN_DIR}/remote_webgpu.pb-c.h
    COMMAND ${CMAKE_COMMAND} -E make_directory ${PROTO_GEN_DIR}
    COMMAND ${PROTOC_EXECUTABLE} --c_out=${PROTO_GEN_DIR} -I ${PROTO_DIR}
            ${PROTO_DIR}/remote_webgpu.proto
    DEPENDS ${PROTO_DIR}/remote_webgpu.proto
    COMMENT "Generating protobuf-c code from remote_webgpu.proto")

# Our implementation of the WebGPU headers.  Instead of driving a local GPU,
# it forwards (well, will forward) every call to a remote GPU over a socket.
add_library(remote_webgpu STATIC
    src/remote_webgpu.c
    src/remote_methods.c
    src/stubs.c   # regenerate with tools/generate_stubs.py
    ${PROTO_GEN_DIR}/remote_webgpu.pb-c.c)

target_include_directories(remote_webgpu PUBLIC include)
target_include_directories(remote_webgpu PRIVATE src ${PROTO_GEN_DIR})
target_link_libraries(remote_webgpu PRIVATE PkgConfig::PROTOBUF_C)
