// onecad_worker.cpp — see onecad_worker.h. P0.3 in-process spike, ONECAD-258.
#include "capi/onecad_worker.h"

#include <cstdint>
#include <exception>
#include <string>
#include <mutex>
#include <sys/socket.h>
#include <unistd.h>
#include <vector>

#include "capi/WorkerRuntime.h"
#include "io/OcctStaticGuard.h"
#include "util/LittleEndian.h"
#include "util/Log.h"

namespace {

onecad::log::Level level_from_abi(int log_level) {
    switch (log_level) {
        case ONECAD_WORKER_LOG_ERROR: return onecad::log::Level::Error;
        case ONECAD_WORKER_LOG_WARN: return onecad::log::Level::Warn;
        case ONECAD_WORKER_LOG_DEBUG: return onecad::log::Level::Debug;
        default: return onecad::log::Level::Info;
    }
}

void set_no_sigpipe(int fd) {
#ifdef SO_NOSIGPIPE
    int on = 1;
    // ENOTSOCK for a pipe is fine: a pipe host must ignore SIGPIPE itself.
    (void)::setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &on, sizeof(on));
#else
    (void)fd;
#endif
}

// Every live library run's (callback, ctx), oldest first, guarded by Log.h's
// log_mutex(). The process-global Log.h sink always points at the NEWEST live
// run; when a run returns it drops its own entry and the sink falls back to the
// newest survivor (or stderr when none is left). So a callback is only ever
// invoked while its own run is still inside onecad_worker_run — the window in
// which the ABI guarantees its ctx is alive — and a surviving run never loses
// its lines to stderr because a sibling finished first.
std::vector<onecad::log::SinkSlot>& live_sinks() {
    static std::vector<onecad::log::SinkSlot> v;
    return v;
}

class SinkScope {
public:
    SinkScope(onecad_worker_log_fn fn, void* ctx) : slot_{fn, ctx} {
        if (slot_.fn == nullptr) return;
        std::lock_guard<std::mutex> guard(onecad::log::log_mutex());
        live_sinks().push_back(slot_);
        onecad::log::sink_storage() = slot_;
    }
    ~SinkScope() {
        if (slot_.fn == nullptr) return;
        std::lock_guard<std::mutex> guard(onecad::log::log_mutex());
        auto& v = live_sinks();
        for (auto it = v.rbegin(); it != v.rend(); ++it) {
            if (it->fn == slot_.fn && it->ctx == slot_.ctx) {
                v.erase(std::next(it).base());
                break;
            }
        }
        onecad::log::sink_storage() = v.empty() ? onecad::log::SinkSlot{} : v.back();
    }
    SinkScope(const SinkScope&) = delete;
    SinkScope& operator=(const SinkScope&) = delete;

private:
    onecad::log::SinkSlot slot_;
};

onecad_worker_journal_event journal_event(int kind, std::uint64_t id, const std::string& name,
                                          const std::string& fingerprint) {
    onecad_worker_journal_event ev{};
    ev.struct_size = sizeof(ev);
    ev.event = kind;
    ev.request_id = id;
    ev.name = name.c_str();
    ev.prefix_hash = "";
    ev.occt_fingerprint = fingerprint.c_str();
    return ev;
}

onecad::protocol::JournalHooks make_journal_hooks(onecad_worker_journal_fn fn, void* ctx) {
    onecad::protocol::JournalHooks hooks;
    if (fn == nullptr) return hooks;
    // The hello's fingerprint: the third component of the crash-circuit key.
    const std::string fingerprint =
        onecad::worker::make_hello_result()["occt"]["fingerprint"].get<std::string>();
    hooks.job_begin = [fn, ctx, fingerprint](std::uint64_t id, const std::string& verb) {
        const auto ev = journal_event(ONECAD_WORKER_JOURNAL_JOB_BEGIN, id, verb, fingerprint);
        fn(&ev, ctx);
    };
    hooks.plan_op_begin = [fn, ctx, fingerprint](std::uint64_t id, std::uint64_t step,
                                                 std::size_t position, const std::string& op,
                                                 const std::string& prefix_hash) {
        auto ev = journal_event(ONECAD_WORKER_JOURNAL_PLAN_OP_BEGIN, id, op, fingerprint);
        ev.step_index = step;
        ev.op_position = position;
        ev.prefix_hash = prefix_hash.c_str();
        fn(&ev, ctx);
    };
    hooks.job_end = [fn, ctx, fingerprint](std::uint64_t id, const std::string& verb) {
        const auto ev = journal_event(ONECAD_WORKER_JOURNAL_JOB_END, id, verb, fingerprint);
        fn(&ev, ctx);
    };
    return hooks;
}

void close_valid(int in_fd, int out_fd) {
    if (in_fd >= 0) ::close(in_fd);
    if (out_fd >= 0 && out_fd != in_fd) ::close(out_fd);
}

int run_worker(const onecad_worker_config& cfg) {
    const int in_fd = cfg.in_fd;
    const int out_fd = cfg.out_fd;
    if (in_fd < 0 || out_fd < 0 || in_fd == out_fd) {
        close_valid(in_fd, out_fd);
        return ONECAD_WORKER_EXIT_INTERNAL;
    }
    SinkScope sink(cfg.log_cb, cfg.log_ctx);
    onecad::log::set_level(level_from_abi(cfg.log_level));
    onecad::io::set_occt_lock_wait_ms(cfg.occt_lock_wait_ms);
    set_no_sigpipe(out_fd);
    // Until run_dispatch_loop owns the fds (from its first statement it closes
    // both on every exit, exceptional ones included), an exception here must
    // close them: otherwise the host end never sees EOF or a hello.
    bool handed_off = false;
    // No C++ exception may cross into a C/Rust caller.
    try {
        if (!onecad::endian::host_is_little_endian()) {
            WLOG_ERROR("fatal: host is not little-endian; worker requires LE");
            close_valid(in_fd, out_fd);
            return 2;
        }
        onecad::worker::route_occt_diagnostics();
        onecad::protocol::JournalHooks hooks = make_journal_hooks(cfg.journal_cb, cfg.journal_ctx);
        handed_off = true;
        return onecad::worker::run_dispatch_loop(in_fd, out_fd, /*close_fds=*/true,
                                                 std::move(hooks));
    } catch (const std::exception& ex) {
        WLOG_ERROR("onecad_worker_run: escaped exception: %s", ex.what());
    } catch (...) {
        WLOG_ERROR("onecad_worker_run: escaped non-std exception");
    }
    if (!handed_off) close_valid(in_fd, out_fd);
    return ONECAD_WORKER_EXIT_INTERNAL;
}

}  // namespace

extern "C" int onecad_worker_abi_version(void) { return ONECAD_WORKER_ABI_VERSION; }

extern "C" int onecad_worker_run(int in_fd, int out_fd, onecad_worker_log_fn log_cb, void* ctx,
                                 int log_level) {
    onecad_worker_config cfg{};
    cfg.struct_size = sizeof(cfg);
    cfg.in_fd = in_fd;
    cfg.out_fd = out_fd;
    cfg.log_cb = log_cb;
    cfg.log_ctx = ctx;
    cfg.log_level = log_level;
    return run_worker(cfg);
}

extern "C" int onecad_worker_run_config(const onecad_worker_config* config) {
    if (config == nullptr) return ONECAD_WORKER_EXIT_INTERNAL;
    if (config->struct_size < sizeof(onecad_worker_config)) {
        close_valid(config->in_fd, config->out_fd);
        return ONECAD_WORKER_EXIT_INTERNAL;
    }
    return run_worker(*config);
}
