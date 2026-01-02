#include "Application.h"
#include <QCoreApplication>
#include <iostream>

namespace onecad {
namespace app {

Application& Application::instance() {
    static Application instance;
    return instance;
}

Application::Application() : QObject(nullptr) {
}

Application::~Application() {
    if (m_initialized) {
        shutdown();
    }
}

bool Application::initialize() {
    if (m_initialized) {
        return true;
    }

    // Set application metadata
    QCoreApplication::setApplicationName(appName());
    QCoreApplication::setApplicationVersion(appVersion());
    QCoreApplication::setOrganizationName(orgName());
    QCoreApplication::setOrganizationDomain(orgDomain());

    std::cout << "OneCAD Application initialized" << std::endl;
    
    m_initialized = true;
    return true;
}

void Application::shutdown() {
    if (!m_initialized) {
        return;
    }

    std::cout << "OneCAD Application shutdown" << std::endl;
    m_initialized = false;
}

} // namespace app
} // namespace onecad
