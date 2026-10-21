// LaneThread.h — a joinable worker-lane thread with an explicit stack size.
//
// std::thread cannot choose its stack, and a secondary thread gets the platform
// default: 512 KiB on Apple platforms. That is fine for the sidecar (unchanged:
// stack 0 below means exactly that default), but the in-process library runs
// the kernel and solver lanes inside the HOST app, where a deep OCCT recursion
// (booleans, fillets, STEP) that overflows takes the app down with it. The
// library therefore asks for the same 8 MiB its own run thread gets
// (capi/onecad_worker.cpp, docs/ios/KERNEL.md).
//
// Same contract as std::thread: destroying a joinable LaneThread terminates.
#pragma once

#include <pthread.h>

#include <cstddef>
#include <exception>
#include <functional>
#include <memory>
#include <system_error>
#include <utility>

namespace onecad::util {

class LaneThread {
public:
    LaneThread() = default;

    // Starts `body` on a new thread. `stack_bytes == 0` keeps the platform
    // default; otherwise it must be a multiple of the page size. Throws
    // std::system_error when the thread cannot be created (like std::thread).
    LaneThread(std::function<void()> body, std::size_t stack_bytes) {
        auto owned = std::make_unique<std::function<void()>>(std::move(body));
        pthread_attr_t attr;
        int rc = pthread_attr_init(&attr);
        if (rc != 0) throw std::system_error(rc, std::generic_category(), "pthread_attr_init");
        if (stack_bytes != 0) rc = pthread_attr_setstacksize(&attr, stack_bytes);
        if (rc == 0) rc = pthread_create(&thread_, &attr, &LaneThread::trampoline, owned.get());
        pthread_attr_destroy(&attr);
        if (rc != 0) throw std::system_error(rc, std::generic_category(), "pthread_create");
        owned.release();  // the new thread owns the body now (trampoline deletes it)
        joinable_ = true;
    }

    LaneThread(LaneThread&& other) noexcept
        : thread_(other.thread_), joinable_(std::exchange(other.joinable_, false)) {}

    LaneThread& operator=(LaneThread&& other) noexcept {
        if (joinable_) std::terminate();  // std::thread's rule: never drop a live thread
        thread_ = other.thread_;
        joinable_ = std::exchange(other.joinable_, false);
        return *this;
    }

    LaneThread(const LaneThread&) = delete;
    LaneThread& operator=(const LaneThread&) = delete;

    ~LaneThread() {
        if (joinable_) std::terminate();
    }

    bool joinable() const noexcept { return joinable_; }

    void join() {
        if (!joinable_) return;
        const int rc = pthread_join(thread_, nullptr);
        joinable_ = false;
        if (rc != 0) throw std::system_error(rc, std::generic_category(), "pthread_join");
    }

private:
    static void* trampoline(void* arg) noexcept {
        std::unique_ptr<std::function<void()>> body(static_cast<std::function<void()>*>(arg));
        // An exception escaping a lane is a bug either way; noexcept makes it a
        // deterministic std::terminate, as an escape from a std::thread body is.
        (*body)();
        return nullptr;
    }

    pthread_t thread_{};
    bool joinable_ = false;
};

}  // namespace onecad::util
