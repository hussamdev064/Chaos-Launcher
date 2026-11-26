# CMake toolchain for Windows x86-64 cross-compilation from the builder image.
# Phase 4 passes this via -DCMAKE_TOOLCHAIN_FILE=/opt/cmake/mingw-x86_64.cmake.
set(CMAKE_SYSTEM_NAME Windows)
set(CMAKE_SYSTEM_PROCESSOR x86_64)

set(CMAKE_C_COMPILER x86_64-w64-mingw32-gcc)
set(CMAKE_CXX_COMPILER x86_64-w64-mingw32-g++)
set(CMAKE_RC_COMPILER x86_64-w64-mingw32-windres)

# The pinned upstream passes a function pointer to RSDK_main's void* parameter,
# which MSVC accepts but GCC rejects without this compatibility flag.
set(CMAKE_CXX_FLAGS_INIT "-fpermissive")

set(CMAKE_FIND_ROOT_PATH /usr/x86_64-w64-mingw32)
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)
