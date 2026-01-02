#include <QApplication>
#include <QSurfaceFormat>
#include "ui/mainwindow/MainWindow.h"
#include "app/Application.h"

#include <iostream>

// Eigen3
#include <Eigen/Dense>

// OpenCASCADE (OCCT)
#include <Standard_Version.hxx>
#include <gp_Pnt.hxx>

int main(int argc, char *argv[]) {
    // CRITICAL: Set OpenGL format BEFORE QApplication creation
    // This ensures all QOpenGLWidgets use the correct context
    QSurfaceFormat format;
    format.setVersion(4, 1);  // macOS supports up to 4.1 Core
    format.setProfile(QSurfaceFormat::CoreProfile);
    format.setSamples(4);  // 4x MSAA
    format.setDepthBufferSize(24);
    format.setStencilBufferSize(8);
    format.setSwapBehavior(QSurfaceFormat::DoubleBuffer);
    QSurfaceFormat::setDefaultFormat(format);

    QApplication app(argc, argv);
    
    // Initialize application singleton
    onecad::app::Application& oneCAD = onecad::app::Application::instance();
    if (!oneCAD.initialize()) {
        std::cerr << "Failed to initialize OneCAD" << std::endl;
        return 1;
    }

    std::cout << "OCCT Version: " << OCC_VERSION_COMPLETE << std::endl;

    MainWindow window;
    window.show();

    int result = app.exec();
    
    oneCAD.shutdown();
    return result;
}
