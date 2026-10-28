cmake_minimum_required(VERSION 3.20)

# GLFW path: C:\Apps\Libraries\GLFW\include
# Compiler path: C:\Apps\Clang+LLVM\bin\clang++.exe
# Languages: C++20, Rust

# Volcano Client links this library, and its Debug build uses the release CRT and STL (the debug ones are several times
# slower and nobody steps through them). The C++ API passes std::string and std::vector, so both sides must agree: turn
# this ON together with VOLCANO_DEBUG_CHECKS there.
option(PINCEL_DEBUG_CHECKS "Debug builds use the debug CRT and STL" OFF)
if(WIN32 AND NOT PINCEL_DEBUG_CHECKS)
    set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreadedDLL")
endif()

project(Pincel VERSION 0.1.0 LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

if(NOT CMAKE_BUILD_TYPE AND NOT CMAKE_CONFIGURATION_TYPES)
    set(CMAKE_BUILD_TYPE Debug CACHE STRING "Build type" FORCE)
endif()

option(PINCEL_BUILD_SHARED "Build pincel as a shared library (pincel.dll / libpincel.so) instead of a static one" OFF)
option(PINCEL_BUILD_EXAMPLE "Build the example application in src/example" ON)

# =============================================================================
# 1. Rust core (src/pincel/rust) -> static library via cargo
# =============================================================================
find_program(CARGO_EXECUTABLE cargo HINTS "$ENV{USERPROFILE}/.cargo/bin" "$ENV{HOME}/.cargo/bin")
if(NOT CARGO_EXECUTABLE)
    message(FATAL_ERROR "cargo not found. Install Rust from https://rustup.rs or add cargo to PATH.")
endif()

set(PINCEL_RUST_DIR  "${CMAKE_CURRENT_SOURCE_DIR}/src/pincel/rust")
set(PINCEL_CARGO_DIR "${CMAKE_BINARY_DIR}/cargo")
# Single-config generators only (Ninja, Makefiles): BYPRODUCTS can't use generator expressions.
if(CMAKE_BUILD_TYPE STREQUAL "Debug")
    set(PINCEL_RUST_PROFILE_DIR debug)
    set(PINCEL_CARGO_PROFILE_ARG "")
else()
    set(PINCEL_RUST_PROFILE_DIR release)
    set(PINCEL_CARGO_PROFILE_ARG --release)
endif()

if(WIN32 AND NOT MINGW)
    set(PINCEL_RUST_LIB_NAME "pincel_core.lib")
else()
    set(PINCEL_RUST_LIB_NAME "libpincel_core.a")
endif()
set(PINCEL_RUST_LIB "${PINCEL_CARGO_DIR}/${PINCEL_RUST_PROFILE_DIR}/${PINCEL_RUST_LIB_NAME}")

file(GLOB_RECURSE PINCEL_RUST_SOURCES CONFIGURE_DEPENDS "${PINCEL_RUST_DIR}/src/*.rs")

# Always runs; cargo itself decides whether anything needs rebuilding.
add_custom_target(pincel_rust_build
    COMMAND ${CMAKE_COMMAND} -E env "CARGO_TARGET_DIR=${PINCEL_CARGO_DIR}"
            ${CARGO_EXECUTABLE} build ${PINCEL_CARGO_PROFILE_ARG}
            --manifest-path "${PINCEL_RUST_DIR}/Cargo.toml"
    BYPRODUCTS "${PINCEL_RUST_LIB}"
    SOURCES "${PINCEL_RUST_DIR}/Cargo.toml" ${PINCEL_RUST_SOURCES}
    COMMENT "Building Rust core (cargo)"
    VERBATIM)

add_library(pincel_rust STATIC IMPORTED GLOBAL)
set_target_properties(pincel_rust PROPERTIES IMPORTED_LOCATION "${PINCEL_RUST_LIB}")
add_dependencies(pincel_rust pincel_rust_build)

# System libraries Rust's std needs when linked from a non-Rust linker.
# Regenerate with: cargo rustc --lib -- --print native-static-libs
if(WIN32)
    set_property(TARGET pincel_rust PROPERTY INTERFACE_LINK_LIBRARIES
        advapi32 bcrypt kernel32 ntdll userenv ws2_32 synchronization)
elseif(UNIX)
    set_property(TARGET pincel_rust PROPERTY INTERFACE_LINK_LIBRARIES ${CMAKE_DL_LIBS} pthread m)
endif()

# =============================================================================
# 2. C++ API (src/pincel/cpp), links the Rust core
# =============================================================================
set(PINCEL_CPP_DIR "${CMAKE_CURRENT_SOURCE_DIR}/src/pincel/cpp")
file(GLOB_RECURSE PINCEL_CPP_SOURCES CONFIGURE_DEPENDS "${PINCEL_CPP_DIR}/src/*.cpp")

if(PINCEL_BUILD_SHARED)
    add_library(pincel SHARED ${PINCEL_CPP_SOURCES})
    target_compile_definitions(pincel PRIVATE PINCEL_BUILDING PUBLIC PINCEL_SHARED)

    # pincel.dll exports two things: the C++ classes and functions (marked PINCEL_API in the headers), and the C interface
    # in ffi.h, which is implemented in Rust and linked into the DLL. Nothing marks the Rust functions for export, so the
    # export list is made here from the header: every `pincel_*` function it declares. (A test in the Rust crate checks
    # that the header and the Rust exports declare the same functions, so the list can't drift from what exists.)
    # Mods and anything else that wants the C interface can call these from the DLL.
    set(PINCEL_FFI_HEADER "${PINCEL_CPP_DIR}/include/Pincel/ffi.h")
    file(READ "${PINCEL_FFI_HEADER}" PINCEL_FFI_TEXT)
    string(REGEX REPLACE "//[^\n]*" "" PINCEL_FFI_CODE "${PINCEL_FFI_TEXT}")
    string(REGEX MATCHALL "pincel_[a-z0-9_]+[ \t]*\\(" PINCEL_FFI_CALLS "${PINCEL_FFI_CODE}")
    set(PINCEL_EXPORTS "")
    foreach(call IN LISTS PINCEL_FFI_CALLS)
        string(REGEX REPLACE "[ \t]*\\($" "" name "${call}")
        list(APPEND PINCEL_EXPORTS "${name}")
    endforeach()
    list(REMOVE_DUPLICATES PINCEL_EXPORTS)
    list(LENGTH PINCEL_EXPORTS PINCEL_EXPORT_COUNT)
    if(PINCEL_EXPORT_COUNT LESS 50)
        message(FATAL_ERROR "Only found ${PINCEL_EXPORT_COUNT} functions in ffi.h to export. Has its format changed?")
    endif()
    string(REPLACE ";" "\n    " PINCEL_EXPORTS_TEXT "${PINCEL_EXPORTS}")
    set(PINCEL_DEF_FILE "${CMAKE_BINARY_DIR}/pincel_c_abi.def")
    file(WRITE "${PINCEL_DEF_FILE}" "EXPORTS\n    ${PINCEL_EXPORTS_TEXT}\n")
    # Regenerate when the header changes.
    set_property(DIRECTORY APPEND PROPERTY CMAKE_CONFIGURE_DEPENDS "${PINCEL_FFI_HEADER}")
    if(WIN32)
        target_link_options(pincel PRIVATE "LINKER:/DEF:${PINCEL_DEF_FILE}")
    endif()
    message(STATUS "pincel.dll will export the C++ API and ${PINCEL_EXPORT_COUNT} C functions")
else()
    add_library(pincel STATIC ${PINCEL_CPP_SOURCES})
endif()
add_library(Pincel::Pincel ALIAS pincel)

target_include_directories(pincel PUBLIC "${PINCEL_CPP_DIR}/include")
target_compile_features(pincel PUBLIC cxx_std_20)
target_link_libraries(pincel PRIVATE pincel_rust)
set_target_properties(pincel PROPERTIES
    ARCHIVE_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/build"
    LIBRARY_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/build"
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/build")

# =============================================================================
# 3. Example application (src/example)
# =============================================================================
if(PINCEL_BUILD_EXAMPLE)
    file(GLOB_RECURSE PINCEL_EXAMPLE_SOURCES CONFIGURE_DEPENDS "${CMAKE_CURRENT_SOURCE_DIR}/src/example/*.cpp")
    # glad.c is the generated OpenGL 4.6 core loader (glad 0.1.x); glad.h and KHR/ sit beside it.
    add_executable(PincelExample ${PINCEL_EXAMPLE_SOURCES} src/example/glad.c)
    target_include_directories(PincelExample PRIVATE src/example)
    target_link_libraries(PincelExample PRIVATE Pincel::Pincel)
    set_target_properties(PincelExample PROPERTIES
        RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/build")

    # GLFW is a prebuilt binary distribution; glfw3.lib links the dynamic CRT, matching this project.
    set(GLFW_ROOT "C:/Apps/Libraries/GLFW" CACHE PATH "GLFW root directory")
    find_library(GLFW_LIBRARY glfw3
        PATHS "${GLFW_ROOT}/lib-vc2026" "${GLFW_ROOT}/lib-vc2022" "${GLFW_ROOT}/lib-vc2019"
        NO_DEFAULT_PATH)
    if(NOT GLFW_LIBRARY OR NOT EXISTS "${GLFW_ROOT}/include/GLFW/glfw3.h")
        message(FATAL_ERROR "GLFW not found under ${GLFW_ROOT}. Set -DGLFW_ROOT=<path>.")
    endif()
    target_include_directories(PincelExample PRIVATE "${GLFW_ROOT}/include")
    target_link_libraries(PincelExample PRIVATE "${GLFW_LIBRARY}")
    if(WIN32)
        target_link_libraries(PincelExample PRIVATE opengl32 gdi32 user32 shell32)
    endif()
endif()
