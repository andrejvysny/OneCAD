#ifndef ONECAD_APP_APPLICATION_H
#define ONECAD_APP_APPLICATION_H

#include <QObject>
#include <QString>

namespace onecad {
namespace app {

/**
 * @brief Main application controller for OneCAD.
 * 
 * Manages document lifecycle, global state, and coordinates
 * between UI, core CAD engine, and file I/O subsystems.
 */
class Application : public QObject {
    Q_OBJECT

public:
    static Application& instance();

    bool initialize();
    void shutdown();

    // Application metadata
    static QString appName() { return QStringLiteral("OneCAD"); }
    static QString appVersion() { return QStringLiteral("0.1.0"); }
    static QString orgName() { return QStringLiteral("OneCAD"); }
    static QString orgDomain() { return QStringLiteral("onecad.app"); }

private:
    Application();
    ~Application();
    
    Application(const Application&) = delete;
    Application& operator=(const Application&) = delete;

    bool m_initialized = false;
};

} // namespace app
} // namespace onecad

#endif // ONECAD_APP_APPLICATION_H
