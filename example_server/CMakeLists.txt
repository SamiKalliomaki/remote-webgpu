cmake_minimum_required(VERSION 3.20)
project(spinning_triangle C)

set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)

# Our own implementation of the WebGPU headers; it talks to a remote GPU
# over a socket instead of driving a local one.
add_subdirectory(${CMAKE_CURRENT_SOURCE_DIR}/../remote_webgpu remote_webgpu)

add_executable(spinning_triangle
    src/main.c
    src/gpu_setup.c
    src/connection.c
    src/ws_transport.c
    src/render.c
    src/ws_server.c)

target_include_directories(spinning_triangle PRIVATE src)
target_link_libraries(spinning_triangle PRIVATE remote_webgpu m)
