// cout_bridge.cpp — routes C++ std::cout into the host's log callback.
//
// OCCT writes some diagnostics to std::cout DIRECTLY, bypassing
// Message_Messenger (BinTools_ShapeSet::Read's "File was not written with this
// version of the topology", the path worker/src/io/BrepCodec.cpp pre-empts, and
// ~10 other TKBRep files in 8.0.1). In the sidecar fd 1 is the frame channel;
// in library mode it is the HOST's stdout, so those lines would bypass the log
// callback. This swaps std::cout's streambuf for one that forwards whole lines.
//
// It lives in the spike, not in worker/src/capi, ONLY because
// scripts/check-worker-stdout-hygiene.sh bans the identifier `std::cout` under
// worker/src (a gate scoped to the sidecar's fd-1 invariant). Moving it into the
// library needs a one-line exemption for this file; see the spike doc.
#include <iostream>
#include <mutex>
#include <streambuf>
#include <string>

namespace {

using LogFn = void (*)(int level, const char* msg, void* ctx);
constexpr int kLevelInfo = 2;

class LineForwardBuf : public std::streambuf {
public:
    void set_target(LogFn fn, void* ctx) {
        std::lock_guard<std::mutex> lock(mu_);
        fn_ = fn;
        ctx_ = ctx;
    }

protected:
    int overflow(int ch) override {
        if (ch == traits_type::eof()) return traits_type::not_eof(ch);
        std::lock_guard<std::mutex> lock(mu_);
        if (ch == '\n') {
            flush_line();
        } else {
            line_.push_back(static_cast<char>(ch));
        }
        return ch;
    }

    std::streamsize xsputn(const char* s, std::streamsize n) override {
        for (std::streamsize i = 0; i < n; ++i) overflow(static_cast<unsigned char>(s[i]));
        return n;
    }

    int sync() override {
        std::lock_guard<std::mutex> lock(mu_);
        if (!line_.empty()) flush_line();
        return 0;
    }

private:
    void flush_line() {
        if (fn_ != nullptr) {
            const std::string msg = "cout: " + line_;
            fn_(kLevelInfo, msg.c_str(), ctx_);
        }
        line_.clear();
    }

    std::mutex mu_;
    std::string line_;
    LogFn fn_ = nullptr;
    void* ctx_ = nullptr;
};

LineForwardBuf& bridge() {
    static LineForwardBuf buf;
    return buf;
}

}  // namespace

// Process-global and idempotent; the host installs it once at start. A null
// `fn` keeps the bridge installed but drops lines.
extern "C" void onecad_spike_route_cout(LogFn fn, void* ctx) {
    bridge().set_target(fn, ctx);
    std::cout.rdbuf(&bridge());
}

// Writes one line through std::cout so the host can assert the bridge works.
extern "C" void onecad_spike_cout_selftest(const char* text) {
    std::cout << text << std::endl;
}
