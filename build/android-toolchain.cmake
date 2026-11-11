set(ANDROID_ABI "$ENV{NABI_ANDROID_ABI}" CACHE STRING "Android ABI")
set(ANDROID_PLATFORM android-30 CACHE STRING "Minimum Android API")
include("$ENV{ANDROID_NDK_HOME}/build/cmake/android.toolchain.cmake")

# The NDK sets this to its bundled yasm, which cannot run natively on Apple
# Silicon. Use a host NASM while still producing Android x86_64 assembly.
if(ANDROID_ABI STREQUAL "x86_64")
  find_program(NABI_HOST_NASM NAMES nasm HINTS ENV NASM_PATH
    NO_CMAKE_FIND_ROOT_PATH REQUIRED)
  set(CMAKE_ASM_NASM_COMPILER "${NABI_HOST_NASM}")
endif()
