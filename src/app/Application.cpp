#include "Application.h"

#include <QCoreApplication>
#include <QLoggingCategory>

namespace onecad::app {

Q_LOGGING_CATEGORY(logApp, "onecad.app")

Application& Application::instance() {
    static Application instance;
    return instance;
}

Application::Application()
    : QObject(nullptr) {
}

Application::~Application() {
    if (m_initialized) {
        shutdown();
    }
}

bool Application::initialize() {
    if (m_initialized) {
        qCWarning(logApp) << "initialize() called when app is already initialized";
        return true;
    }

    qCInfo(logApp) << "Initializing application metadata";

    QCoreApplication::setApplicationName(appName());
    QCoreApplication::setApplicationVersion(appVersion());
    QCoreApplication::setOrganizationName(orgName());
    QCoreApplication::setOrganizationDomain(orgDomain());

    qCDebug(logApp) << "Application metadata set"
                   << "name=" << QCoreApplication::applicationName()
                   << "version=" << QCoreApplication::applicationVersion()
                   << "organization=" << QCoreApplication::organizationName()
                   << "domain=" << QCoreApplication::organizationDomain();

    m_initialized = true;
    qCInfo(logApp) << "Application initialized";
    return true;
}

void Application::shutdown() {
    if (!m_initialized) {
        qCWarning(logApp) << "shutdown() called when app is not initialized";
        return;
    }

    qCInfo(logApp) << "Application shutdown started";
    m_initialized = false;
    qCInfo(logApp) << "Application shutdown completed";
}

} // namespace onecad::app
