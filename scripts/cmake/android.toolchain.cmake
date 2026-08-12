if(NOT DEFINED ENV{ANDROID_NDK_ROOT} OR "$ENV{ANDROID_NDK_ROOT}" STREQUAL "")
    message(FATAL_ERROR "ANDROID_NDK_ROOT is required")
endif()

set(ANDROID_ABI "$ENV{ANDROID_ABI}" CACHE STRING "Android ABI" FORCE)
set(ANDROID_PLATFORM "$ENV{ANDROID_PLATFORM}" CACHE STRING "Android platform" FORCE)

# Keep whisper-rs-sys aligned with the static libraries its Rust build script
# links. Android's legacy toolchain re-enters this file while configuring and
# can otherwise restore CMake's shared-library default from a stale cache.
set(BUILD_SHARED_LIBS OFF CACHE BOOL "Build shared libraries" FORCE)

# whisper-rs-sys builds whisper.cpp as the top-level CMake project, whose
# defaults enable targets omitted from the crate source package. Keep the
# mobile runtime build limited to the libraries Cargo links.
set(BUILD_TESTING OFF CACHE BOOL "Build tests" FORCE)
set(WHISPER_BUILD_EXAMPLES OFF CACHE BOOL "Build Whisper examples" FORCE)
set(WHISPER_BUILD_SERVER OFF CACHE BOOL "Build Whisper server" FORCE)
set(WHISPER_BUILD_TESTS OFF CACHE BOOL "Build Whisper tests" FORCE)

include("$ENV{ANDROID_NDK_ROOT}/build/cmake/android.toolchain.cmake")
