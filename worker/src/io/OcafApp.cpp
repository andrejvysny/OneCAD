// OcafApp.cpp — see OcafApp.h.
#include "io/OcafApp.h"

#include <BinXCAFDrivers.hxx>
#include <Message_Messenger.hxx>
#include <Message_PrinterOStream.hxx>
#include <Standard_Failure.hxx>
#include <Standard_Type.hxx>

namespace onecad::io {

std::mutex& ocaf_mutex() {
    static std::mutex m;
    return m;
}

Handle(TDocStd_Application) ocaf_application() {
    static Handle(TDocStd_Application) app;
    if (app.IsNull()) {
        app = new TDocStd_Application();
        BinXCAFDrivers::DefineFormat(app);
        // The stdout barrier — see OcafApp.h. Mirrors main.cpp's
        // redirect_occt_to_stderr(), but on the APPLICATION's own messenger, which
        // that function cannot reach.
        Handle(Message_Messenger) messenger = app->MessageDriver();
        if (!messenger.IsNull()) {
            messenger->RemovePrinters(STANDARD_TYPE(Message_PrinterOStream));
            messenger->AddPrinter(
                new Message_PrinterOStream("cerr", Standard_False, Message_Info));
        }
    }
    return app;
}

OcafDocGuard::~OcafDocGuard() {
    if (doc_.IsNull()) return;
    try {
        ocaf_application()->Close(doc_);
    } catch (const Standard_Failure&) {
        // Closing must never throw out of a destructor; a failure here leaks one
        // document, which is strictly better than terminating.
    }
}

}  // namespace onecad::io
