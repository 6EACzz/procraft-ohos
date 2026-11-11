cmake_minimum_required(VERSION 3.5)
project(procraft)

set(NATIVERENDER_ROOT_PATH ${CMAKE_CURRENT_SOURCE_DIR})

include_directories(${NATIVERENDER_ROOT_PATH})

# The app itself: the desktop's interface, built as a library (procraft/desktop, crate
# `procraft_ui`). Built by `harmony/build.sh`, which puts it among the module's own libraries, so
# it ships in the HAP and is found by its name at run time (an absolute path would be the build
# machine's, which the device has never heard of).
set(PROCRAFT_UI "${CMAKE_CURRENT_SOURCE_DIR}/../../../libs/${OHOS_ARCH}/libprocraft_ui.so")
if(NOT EXISTS "${PROCRAFT_UI}")
    message(FATAL_ERROR "libprocraft_ui.so is missing: run harmony/build.sh (it builds it)")
endif()
add_library(procraft_ui SHARED IMPORTED)
set_target_properties(procraft_ui PROPERTIES
    IMPORTED_LOCATION "${PROCRAFT_UI}"
    IMPORTED_NO_SONAME TRUE
)

add_library(procraft SHARED napi_init.cpp)

target_link_libraries(procraft PUBLIC
    procraft_ui
    libace_napi.z.so
    libhilog_ndk.z.so
    libnative_window.so
    libace_ndk.z.so
)
