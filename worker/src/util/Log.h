// Log.h — stderr-only logging for the OneCAD worker.
//
// HARD INVARIANT: the worker's stdout carries protocol frames ONLY. Every
// diagnostic MUST go to stderr. These macros never touch stdout. Do not add a
// stdout sink here. scripts/check-worker-stdout-hygiene.sh — run by
// scripts/build-worker.sh and by CI's worker job — greps worker/src for
// printf/puts/putchar/std::cout/fprintf(stdout, ...) and fails the build on
// a hit.
#pragma once

#include <atomic>
#include <cctype>
#include <cstdarg>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <ctime>
#include <mutex>
#include <string>
#include <string_view>

namespace onecad::log {

enum class Level { Error = 0, Warn = 1, Info = 2, Debug = 3 };

inline std::string_view level_name(Level l) {
    switch (l) {
        case Level::Error: return "ERROR";
        case Level::Warn:  return "WARN";
        case Level::Info:  return "INFO";
        case Level::Debug: return "DEBUG";
    }
    return "?";
}

// Minimum level that emit() will actually print, set once from
// ONECAD_WORKER_LOG via init_level_from_env(). Atomic: the stdin reader
// thread and kernel/solver threads all log concurrently. Defaults to Info so
// a worker that never calls init_level_from_env() (e.g. a unit test binary)
// still behaves sanely.
inline std::atomic<int>& min_level_storage() {
    static std::atomic<int> lvl{static_cast<int>(Level::Info)};
    return lvl;
}

// True if a message at `l` should be printed given the current min level.
// Levels are ordered Error < Warn < Info < Debug (increasing verbosity), so a
// message is enabled when it is at least as severe as (numerically <=) the
// threshold.
inline bool enabled(Level l) {
    return static_cast<int>(l) <= min_level_storage().load(std::memory_order_relaxed);
}

// Serializes concurrent log lines (stdin reader thread + kernel thread both log).
inline std::mutex& log_mutex() {
    static std::mutex m;
    return m;
}

// Millisecond-precision UTC-ish local timestamp: "YYYY-MM-DD HH:MM:SS.mmm".
inline void write_timestamp(char* buf, size_t n) {
    using namespace std::chrono;
    const auto now = system_clock::now();
    const auto secs = time_point_cast<seconds>(now);
    const auto ms = duration_cast<milliseconds>(now - secs).count();
    const std::time_t t = system_clock::to_time_t(now);
    std::tm tm_buf{};
#if defined(_WIN32)
    localtime_s(&tm_buf, &t);
#else
    localtime_r(&t, &tm_buf);
#endif
    char date[20];
    std::strftime(date, sizeof(date), "%Y-%m-%d %H:%M:%S", &tm_buf);
    std::snprintf(buf, n, "%s.%03lld", date, static_cast<long long>(ms));
}

// Single entry point. Always writes to stderr and flushes. printf-style; the
// format(printf) attribute enables compile-time format checking and avoids the
// -Wformat-security warning a non-literal fprintf passthrough would raise.
// enabled() is checked FIRST, before touching the mutex or formatting the
// timestamp, so a filtered-out WLOG_DEBUG in a hot path costs one atomic load.
__attribute__((format(printf, 2, 3))) inline void emit(Level lvl, const char* fmt, ...) {
    if (!enabled(lvl)) {
        return;
    }
    std::lock_guard<std::mutex> guard(log_mutex());
    char ts[32];
    write_timestamp(ts, sizeof(ts));
    std::fprintf(stderr, "[%s] %-5s ", ts, level_name(lvl).data());
    std::va_list ap;
    va_start(ap, fmt);
    std::vfprintf(stderr, fmt, ap);
    va_end(ap);
    std::fputc('\n', stderr);
    std::fflush(stderr);
}

// Reads ONECAD_WORKER_LOG once (error|warn|info|debug, case-insensitive) and
// sets the process-wide min level. Unset/empty => Info (unchanged default).
// An unrecognized value also falls back to Info, plus a single WLOG_WARN so a
// typo'd env var is visible instead of silently under/over-logging. Call this
// exactly once, as the first statement of main() — logging before this point
// (there is none today) would run at the Info default anyway.
inline void init_level_from_env() {
    const char* raw = std::getenv("ONECAD_WORKER_LOG");
    if (raw == nullptr || raw[0] == '\0') {
        min_level_storage().store(static_cast<int>(Level::Info), std::memory_order_relaxed);
        return;
    }

    std::string v(raw);
    for (char& c : v) {
        c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
    }

    Level lvl = Level::Info;
    bool known = true;
    if (v == "error") {
        lvl = Level::Error;
    } else if (v == "warn") {
        lvl = Level::Warn;
    } else if (v == "info") {
        lvl = Level::Info;
    } else if (v == "debug") {
        lvl = Level::Debug;
    } else {
        known = false;
    }

    min_level_storage().store(static_cast<int>(lvl), std::memory_order_relaxed);
    if (!known) {
        // Call emit() directly (not the WLOG_WARN macro): the macro isn't
        // defined until below this function in the header.
        emit(Level::Warn, "unrecognized ONECAD_WORKER_LOG='%s'; defaulting to info", raw);
    }
}

}  // namespace onecad::log

// Usage: WLOG_INFO("hello %s", name);  — printf-style, newline appended.
#define WLOG_INFO(...)  ::onecad::log::emit(::onecad::log::Level::Info,  __VA_ARGS__)
#define WLOG_WARN(...)  ::onecad::log::emit(::onecad::log::Level::Warn,  __VA_ARGS__)
#define WLOG_ERROR(...) ::onecad::log::emit(::onecad::log::Level::Error, __VA_ARGS__)
#define WLOG_DEBUG(...) ::onecad::log::emit(::onecad::log::Level::Debug, __VA_ARGS__)
