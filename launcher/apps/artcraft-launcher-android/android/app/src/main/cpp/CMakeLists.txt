cmake_minimum_required(VERSION 3.22.1)
project(artcraft_launcher_android LANGUAGES CXX)

# This CMakeLists.txt exists only to satisfy AGP's externalNativeBuild requirement.
# The actual Rust library (libartcraft_launcher_android.so) is built by cargo-ndk
# and placed in the jniLibs directory by the build.sh script.

# Create a dummy target so CMake doesn't complain
add_library(dummy STATIC dummy.cpp)
target_include_directories(dummy PRIVATE ${CMAKE_CURRENT_SOURCE_DIR})