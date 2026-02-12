#ifndef ONECAD_APP_LOGGING_H
#define ONECAD_APP_LOGGING_H

#include <QString>

namespace onecad::app {

class Logging {
public:
    static bool initialize(const QString& appName, bool debugBuild);
    static void shutdown();
    static QString logFilePath();
    static bool isDebugLoggingEnabled();

private:
    Logging() = delete;
};

} // namespace onecad::app

#endif // ONECAD_APP_LOGGING_H
