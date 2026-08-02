// OcctStaticGuard.cpp — see OcctStaticGuard.h.
#include "io/OcctStaticGuard.h"

#include <Interface_Static.hxx>
#include <Message.hxx>
#include <Message_Messenger.hxx>
#include <Message_PrinterOStream.hxx>
#include <Standard_Failure.hxx>

namespace onecad::io {

bool InterfaceStaticGuard::remember(const char* name, Kind kind) {
    for (const Saved& s : saved_) {
        if (s.name == name) return s.present;
    }
    Saved s;
    s.name = name;
    s.kind = kind;
    s.present = Interface_Static::IsPresent(name) == Standard_True;
    if (s.present) {
        switch (kind) {
            case Kind::Int:
                s.ival = static_cast<int>(Interface_Static::IVal(name));
                break;
            case Kind::Real:
                s.rval = static_cast<double>(Interface_Static::RVal(name));
                break;
            case Kind::Str: {
                // CVal returns a pointer into the knob's own storage; copy NOW,
                // before any SetCVal invalidates it.
                const Standard_CString raw = Interface_Static::CVal(name);
                s.sval = raw != nullptr ? std::string(raw) : std::string();
                break;
            }
        }
    }
    const bool present = s.present;
    saved_.push_back(std::move(s));
    return present;
}

void InterfaceStaticGuard::set_int(const char* name, int value) {
    if (remember(name, Kind::Int)) Interface_Static::SetIVal(name, value);
}

void InterfaceStaticGuard::set_real(const char* name, double value) {
    if (remember(name, Kind::Real)) Interface_Static::SetRVal(name, value);
}

void InterfaceStaticGuard::set_cstr(const char* name, const std::string& value) {
    if (remember(name, Kind::Str)) Interface_Static::SetCVal(name, value.c_str());
}

void InterfaceStaticGuard::watch_int(const char* name) { remember(name, Kind::Int); }
void InterfaceStaticGuard::watch_real(const char* name) { remember(name, Kind::Real); }
void InterfaceStaticGuard::watch_cstr(const char* name) { remember(name, Kind::Str); }

InterfaceStaticGuard::~InterfaceStaticGuard() {
    // Restore in reverse order so a knob snapshotted twice (impossible today —
    // remember() de-duplicates — stays correct if that ever changes).
    for (auto it = saved_.rbegin(); it != saved_.rend(); ++it) {
        if (!it->present) continue;
        try {
            switch (it->kind) {
                case Kind::Int:
                    Interface_Static::SetIVal(it->name.c_str(), it->ival);
                    break;
                case Kind::Real:
                    Interface_Static::SetRVal(it->name.c_str(), it->rval);
                    break;
                case Kind::Str:
                    Interface_Static::SetCVal(it->name.c_str(), it->sval.c_str());
                    break;
            }
        } catch (const Standard_Failure&) {
            // A destructor must not throw. A knob that refuses its own previous
            // value is an OCCT bug we cannot act on here.
        }
    }
}

OcctMessengerGuard::OcctMessengerGuard() {
    Handle(Message_Messenger) messenger = Message::DefaultMessenger();
    saved_ = messenger->Printers();  // deep-copies the handle sequence
    messenger->ChangePrinters().Clear();
    // "cerr" is the name Message_PrinterOStream maps to std::cerr.
    messenger->AddPrinter(new Message_PrinterOStream("cerr", Standard_False, Message_Info));
}

OcctMessengerGuard::~OcctMessengerGuard() {
    Handle(Message_Messenger) messenger = Message::DefaultMessenger();
    messenger->ChangePrinters().Assign(saved_);
}

}  // namespace onecad::io
