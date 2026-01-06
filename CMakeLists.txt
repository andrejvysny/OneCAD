cmake_minimum_required(VERSION 3.25)
project(OneCAD VERSION 0.1.0 LANGUAGES CXX)

# C++20 as per specification
set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# --- Qt6 Setup ---
set(CMAKE_AUTOMOC ON)
set(CMAKE_AUTORCC ON)
set(CMAKE_AUTOUIC ON)

# Hint for Homebrew Qt6
set(CMAKE_PREFIX_PATH "/opt/homebrew/opt/qt" ${CMAKE_PREFIX_PATH})

find_package(Qt6 COMPONENTS Core Gui Widgets OpenGLWidgets Svg REQUIRED)

# --- Dependencies ---

# 1. Eigen3
find_package(Eigen3 3.3 REQUIRED NO_MODULE)

# 2. OpenCASCADE (OCCT)
find_package(OpenCASCADE)
if(NOT OpenCASCADE_FOUND)
    set(POSSIBLE_OCCT_DIRS 
        "/opt/homebrew/lib/cmake/opencascade"
        "/usr/local/lib/cmake/opencascade"
        "/opt/homebrew/opt/opencascade/lib/cmake/opencascade"
    )
    foreach(DIR ${POSSIBLE_OCCT_DIRS})
        if(EXISTS ${DIR})
            message(STATUS "Found potential OpenCASCADE config at: ${DIR}")
            set(OpenCASCADE_DIR ${DIR})
            find_package(OpenCASCADE REQUIRED)
            break()
        endif()
    endforeach()
endif()

if(NOT OpenCASCADE_FOUND)
    message(FATAL_ERROR "OpenCASCADE not found. Please install it.")
endif()

message(STATUS "Qt6 Version: ${Qt6_VERSION}")
message(STATUS "Eigen3 Version: ${Eigen3_VERSION}")
message(STATUS "OpenCASCADE Version: ${OpenCASCADE_VERSION}")

# --- Subdirectories ---
add_subdirectory(third_party/planegcs)
add_subdirectory(src/app)
add_subdirectory(src/core)
add_subdirectory(src/kernel)
add_subdirectory(src/render)
add_subdirectory(src/ui)
add_subdirectory(src/io)
add_subdirectory(tests)

# --- macOS App Icon ---
if(APPLE)
    set(MACOSX_BUNDLE_ICON_FILE AppIcon.icns)
    set(APP_ICON_MACOSX ${CMAKE_SOURCE_DIR}/resources/AppIcon.icns)
    set_source_files_properties(${APP_ICON_MACOSX} PROPERTIES
        MACOSX_PACKAGE_LOCATION "Resources"
    )
endif()

# --- Main Executable ---

add_executable(OneCAD
    src/main.cpp
    resources/resources.qrc
    ${APP_ICON_MACOSX}
)

# --- Linking ---

target_link_libraries(OneCAD 
    PRIVATE 
    onecad_app
    onecad_ui
    Qt6::Core 
    Qt6::Gui 
    Qt6::Widgets
    Qt6::OpenGLWidgets
    Qt6::Svg
    Eigen3::Eigen
    ${OpenCASCADE_LIBRARIES}
)

if(OpenCASCADE_LIBRARY_DIR)
    target_link_directories(OneCAD PUBLIC ${OpenCASCADE_LIBRARY_DIR})
endif()

target_include_directories(OneCAD 
    PRIVATE 
    ${CMAKE_SOURCE_DIR}/src
    ${OpenCASCADE_INCLUDE_DIR}
)

# --- macOS Bundle Configuration ---
set_target_properties(OneCAD PROPERTIES
    MACOSX_BUNDLE ON
    MACOSX_BUNDLE_GUI_IDENTIFIER "com.onecad.app"
    MACOSX_BUNDLE_BUNDLE_NAME "OneCAD"
    MACOSX_BUNDLE_BUNDLE_VERSION "${PROJECT_VERSION}"
    MACOSX_BUNDLE_SHORT_VERSION_STRING "${PROJECT_VERSION}"
    MACOSX_BUNDLE_ICON_FILE "${MACOSX_BUNDLE_ICON_FILE}"
)
