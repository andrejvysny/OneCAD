/**
 * @file KernelScheduler.h
 * @brief Single-writer scheduler for kernel regeneration tasks.
 */
#ifndef ONECAD_APP_HISTORY_KERNELSCHEDULER_H
#define ONECAD_APP_HISTORY_KERNELSCHEDULER_H

#include "RegenerationEngine.h"

#include <cstdint>
#include <condition_variable>
#include <deque>
#include <functional>
#include <mutex>
#include <thread>
#include <unordered_set>

namespace onecad::app {
class Document;
}

namespace onecad::app::history {

using JobId = std::uint64_t;

struct RegenRequest {
    Document* document = nullptr;
    std::size_t appliedOpCount = 0;
    bool useAppliedCount = true;
};

struct RegenJobResult {
    JobId id = 0;
    RegenResult result;
    bool cancelled = false;
};

class KernelScheduler {
public:
    using CompletionCallback = std::function<void(const RegenJobResult&)>;

    KernelScheduler();
    ~KernelScheduler();

    JobId submitRegen(const RegenRequest& request, CompletionCallback callback = {});
    void cancel(JobId id);
    void shutdown();

private:
    struct Job {
        JobId id = 0;
        RegenRequest request;
        CompletionCallback callback;
    };

    void workerLoop();
    bool isCancelledLocked(JobId id) const;

    mutable std::mutex mutex_;
    std::condition_variable cv_;
    std::deque<Job> queue_;
    std::unordered_set<JobId> cancelled_;
    std::thread worker_;
    bool stopping_ = false;
    JobId nextId_ = 1;
};

} // namespace onecad::app::history

#endif // ONECAD_APP_HISTORY_KERNELSCHEDULER_H
