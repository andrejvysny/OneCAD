#include "Logging.h"

#include <QCoreApplication>
#include <QDateTime>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QLoggingCategory>
#include <QMessageLogContext>
#include <QMutex>
#include <QMutexLocker>
#include <QStandardPaths>
#include <QTextStream>
#include <QThread>

#include <cstdlib>
#include <exception>
#include <iostream>

namespace onecad::app {
namespace {

QMutex gLogMutex;
QFile gLogFile;
QString gLogFilePath;
QtMessageHandler gPreviousHandler = nullptr;
bool gInitialized = false;
bool gDebugLoggingEnabled = false;
std::terminate_handler gPreviousTerminateHandler = nullptr;

constexpr int kLogRetentionDays = 30;
constexpr int kMaxRunLogFiles = 30;

const char* levelToString(QtMsgType type) {
    switch (type) {
        case QtDebugMsg:
            return "DEBUG";
        case QtInfoMsg:
            return "INFO";
        case QtWarningMsg:
            return "WARN";
        case QtCriticalMsg:
            return "ERROR";
        case QtFatalMsg:
            return "FATAL";
    }
    return "UNKNOWN";
}

bool isEnabledFlag(QStringView value) {
    const QString normalized = value.toString().trimmed().toLower();
    return normalized == "1" || normalized == "true" || normalized == "yes" || normalized == "on";
}

bool isDebugEnabledByEnvironment() {
    return isEnabledFlag(qEnvironmentVariable("ONECAD_LOG_DEBUG"));
}

QStringList selectedReleaseDebugCategories() {
    QString configured = qEnvironmentVariable("ONECAD_LOG_DEBUG_CATEGORIES").trimmed();
    if (configured.isEmpty()) {
        configured = QStringLiteral("onecad.main,onecad.app,onecad.io");
    }

    QStringList categories;
    for (const QString& token : configured.split(',', Qt::SkipEmptyParts)) {
        const QString category = token.trimmed();
        if (!category.isEmpty()) {
            categories.push_back(category);
        }
    }
    categories.removeDuplicates();
    return categories;
}

void setLoggingRules(bool debugBuild) {
    gDebugLoggingEnabled = debugBuild || isDebugEnabledByEnvironment();

    QStringList rules;
    rules << QStringLiteral("*.info=false");
    rules << QStringLiteral("default.info=true");
    rules << QStringLiteral("onecad.info=true");
    rules << QStringLiteral("onecad.*.info=true");
    rules << QStringLiteral("*.warning=true");
    rules << QStringLiteral("*.critical=true");
    rules << QStringLiteral("*.debug=false");

    if (gDebugLoggingEnabled) {
        rules << QStringLiteral("onecad.debug=true");
        rules << QStringLiteral("onecad.*.debug=true");
        if (isEnabledFlag(qEnvironmentVariable("ONECAD_LOG_QT_DEBUG"))) {
            rules << QStringLiteral("qt.*.debug=true");
        }
    } else {
        const QStringList categories = selectedReleaseDebugCategories();
        for (const QString& category : categories) {
            rules << QStringLiteral("%1.debug=true").arg(category);
            rules << QStringLiteral("%1.*.debug=true").arg(category);
        }
    }

    QLoggingCategory::setFilterRules(rules.join('\n'));
}

QString formatMessage(QtMsgType type, const QMessageLogContext& context, const QString& msg) {
    const QString timestamp = QDateTime::currentDateTime().toString(Qt::ISODateWithMs);
    const QString threadId = QString::number(reinterpret_cast<quintptr>(QThread::currentThreadId()), 16);

    const QString location = (context.file && context.line > 0)
                                 ? QStringLiteral("%1:%2").arg(context.file).arg(context.line)
                                 : QStringLiteral("<unknown>");

    const QString function = context.function ? QString::fromUtf8(context.function) : QStringLiteral("<unknown>");
    const QString category = context.category ? QString::fromUtf8(context.category) : QStringLiteral("default");

    return QStringLiteral("%1 [%2] [tid=0x%3] [%4] [%5] [%6] %7")
        .arg(timestamp,
             QString::fromLatin1(levelToString(type)),
             threadId,
             category,
             location,
             function,
             msg);
}

void writeToConsole(QtMsgType type, const QString& formatted) {
    if (type == QtWarningMsg || type == QtCriticalMsg || type == QtFatalMsg) {
        std::cerr << formatted.toStdString() << std::endl;
    } else {
        std::cout << formatted.toStdString() << std::endl;
    }
}

void messageHandler(QtMsgType type, const QMessageLogContext& context, const QString& msg) {
    const QString formatted = formatMessage(type, context, msg);

    {
        QMutexLocker lock(&gLogMutex);
        if (gLogFile.isOpen()) {
            QTextStream stream(&gLogFile);
            stream << formatted << Qt::endl;
            gLogFile.flush();
        }
    }

    writeToConsole(type, formatted);

    if (type == QtFatalMsg) {
        std::abort();
    }
}

void terminateHandler() {
    const QString timestamp = QDateTime::currentDateTime().toString(Qt::ISODateWithMs);
    const QString message = QStringLiteral("%1 [FATAL] [terminate] Unhandled exception triggered std::terminate").arg(timestamp);

    {
        QMutexLocker lock(&gLogMutex);
        if (gLogFile.isOpen()) {
            QTextStream stream(&gLogFile);
            stream << message << Qt::endl;
            gLogFile.flush();
        }
    }

    std::cerr << message.toStdString() << std::endl;

    if (gPreviousTerminateHandler != nullptr) {
        gPreviousTerminateHandler();
    }

    std::abort();
}

QStringList makeLogDirectoryPaths() {
    QStringList paths;
    auto appendUniquePath = [&paths](const QString& path) {
        const QString cleaned = QDir::cleanPath(path.trimmed());
        if (!cleaned.isEmpty() && !paths.contains(cleaned)) {
            paths.push_back(cleaned);
        }
    };

    appendUniquePath(qEnvironmentVariable("ONECAD_LOG_DIR"));

    const QString appDataPath = QStandardPaths::writableLocation(QStandardPaths::AppLocalDataLocation);
    if (!appDataPath.isEmpty()) {
        appendUniquePath(QDir(appDataPath).filePath(QStringLiteral("logs")));
    }

    appendUniquePath(QDir::current().filePath(QStringLiteral("logs")));

    const QString tempPath = QStandardPaths::writableLocation(QStandardPaths::TempLocation);
    if (!tempPath.isEmpty()) {
        appendUniquePath(QDir(tempPath).filePath(QStringLiteral("onecad/logs")));
    }

    return paths;
}

void pruneOldLogs(const QDir& dir, const QString& currentLogPath) {
    QFileInfoList logFiles = dir.entryInfoList(QStringList() << QStringLiteral("*.log"),
                                               QDir::Files,
                                               QDir::Time | QDir::Reversed);

    const QDateTime cutoff = QDateTime::currentDateTime().addDays(-kLogRetentionDays);
    for (const QFileInfo& fileInfo : logFiles) {
        if (fileInfo.absoluteFilePath() == currentLogPath) {
            continue;
        }

        if (fileInfo.lastModified().isValid() && fileInfo.lastModified() < cutoff) {
            QFile::remove(fileInfo.absoluteFilePath());
        }
    }

    logFiles = dir.entryInfoList(QStringList() << QStringLiteral("*.log"),
                                 QDir::Files,
                                 QDir::Time);

    int retained = 0;
    for (const QFileInfo& fileInfo : logFiles) {
        if (fileInfo.absoluteFilePath() == currentLogPath) {
            retained++;
            continue;
        }

        retained++;
        if (retained <= kMaxRunLogFiles) {
            continue;
        }

        QFile::remove(fileInfo.absoluteFilePath());
    }
}

} // namespace

bool Logging::initialize(const QString& appName, bool debugBuild) {
    QString initializedLogFilePath;
    QString logDirectoryPath;
    QStringList startupWarnings;

    {
        QMutexLocker lock(&gLogMutex);
        if (gInitialized) {
            return true;
        }

        setLoggingRules(debugBuild);

        const QString timestamp = QDateTime::currentDateTime().toString(QStringLiteral("yyyyMMdd_HHmmss_zzz"));
        const QString fileName = QStringLiteral("%1_%2_%3.log")
                                     .arg(appName.toLower(), timestamp)
                                     .arg(QCoreApplication::applicationPid());

        for (const QString& logDirPath : makeLogDirectoryPaths()) {
            QDir dir(logDirPath);
            if (!dir.exists() && !dir.mkpath(QStringLiteral("."))) {
                startupWarnings.push_back(QStringLiteral("Failed to create log directory: %1").arg(logDirPath));
                continue;
            }

            const QString candidatePath = dir.filePath(fileName);
            gLogFile.setFileName(candidatePath);
            if (!gLogFile.open(QIODevice::WriteOnly | QIODevice::Text)) {
                startupWarnings.push_back(QStringLiteral("Failed to open log file: %1").arg(candidatePath));
                continue;
            }

            gLogFilePath = candidatePath;
            initializedLogFilePath = gLogFilePath;
            logDirectoryPath = dir.absolutePath();
            break;
        }

        gPreviousHandler = qInstallMessageHandler(messageHandler);
        gPreviousTerminateHandler = std::set_terminate(terminateHandler);
        gInitialized = true;

        if (gLogFilePath.isEmpty()) {
            startupWarnings.push_back(QStringLiteral("File logging disabled; continuing with console-only logging"));
        }
    }

    for (const QString& warningMessage : startupWarnings) {
        qWarning().noquote() << warningMessage;
    }

    qInfo().noquote() << "Logging initialized"
                      << "logFile="
                      << (initializedLogFilePath.isEmpty()
                              ? QStringLiteral("<disabled>")
                              : QFileInfo(initializedLogFilePath).absoluteFilePath())
                      << "logDir="
                      << (logDirectoryPath.isEmpty() ? QStringLiteral("<none>") : logDirectoryPath)
                      << "debugBuild=" << debugBuild
                      << "debugLogsEnabled=" << gDebugLoggingEnabled;

    if (!initializedLogFilePath.isEmpty()) {
        pruneOldLogs(QDir(logDirectoryPath), initializedLogFilePath);
        qInfo().noquote() << "Log retention applied"
                          << "days=" << kLogRetentionDays
                          << "maxFiles=" << kMaxRunLogFiles;
    }
    return true;
}

void Logging::shutdown() {
    QString closingLogFilePath;

    {
        QMutexLocker lock(&gLogMutex);
        if (!gInitialized) {
            return;
        }
        closingLogFilePath = gLogFilePath;
    }

    qInfo().noquote() << "Logging shutdown" << "logFile=" << closingLogFilePath;

    {
        QMutexLocker lock(&gLogMutex);
        qInstallMessageHandler(gPreviousHandler);
        gPreviousHandler = nullptr;

        std::set_terminate(gPreviousTerminateHandler);
        gPreviousTerminateHandler = nullptr;

        if (gLogFile.isOpen()) {
            gLogFile.flush();
            gLogFile.close();
        }

        gLogFilePath.clear();
        gInitialized = false;
    }
}

QString Logging::logFilePath() {
    QMutexLocker lock(&gLogMutex);
    return gLogFilePath;
}

bool Logging::isDebugLoggingEnabled() {
    QMutexLocker lock(&gLogMutex);
    return gDebugLoggingEnabled;
}

} // namespace onecad::app
