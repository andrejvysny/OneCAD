// lock_probe.cpp — drill-only: hold the worker's process-wide OCCT-globals lock
// from outside, standing in for a sibling in-process worker wedged INSIDE a
// STEP call (worker/src/io/OcctStaticGuard.h). A std::mutex must be unlocked by
// the thread that locked it, so a helper thread owns it for the whole hold.
#include <condition_variable>
#include <mutex>
#include <thread>

namespace onecad::io {
std::timed_mutex& occt_globals_mutex();  // defined in libonecad_worker_lib.a
}

namespace {

std::mutex g_mu;
std::condition_variable g_cv;
bool g_held = false;
bool g_release = false;

}  // namespace

// Returns once the helper thread owns the lock. Not re-entrant.
extern "C" void onecad_spike_hold_occt_globals() {
    std::unique_lock<std::mutex> lk(g_mu);
    if (g_held) return;
    g_release = false;
    std::thread([] {
        std::lock_guard<std::timed_mutex> occt(onecad::io::occt_globals_mutex());
        std::unique_lock<std::mutex> l(g_mu);
        g_held = true;
        g_cv.notify_all();
        g_cv.wait(l, [] { return g_release; });
        g_held = false;
        g_cv.notify_all();
    }).detach();
    g_cv.wait(lk, [] { return g_held; });
}

// Returns once the helper thread released the lock.
extern "C" void onecad_spike_release_occt_globals() {
    std::unique_lock<std::mutex> lk(g_mu);
    if (!g_held) return;
    g_release = true;
    g_cv.notify_all();
    g_cv.wait(lk, [] { return !g_held; });
}
