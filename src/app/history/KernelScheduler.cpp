/**
 * @file KernelScheduler.cpp
 * @brief Implementation of kernel task scheduler.
 */
#include "KernelScheduler.h"

#include "../document/Document.h"

#include <utility>

namespace onecad::app::history {

KernelScheduler::KernelScheduler()
    : worker_(&KernelScheduler::workerLoop, this) {
}

KernelScheduler::~KernelScheduler() {
    shutdown();
}

JobId KernelScheduler::submitRegen(const RegenRequest& request, CompletionCallback callback) {
    std::lock_guard<std::mutex> lock(mutex_);
    if (stopping_) {
        return 0;
    }

    Job job;
    job.id = nextId_++;
    job.request = request;
    job.callback = std::move(callback);
    queue_.push_back(std::move(job));
    cv_.notify_one();
    return queue_.back().id;
}

void KernelScheduler::cancel(JobId id) {
    std::lock_guard<std::mutex> lock(mutex_);
    cancelled_.insert(id);
}

void KernelScheduler::shutdown() {
    {
        std::lock_guard<std::mutex> lock(mutex_);
        if (stopping_) {
            return;
        }
        stopping_ = true;
    }
    cv_.notify_all();
    if (worker_.joinable()) {
        worker_.join();
    }
}

bool KernelScheduler::isCancelledLocked(JobId id) const {
    return cancelled_.find(id) != cancelled_.end();
}

void KernelScheduler::workerLoop() {
    while (true) {
        Job job;
        {
            std::unique_lock<std::mutex> lock(mutex_);
            cv_.wait(lock, [this]() { return stopping_ || !queue_.empty(); });
            if (stopping_ && queue_.empty()) {
                return;
            }
            job = std::move(queue_.front());
            queue_.pop_front();
            if (isCancelledLocked(job.id)) {
                cancelled_.erase(job.id);
                continue;
            }
        }

        RegenJobResult output;
        output.id = job.id;

        if (!job.request.document) {
            output.result.status = RegenStatus::CriticalFailure;
        } else {
            RegenerationEngine engine(job.request.document);
            if (job.request.useAppliedCount) {
                output.result = engine.regenerateToAppliedCount(job.request.appliedOpCount);
            } else {
                output.result = engine.regenerateAll();
            }
        }

        {
            std::lock_guard<std::mutex> lock(mutex_);
            if (isCancelledLocked(job.id)) {
                cancelled_.erase(job.id);
                output.cancelled = true;
            }
        }

        if (job.callback) {
            job.callback(output);
        }
    }
}

} // namespace onecad::app::history
