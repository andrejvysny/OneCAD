#pragma once

#include <atomic>
#include <cstddef>
#include <memory>
#include <limits>

namespace onecad::io {

// Independent of geometry limits: this bounds source identity metadata retained
// by all sessions and checkpoint generations in one worker process.
inline constexpr std::size_t kSourceIdentityBudgetBytes = 16u * 1024u * 1024u;

inline bool source_capacity_fits(std::size_t requested, std::size_t actual) {
    if (requested > std::numeric_limits<std::size_t>::max() / 2) return false;
    return actual <= requested * 2;
}

class SourceClaimBudget {
    struct State {
        explicit State(std::size_t maximum) : limit(maximum) {}
        std::atomic<std::size_t> used{0};
        std::atomic<std::size_t> peak{0};
        const std::size_t limit;
    };
public:
    class Lease {
    public:
        Lease(std::shared_ptr<State> state, std::size_t bytes)
            : state_(std::move(state)), bytes_(bytes) {}
        ~Lease() { state_->used.fetch_sub(bytes_, std::memory_order_relaxed); }
        Lease(const Lease&) = delete;
        Lease& operator=(const Lease&) = delete;
        std::size_t bytes() const { return bytes_; }
    private:
        std::shared_ptr<State> state_;
        std::size_t bytes_;
    };

    explicit SourceClaimBudget(std::size_t limit) : state_(std::make_shared<State>(limit)) {}
    std::shared_ptr<const Lease> acquire(std::size_t bytes) const {
        auto current = state_->used.load(std::memory_order_relaxed);
        do {
            if (current > state_->limit || bytes > state_->limit - current) return {};
        } while (!state_->used.compare_exchange_weak(current, current + bytes,
                                                     std::memory_order_relaxed));
        const auto now = current + bytes;
        auto peak = state_->peak.load(std::memory_order_relaxed);
        while (peak < now && !state_->peak.compare_exchange_weak(peak, now, std::memory_order_relaxed)) {}
        try {
            return std::make_shared<const Lease>(state_, bytes);
        } catch (...) {
            state_->used.fetch_sub(bytes, std::memory_order_relaxed);
            throw;
        }
    }
    std::size_t used() const { return state_->used.load(std::memory_order_relaxed); }
    std::size_t peak() const { return state_->peak.load(std::memory_order_relaxed); }
private:
    std::shared_ptr<State> state_;
};

inline SourceClaimBudget& source_claim_budget() {
    static SourceClaimBudget budget(kSourceIdentityBudgetBytes);
    return budget;
}

}  // namespace onecad::io
