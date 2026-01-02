# OneCAD

## Overview
OneCAD is a C++ CAD application utilizing:
- **CMake** for build configuration.
- **Qt6** for the Graphical User Interface (GUI).
- **Eigen3** for linear algebra.
- **OpenCASCADE Technology (OCCT)** for geometric modeling.

## Prerequisites (macOS)

1.  **Xcode Command Line Tools**:
    ```bash
    xcode-select --install
    ```
2.  **Homebrew**:
    Visit [brew.sh](https://brew.sh).
3.  **Dependencies**:
    ```bash
    brew install cmake eigen opencascade qt
    ```

## Building

1.  Create a build directory:
    ```bash
    mkdir build
    cd build
    ```

2.  Configure with CMake:
    ```bash
    cmake ..
    ```

3.  Build:
    ```bash
    make
    ```

4.  Run:
    ```bash
    ./OneCAD
    ```

## Development

- **Dependencies**:
  - Qt6 is expected at `/opt/homebrew/opt/qt` (Homebrew default) or via `CMAKE_PREFIX_PATH`.
  - OCCT is expected via Homebrew paths.
- **Project Structure**:
  - `src/main.cpp`: Application entry point.
  - `src/MainWindow.cpp`/`.h`: Main application window setup.