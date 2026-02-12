#include <QApplication>
#include <QCoreApplication>
#include <QEvent>
#include <QKeyEvent>
#include <QLoggingCategory>
#include <QMouseEvent>
#include <QObject>
#include <QSurfaceFormat>
#include <QTimer>
#include <QWheelEvent>

#include "app/Application.h"
#include "app/Logging.h"
#include "ui/mainwindow/MainWindow.h"

// Eigen3
#include <Eigen/Dense>

// OpenCASCADE (OCCT)
#include <Standard_Version.hxx>
#include <gp_Pnt.hxx>

Q_LOGGING_CATEGORY(logMain, "onecad.main")
Q_LOGGING_CATEGORY(logUiEvents, "onecad.ui.events")

namespace {

bool isEnabledFlag(const QString& value) {
    const QString normalized = value.trimmed().toLower();
    return normalized == "1" || normalized == "true" || normalized == "yes" || normalized == "on";
}

QString eventTypeName(QEvent::Type type) {
    switch (type) {
        case QEvent::MouseButtonPress:
            return QStringLiteral("MouseButtonPress");
        case QEvent::MouseButtonRelease:
            return QStringLiteral("MouseButtonRelease");
        case QEvent::MouseMove:
            return QStringLiteral("MouseMove");
        case QEvent::Wheel:
            return QStringLiteral("Wheel");
        case QEvent::KeyPress:
            return QStringLiteral("KeyPress");
        case QEvent::KeyRelease:
            return QStringLiteral("KeyRelease");
        case QEvent::Shortcut:
            return QStringLiteral("Shortcut");
        case QEvent::ShortcutOverride:
            return QStringLiteral("ShortcutOverride");
        case QEvent::ContextMenu:
            return QStringLiteral("ContextMenu");
        case QEvent::FocusIn:
            return QStringLiteral("FocusIn");
        case QEvent::FocusOut:
            return QStringLiteral("FocusOut");
        case QEvent::Enter:
            return QStringLiteral("Enter");
        case QEvent::Leave:
            return QStringLiteral("Leave");
        case QEvent::Show:
            return QStringLiteral("Show");
        case QEvent::Hide:
            return QStringLiteral("Hide");
        case QEvent::Close:
            return QStringLiteral("Close");
        case QEvent::Resize:
            return QStringLiteral("Resize");
        case QEvent::Move:
            return QStringLiteral("Move");
        case QEvent::DragEnter:
            return QStringLiteral("DragEnter");
        case QEvent::DragMove:
            return QStringLiteral("DragMove");
        case QEvent::Drop:
            return QStringLiteral("Drop");
        default:
            return QStringLiteral("EventType(%1)").arg(static_cast<int>(type));
    }
}

class UserActionEventFilter final : public QObject {
public:
    explicit UserActionEventFilter(QObject* parent = nullptr)
        : QObject(parent) {
    }

protected:
    bool eventFilter(QObject* watched, QEvent* event) override {
        if (watched == nullptr || event == nullptr) {
            return QObject::eventFilter(watched, event);
        }

        const QEvent::Type type = event->type();
        if (type == QEvent::MouseMove) {
            // Skip high-frequency move events to keep logs useful.
            return QObject::eventFilter(watched, event);
        }

        const QString objectName = watched->objectName().isEmpty()
                                       ? QStringLiteral("<unnamed>")
                                       : watched->objectName();

        qCDebug(logUiEvents).noquote()
            << "ui_event"
            << "type=" << eventTypeName(type)
            << "receiverClass=" << watched->metaObject()->className()
            << "receiverObjectName=" << objectName
            << "accepted=" << event->isAccepted();

        if (type == QEvent::KeyPress || type == QEvent::KeyRelease) {
            const auto* keyEvent = static_cast<QKeyEvent*>(event);
            qCDebug(logUiEvents).noquote()
                << "ui_key"
                << "type=" << eventTypeName(type)
                << "key=" << keyEvent->key()
                << "text=" << keyEvent->text()
                << "modifiers=" << static_cast<int>(keyEvent->modifiers())
                << "isAutoRepeat=" << keyEvent->isAutoRepeat()
                << "count=" << keyEvent->count();
        } else if (type == QEvent::MouseButtonPress || type == QEvent::MouseButtonRelease) {
            const auto* mouseEvent = static_cast<QMouseEvent*>(event);
            qCDebug(logUiEvents).noquote()
                << "ui_mouse"
                << "type=" << eventTypeName(type)
                << "button=" << static_cast<int>(mouseEvent->button())
                << "buttons=" << static_cast<int>(mouseEvent->buttons())
                << "modifiers=" << static_cast<int>(mouseEvent->modifiers())
                << "x=" << mouseEvent->position().x()
                << "y=" << mouseEvent->position().y();
        } else if (type == QEvent::Wheel) {
            const auto* wheelEvent = static_cast<QWheelEvent*>(event);
            qCDebug(logUiEvents).noquote()
                << "ui_wheel"
                << "pixelDeltaX=" << wheelEvent->pixelDelta().x()
                << "pixelDeltaY=" << wheelEvent->pixelDelta().y()
                << "angleDeltaX=" << wheelEvent->angleDelta().x()
                << "angleDeltaY=" << wheelEvent->angleDelta().y()
                << "phase=" << static_cast<int>(wheelEvent->phase())
                << "inverted=" << wheelEvent->inverted();
        }

        return QObject::eventFilter(watched, event);
    }
};

} // namespace

int main(int argc, char* argv[]) {
#ifdef NDEBUG
    constexpr bool debugBuild = false;
#else
    constexpr bool debugBuild = true;
#endif

    QCoreApplication::setApplicationName(onecad::app::Application::appName());
    QCoreApplication::setApplicationVersion(onecad::app::Application::appVersion());
    QCoreApplication::setOrganizationName(onecad::app::Application::orgName());
    QCoreApplication::setOrganizationDomain(onecad::app::Application::orgDomain());

    if (!onecad::app::Logging::initialize(onecad::app::Application::appName(), debugBuild)) {
        return 1;
    }

    qCInfo(logMain) << "Application startup initiated"
                   << "argc=" << argc
                   << "debugBuild=" << debugBuild;

    // CRITICAL: Set OpenGL format BEFORE QApplication creation
    // This ensures all QOpenGLWidgets use the correct context
    QSurfaceFormat format;
    format.setVersion(4, 1); // macOS supports up to 4.1 Core
    format.setProfile(QSurfaceFormat::CoreProfile);
    format.setSamples(4); // 4x MSAA
    format.setDepthBufferSize(24);
    format.setStencilBufferSize(8);
    format.setSwapBehavior(QSurfaceFormat::DoubleBuffer);
    QSurfaceFormat::setDefaultFormat(format);

    qCDebug(logMain) << "OpenGL default format configured"
                    << "version=" << format.majorVersion() << "." << format.minorVersion()
                    << "profile=" << format.profile()
                    << "samples=" << format.samples()
                    << "depthBufferSize=" << format.depthBufferSize()
                    << "stencilBufferSize=" << format.stencilBufferSize();

    QApplication app(argc, argv);

    UserActionEventFilter debugEventFilter;
    const bool uiEventLoggingEnabled =
        onecad::app::Logging::isDebugLoggingEnabled() &&
        isEnabledFlag(qEnvironmentVariable("ONECAD_LOG_UI_EVENTS"));
    if (uiEventLoggingEnabled) {
        app.installEventFilter(&debugEventFilter);
        qCInfo(logMain) << "Installed UI event debug logger";
    } else {
        qCDebug(logMain) << "UI event debug logger disabled";
    }

    qCInfo(logMain) << "QApplication created"
                   << "applicationDirPath=" << QApplication::applicationDirPath()
                   << "logFilePath=" << onecad::app::Logging::logFilePath();

    onecad::app::Application& oneCAD = onecad::app::Application::instance();
    if (!oneCAD.initialize()) {
        qCCritical(logMain) << "Failed to initialize OneCAD application singleton";
        onecad::app::Logging::shutdown();
        return 1;
    }

    qCInfo(logMain) << "Dependency versions"
                   << "occt=" << OCC_VERSION_COMPLETE;

    MainWindow window;
    qCDebug(logMain) << "MainWindow created";
    window.show();
    qCInfo(logMain) << "MainWindow shown; entering event loop";

    if (isEnabledFlag(qEnvironmentVariable("ONECAD_HEADLESS_SMOKE"))) {
        qCInfo(logMain) << "Headless smoke mode enabled; exiting event loop immediately";
        QTimer::singleShot(0, &app, &QApplication::quit);
    }

    const int result = app.exec();
    qCInfo(logMain) << "Qt event loop exited" << "exitCode=" << result;

    oneCAD.shutdown();
    onecad::app::Logging::shutdown();
    return result;
}
