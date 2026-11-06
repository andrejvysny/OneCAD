#pragma once

#include <memory>
#include <optional>
#include <string>
#include <vector>

#include "io/SourceClaimBatch.h"
#include "io/SourceIdentity.h"

namespace onecad::session {

struct SourceLedgerRow {
    std::shared_ptr<const io::SourceFaceClaim> birth;
    TopoDS_Shape live_face;
};
struct SourceLedgerBinding {
    std::string element_id;
    std::shared_ptr<const io::SourceFaceClaim> birth;
};

static_assert(sizeof(SourceLedgerRow) <= 64, "source ledger row budget assumes at most 64 bytes");
static_assert(sizeof(SourceLedgerBinding) <= 64, "source ledger binding budget assumes at most 64 bytes");

// Each state owns a reservation; copies and checkpoints share immutable states.
class SourceLedger {
    struct LiveState {
        std::shared_ptr<const io::SourceClaimBudget::Lease> lease;
        std::vector<SourceLedgerRow> rows;
    };
    struct BindingState {
        std::shared_ptr<const io::SourceClaimBudget::Lease> lease;
        std::vector<SourceLedgerBinding> bindings;
    };
public:
    explicit SourceLedger(io::SourceClaimBudget budget = io::source_claim_budget()) : budget_(std::move(budget)) {}
    const std::vector<SourceLedgerRow>& rows() const {
        static const std::vector<SourceLedgerRow> empty;
        return live_ ? live_->rows : empty;
    }

    const std::vector<SourceLedgerBinding>& bindings() const {
        static const std::vector<SourceLedgerBinding> empty;
        return bindings_ ? bindings_->bindings : empty;
    }
    std::shared_ptr<const io::SourceFaceClaim> bound(const std::string& id) const {
        for (const auto& binding : bindings()) if (binding.element_id == id) return binding.birth;
        return {};
    }
    bool compatible(const std::string& id, const io::SourceFaceClaim& birth) const {
        if (id.empty() || id.size() > io::SourceClaimBatch::kMaxLabelBytes ||
            io::source_identity_json(birth).is_null()) return false;
        const SourceLedgerRow* target = nullptr;
        for (const auto& row : rows()) {
            if (row.birth.get() != &birth) continue;
            if (target) return false;
            target = &row;
        }
        if (!target) return false;
        for (const auto& binding : bindings()) {
            if (binding.element_id == id) {
                if (!io::source_claim_same_identity(*binding.birth, birth)) return false;
            } else if (io::source_claim_same_identity(*binding.birth, birth)) return false;
        }
        for (const auto& row : rows()) {
            if (row.birth.get() == &birth || row.live_face.IsNull() ||
                !row.live_face.IsSame(target->live_face)) continue;
            for (const auto& binding : bindings()) {
                if (binding.element_id != id && binding.birth.get() == row.birth.get()) return false;
            }
        }
        return true;
    }
    bool bind(const std::string& id, std::shared_ptr<const io::SourceFaceClaim> birth) {
        if (!birth || !compatible(id, *birth)) return false;
        if (bound(id)) return true;
        if (bindings().size() == std::numeric_limits<std::size_t>::max()) return false;
        auto next = allocate_bindings(bindings().size() + 1);
        if (!next) return false;
        next->bindings.insert(next->bindings.end(), bindings().begin(), bindings().end());
        next->bindings.push_back({id, std::move(birth)});
        bindings_ = std::move(next);
        return true;
    }

    std::shared_ptr<const io::SourceFaceClaim> lookup(const std::string& body,
                                                       const TopoDS_Shape& face) const {
        std::shared_ptr<const io::SourceFaceClaim> found;
        std::size_t owners = 0;
        for (const auto& row : rows()) {
            if (!row.birth || row.birth->body_id != body || row.live_face.IsNull() || face.IsNull() ||
                !row.live_face.IsSame(face)) continue;
            if (++owners > 1) return {};
            if (row.live_face.IsEqual(face)) found = row.birth;
        }
        return found;
    }

    const SourceLedgerRow* resolve(const std::string& body, const nlohmann::json& identity) const {
        const SourceLedgerRow* found = nullptr;
        for (const auto& row : rows()) {
            if (!row.birth || row.birth->body_id != body ||
                !io::source_identity_matches(*row.birth, identity)) continue;
            const auto live = lookup(body, row.live_face);
            if (!live || !io::source_identity_matches(*live, identity) || found) return nullptr;
            found = &row;
        }
        return found;
    }

    bool append(const std::shared_ptr<io::SourceClaimBatch>& batch) {
        if (!batch || batch->empty()) return true;
        if (batch->consumed()) return false;
        for (const auto& claim : batch->items()) if (claim.face.IsNull()) return false;
        if (batch->size() > std::numeric_limits<std::size_t>::max() - rows().size()) return false;
        auto next = allocate_rows(rows().size() + batch->size());
        if (!next) return false;
        next->rows.insert(next->rows.end(), rows().begin(), rows().end());
        for (auto& claim : batch->mutable_items()) {
            TopoDS_Shape live_face = std::move(claim.face);
            claim.face.Nullify();
            next->rows.push_back({std::shared_ptr<const io::SourceFaceClaim>(batch, &claim), std::move(live_face)});
        }
        batch->mark_consumed();
        live_ = std::move(next);
        return true;
    }

    template <class Mapper>
    bool remap(Mapper&& map) {
        if (!live_ || rows().empty()) return true;
        auto next = allocate_rows(rows().size());
        if (!next) return false;
        for (const auto& row : rows()) {
            const std::optional<TopoDS_Shape> image = map(row);
            if (image && !image->IsNull()) next->rows.push_back({row.birth, *image});
        }
        if (next->rows.empty()) live_.reset();
        else live_ = std::move(next);
        return true;
    }
    void clear() noexcept { live_.reset(); }

private:
    static std::optional<std::size_t> allocation_bytes(std::size_t count, std::size_t per_entry) {
        constexpr std::size_t overhead = 4096;
        if (count > (std::numeric_limits<std::size_t>::max() - overhead) / per_entry) return {};
        return overhead + count * per_entry;
    }
    std::shared_ptr<LiveState> allocate_rows(std::size_t count) const {
        const auto bytes = allocation_bytes(count, 2 * sizeof(SourceLedgerRow) + 128);
        if (!bytes) return {};
        auto lease = budget_.acquire(*bytes);
        if (!lease) return {};
        auto next = std::make_shared<LiveState>();
        next->lease = std::move(lease);
        next->rows.reserve(count);
        if (!io::source_capacity_fits(count, next->rows.capacity())) return {};
        return next;
    }
    std::shared_ptr<BindingState> allocate_bindings(std::size_t count) const {
        const auto bytes = allocation_bytes(count, 2 * sizeof(SourceLedgerBinding) + 1024);
        if (!bytes) return {};
        auto lease = budget_.acquire(*bytes);
        if (!lease) return {};
        auto next = std::make_shared<BindingState>();
        next->lease = std::move(lease);
        next->bindings.reserve(count);
        if (!io::source_capacity_fits(count, next->bindings.capacity())) return {};
        return next;
    }
    io::SourceClaimBudget budget_;
    std::shared_ptr<const LiveState> live_;
    std::shared_ptr<const BindingState> bindings_;
};

}  // namespace onecad::session
