#pragma once

#include <algorithm>
#include <limits>
#include <memory>
#include <stdexcept>
#include <span>
#include <vector>
#include <utility>

#include "io/SourceClaimBudget.h"
#include "io/SourceFaceClaim.h"

namespace onecad::io {

// Mutable only while an import is being prepared. Checkpoints share its final
// immutable allocation; copying a batch would evade process-wide accounting.
static_assert(sizeof(SourceFaceClaim) <= 512, "source claim budget assumes at most 512 bytes per claim");

class SourceClaimBatch {
public:
    SourceClaimBatch() = default;
    SourceClaimBatch(const SourceClaimBatch&) = delete;
    SourceClaimBatch& operator=(const SourceClaimBatch&) = delete;
    SourceClaimBatch(SourceClaimBatch&& other) {
        if (other.consumed_) throw std::logic_error("cannot move an adopted source claim batch");
        leases_ = std::move(other.leases_);
        claims_ = std::move(other.claims_);
        max_faces_ = std::exchange(other.max_faces_, 0);
        std::vector<SourceFaceClaim>().swap(other.claims_);
        std::vector<std::shared_ptr<const SourceClaimBudget::Lease>>().swap(other.leases_);
    }
    SourceClaimBatch& operator=(SourceClaimBatch&& other) {
        if (this != &other) {
            if (consumed_) throw std::logic_error("cannot replace an adopted source claim batch");
            SourceClaimBatch replacement(std::move(other));
            leases_.swap(replacement.leases_);
            claims_.swap(replacement.claims_);
            std::swap(max_faces_, replacement.max_faces_);
            std::swap(consumed_, replacement.consumed_);
        }
        return *this;
    }

    static constexpr std::size_t kMaxLabelBytes = 256;
    // Pessimistic bound for claim vectors, label maps, scoped strings and their
    // temporary duplicates while decoding. All inputs must obey the bounds.
    static constexpr std::size_t kBytesPerFace = 6u * 1024u;
    static constexpr std::size_t kBatchOverhead = 4096;

    bool admit(std::size_t faces, SourceClaimBudget& budget = source_claim_budget()) {
        if (consumed_ || faces == 0 || faces > std::numeric_limits<std::size_t>::max() - max_faces_ ||
            faces > (std::numeric_limits<std::size_t>::max() - kBatchOverhead) / kBytesPerFace)
            return false;
        auto lease = budget.acquire(kBatchOverhead + faces * kBytesPerFace);
        if (!lease) return false;
        leases_.push_back(std::move(lease));
        const auto requested = max_faces_ + faces;
        claims_.reserve(requested);
        if (!source_capacity_fits(requested, claims_.capacity())) {
            std::vector<SourceFaceClaim>().swap(claims_);
            std::vector<std::shared_ptr<const SourceClaimBudget::Lease>>().swap(leases_);
            max_faces_ = 0;
            return false;
        }
        max_faces_ = requested;
        return true;
    }
    bool append(SourceFaceClaim claim) {
        if (consumed_ || claims_.size() >= max_faces_ || !bounded(claim)) return false;
        claims_.push_back(std::move(claim));
        return true;
    }
    static bool bounded(const SourceFaceClaim& claim) {
        const auto fits = [](const std::string& value, std::size_t max) {
            return value.size() <= max && value.capacity() <= std::max<std::size_t>(32, max * 2);
        };
        return fits(claim.source_sha256, 64) && fits(claim.codec, 4) &&
               fits(claim.codec_version, 24) && fits(claim.reader_policy, 8) &&
               fits(claim.kernel_fingerprint, 16) && fits(claim.source_unit, 256) &&
               fits(claim.target_unit, 2) && fits(claim.xbf_root_label, 256) &&
               fits(claim.xbf_face_label, 256) && fits(claim.producer_id, 36) &&
               fits(claim.body_id, 62) && claim.step_occurrence_path.empty() &&
               claim.step_occurrence_path.capacity() == 0 && claim.xbf_occurrence_path.empty() &&
               claim.xbf_occurrence_path.capacity() == 0;
    }
    std::span<SourceFaceClaim> mutable_items() {
        if (consumed_) throw std::logic_error("source claim batch is immutable");
        return claims_;
    }
    std::span<const SourceFaceClaim> items() const { return claims_; }
    template <class Predicate>
    void erase_if(Predicate&& predicate) {
        if (consumed_) throw std::logic_error("source claim batch is immutable");
        std::erase_if(claims_, std::forward<Predicate>(predicate));
    }
    auto begin() { return mutable_items().begin(); }
    auto end() { return mutable_items().end(); }
    auto begin() const { return claims_.begin(); }
    auto end() const { return claims_.end(); }
    SourceFaceClaim& operator[](std::size_t index) { return mutable_items()[index]; }
    const SourceFaceClaim& front() const { return claims_.front(); }
    void clear() {
        if (consumed_) throw std::logic_error("source claim batch is immutable");
        claims_.clear();
    }
    void release_empty() {
        if (!claims_.empty()) return;
        std::vector<SourceFaceClaim>().swap(claims_);
        std::vector<std::shared_ptr<const SourceClaimBudget::Lease>>().swap(leases_);
        max_faces_ = 0;
    }
    std::size_t size() const { return claims_.size(); }
    bool empty() const { return claims_.empty(); }
    bool consumed() const { return consumed_; }
    void mark_consumed() { consumed_ = true; }

private:
    // Leases precede claims so vector/string memory is released before charges.
    std::vector<std::shared_ptr<const SourceClaimBudget::Lease>> leases_;
    std::vector<SourceFaceClaim> claims_;
    std::size_t max_faces_ = 0;
    bool consumed_ = false;
};

inline void retain_complete_source_faces(SourceClaimBatch& claims, const TopoDS_Shape& shape) {
    if (!complete_source_faces(claims.items(), shape)) claims.clear();
}

}  // namespace onecad::io
