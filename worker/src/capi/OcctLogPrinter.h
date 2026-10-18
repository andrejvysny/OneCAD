// OcctLogPrinter.h — OCCT Message_Printer that forwards into util/Log.h
// (P0.3 in-process worker spike, ONECAD-258).
//
// In library mode (capi/onecad_worker.h) the host's log callback is the only
// diagnostic channel, so OCCT's messenger must print through Log.h instead of
// std::cerr. Every site that used to install a `Message_PrinterOStream("cerr")`
// (main.cpp's redirect, io::OcctMessengerGuard, the OCAF application's messenger)
// asks `make_diagnostic_printer()` instead: with no Log.h sink installed it still
// returns that exact cerr printer, so the sidecar's stderr bytes are unchanged.
//
// Header-only on purpose: the io/ call sites live in worker_core, and keeping the
// bridge inline avoids adding a source file to that existing target.
#pragma once

#include <Message_Gravity.hxx>
#include <Message_Printer.hxx>
#include <Message_PrinterOStream.hxx>
#include <Standard_Type.hxx>
#include <TCollection_AsciiString.hxx>

#include <string>

#include "util/Log.h"

namespace onecad::capi {

class LogBridgePrinter : public Message_Printer {
public:
    LogBridgePrinter() { SetTraceLevel(Message_Info); }

    DEFINE_STANDARD_RTTI_INLINE(LogBridgePrinter, Message_Printer)

protected:
    void send(const TCollection_AsciiString& text, const Message_Gravity gravity) const override {
        if (gravity < myTraceLevel) return;
        // Prefix every line, not just the first: OCCT texts are often
        // multi-line banners and the sink delivers them line by line.
        const std::string all(text.ToCString());
        std::size_t start = 0;
        while (start <= all.size()) {
            std::size_t end = all.find('\n', start);
            if (end == std::string::npos) end = all.size();
            const std::string line = all.substr(start, end - start);
            if (line.find_first_not_of(" \t\r") != std::string::npos) {
                ::onecad::log::emit(level_for(gravity), "occt: %s", line.c_str());
            }
            start = end + 1;
        }
    }

private:
    static ::onecad::log::Level level_for(Message_Gravity gravity) {
        switch (gravity) {
            case Message_Trace: return ::onecad::log::Level::Debug;
            case Message_Info: return ::onecad::log::Level::Info;
            case Message_Warning: return ::onecad::log::Level::Warn;
            case Message_Alarm:
            case Message_Fail: return ::onecad::log::Level::Error;
        }
        return ::onecad::log::Level::Info;
    }
};

// The printer every "route OCCT diagnostics off stdout" site installs.
// The library always gets the bridge: Log.h itself falls back to stderr when no
// sink is installed, and a printer chosen once (the OCAF application's, created
// lazily on first use) must not stay on cerr for every later run that does pass
// a log callback.
inline Handle(Message_Printer) make_diagnostic_printer() {
#ifdef ONECAD_WORKER_LIBRARY
    return new LogBridgePrinter();
#else
    if (::onecad::log::sink_installed()) return new LogBridgePrinter();
    // "cerr" is recognized by Message_PrinterOStream as the std::cerr stream.
    return new Message_PrinterOStream("cerr", false, Message_Info);
#endif
}

}  // namespace onecad::capi
