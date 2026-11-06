#include <cstdio>
#include <memory>
#include <string>
#include <vector>

#include <BRep_Builder.hxx>
#include <TopoDS_Face.hxx>

#include "session/SourceLedger.h"

using namespace onecad;

static bool run_count(std::size_t count, bool pressure_test) {
    io::SourceClaimBudget budget(io::kSourceIdentityBudgetBytes);
    {
        auto batch = std::make_shared<io::SourceClaimBatch>();
        if (!batch->admit(count, budget)) return false;
        BRep_Builder builder;
        for (std::size_t i = 0; i < count; ++i) {
            TopoDS_Face face;
            builder.MakeFace(face);
            io::SourceFaceClaim claim;
            claim.face = face;
            claim.body_id = "body_00000000-0000-0000-0000-000000000001";
            claim.producer_id = "00000000-0000-0000-0000-000000000001";
            claim.source_sha256 = std::string(64, 'a');
            claim.codec = "step";
            claim.codec_version = "occt-step-stream-v1";
            claim.reader_policy = "step-v1";
            claim.kernel_fingerprint = std::string(16, 'a');
            claim.step_root_entity = 1;
            claim.step_face_entity = static_cast<int>(i + 1);
            if (!batch->append(std::move(claim))) return false;
        }
        session::SourceLedger ledger(budget);
        if (!ledger.append(batch)) return false;
        for (std::size_t i = 0; i < count; ++i) {
            const auto birth = ledger.rows()[i].birth;
            const auto id = "el_" + std::to_string(i);
            if (pressure_test && i + 1 == count) {
                auto hold = budget.acquire(io::kSourceIdentityBudgetBytes - budget.used());
                if (!hold || ledger.bind(id, birth) || ledger.bound(id)) return false;
            }
            if (!ledger.bind(id, birth)) return false;
        }
        auto checkpoint = ledger;
        const auto used = budget.used();
        if (!checkpoint.bound("el_0") || budget.used() != used) return false;
        if (budget.peak() > io::kSourceIdentityBudgetBytes) return false;
        std::fprintf(stderr, "source_budget: %zu faces %s, used %zu bytes, peak %zu bytes\n",
                     count, pressure_test ? "pressure" : "normal", budget.used(), budget.peak());
    }
    const bool released = budget.used() == 0;
    std::fprintf(stderr, "source_budget: %zu faces %s, released=%s\n", count,
                 pressure_test ? "pressure" : "normal", released ? "yes" : "no");
    return released;
}

int main() {
    for (const std::size_t count : {200u, 440u, 754u, 1804u}) {
        if (!run_count(count, false) || !run_count(count, true)) {
            std::fprintf(stderr, "source_budget: failed at %zu faces\n", count);
            return 1;
        }
    }
    return 0;
}
