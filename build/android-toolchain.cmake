set(ANDROID_ABI "$ENV{NABI_ANDROID_ABI}" CACHE STRING "Android ABI")
set(ANDROID_PLATFORM android-30 CACHE STRING "Minimum Android API")
include("$ENV{ANDROID_NDK_HOME}/build/cmake/android.toolchain.cmake")
