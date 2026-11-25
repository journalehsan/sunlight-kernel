cmake_minimum_required(VERSION 3.16)
project(SunlightQtStyle LANGUAGES CXX)

find_package(Qt6 6.2 REQUIRED COMPONENTS Widgets)
set(CMAKE_AUTOMOC ON)
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

set(plugin_root "${CMAKE_CURRENT_BINARY_DIR}/plugins")
set(SUNLIGHT_QT_PLUGIN_INSTALL_ROOT "plugins" CACHE STRING
    "Qt plugin root relative to the install prefix (or an absolute path)")

add_library(sunlightstyle MODULE SunlightStyle.cpp SunlightStylePlugin.cpp)
target_link_libraries(sunlightstyle PRIVATE Qt6::Widgets)
set_target_properties(sunlightstyle PROPERTIES
    LIBRARY_OUTPUT_DIRECTORY "${plugin_root}/styles/$<0:>")
install(TARGETS sunlightstyle LIBRARY DESTINATION "${SUNLIGHT_QT_PLUGIN_INSTALL_ROOT}/styles")

add_executable(sunlight-preview Preview.cpp)
target_link_libraries(sunlight-preview PRIVATE Qt6::Widgets)
target_compile_definitions(sunlight-preview PRIVATE SUNLIGHT_BUILD_PLUGIN_ROOT="${plugin_root}")
add_dependencies(sunlight-preview sunlightstyle)

add_executable(sunlight-hello HelloWorld.cpp)
target_link_libraries(sunlight-hello PRIVATE Qt6::Widgets)
target_compile_definitions(sunlight-hello PRIVATE SUNLIGHT_BUILD_PLUGIN_ROOT="${plugin_root}")
add_dependencies(sunlight-hello sunlightstyle)

include(CTest)
if(BUILD_TESTING)
    add_test(NAME sunlight-style-smoke COMMAND sunlight-preview --smoke)
    set_tests_properties(sunlight-style-smoke PROPERTIES
        ENVIRONMENT "QT_QPA_PLATFORM=offscreen;QT_SCALE_FACTOR=1;QT_PLUGIN_PATH=")
endif()
