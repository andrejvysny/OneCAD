// Session.cpp — see Session.h.
#include "session/Session.h"

#include <algorithm>
#include <cctype>
#include <utility>

#include "ops/GearOp.h"
#include "session/ClassifyElement.h"
#include "session/HistoryHash.h"
#include "util/Log.h"

namespace onecad::session {

using protocol::ErrorInfo;

namespace {
ErrorInfo protocol_error(std::string message, nlohmann::json detail) {
    return ErrorInfo{"PROTOCOL_ERROR", std::move(message), /*retriable=*/false,
                     std::move(detail)};
}

ErrorInfo bind_error(std::string code, std::string message, std::size_t index) {
    return ErrorInfo{std::move(code), "BindElementIds: " + std::move(message), false,
                     nlohmann::json{{"bindingIndex", index}}};
}

// SCHEMA §7.3: the gear-referenceability refusal. Same `REF_UNRESOLVED` class
// (and same `detail.bindingIndex`) as every other Bind refusal, plus the
// structured evidence that says WHICH sub-element of WHICH gear was refused.
//
// A FACE refusal reports its `surfaceType`; an EDGE or VERTEX refusal reports
// `kind` in its place, because what decided it is the NEIGHBOURHOOD, not the
// element's own geometry.
ErrorInfo gear_bind_refusal(const ElementBindingInput& binding, const std::string& gear_op_id,
                            const TopoDS_Shape& shape, std::size_t index) {
    const bool is_face = !shape.IsNull() && shape.ShapeType() == TopAbs_FACE;
    nlohmann::json evidence = {{"bodyId", binding.body_id},
                               {"topoKey", binding.topo_key},
                               {"gearOpId", gear_op_id}};
    std::string message;
    if (is_face) {
        const std::string surface_type =
            classify_shape(shape).value("surfaceType", std::string("other"));
        evidence["surfaceType"] = surface_type;
        message = "gear face " + binding.topo_key + " of " + binding.body_id +
                  " is not referenceable (surfaceType '" + surface_type +
                  "'): only the two caps and a bore below the root radius carry identity a "
                  "tooth-count edit preserves";
    } else {
        evidence["kind"] = binding.kind;
        message = "gear " + binding.kind + " " + binding.topo_key + " of " + binding.body_id +
                  " is not referenceable: an edge or vertex carries identity a tooth-count edit "
                  "preserves only where EVERY adjacent face does";
    }
    ErrorInfo error = bind_error("REF_UNRESOLVED", std::move(message), index);
    (*error.detail)["diagnostics"] = nlohmann::json::array({nlohmann::json{
        {"code", "REF_UNRESOLVED"},
        {"reasonCode", "GEAR_FACE_NOT_REFERENCEABLE"},
        {"evidence", std::move(evidence)}}});
    return error;
}

bool valid_element_id(const std::string& id) {
    if (!id.starts_with("el_") || id.size() == 3) return false;
    return std::all_of(id.begin() + 3, id.end(), [](unsigned char c) {
        return std::isalnum(c) || c == '_' || c == '-';
    });
}

char prefix_for_kind(const std::string& kind) {
    if (kind == "face") return 'f';
    if (kind == "edge") return 'e';
    if (kind == "vertex") return 'v';
    return 0;
}

bool valid_topo_key(const std::string& key, char prefix) {
    if (key.size() < 3 || key[0] != prefix || key[1] != ':' || key[2] == '0') return false;
    return std::all_of(key.begin() + 2, key.end(), [](unsigned char c) {
        return std::isdigit(c);
    });
}

const elementmap::PartitionEntry* binding_at(
    const elementmap::ElementMapPartition& partition, const std::string& body_id,
    const std::string& topo_key) {
    for (const auto* entry : partition.entries_for_body(body_id)) {
        if (entry->topo_key == topo_key) return entry;
    }
    return nullptr;
}

std::optional<ErrorInfo> stage_binding(const BodyStore& bodies,
                                       elementmap::ElementMapPartition& staged,
                                       const std::map<std::string, ops::GearBodyInfo>& gear_bodies,
                                       const ElementBindingInput& binding, std::size_t index) {
    if (binding.body_id.empty() || !valid_element_id(binding.element_id))
        return bind_error("PROTOCOL_ERROR", "malformed bodyId or elementId", index);
    const char prefix = prefix_for_kind(binding.kind);
    if (prefix == 0 || !valid_topo_key(binding.topo_key, prefix))
        return bind_error("PROTOCOL_ERROR", "kind and topoKey do not match", index);
    const BodyRecord* body = bodies.get(binding.body_id);
    if (!body) return bind_error("REF_UNRESOLVED", "body not found", index);
    const TopoDS_Shape shape = elementmap::ElementMapPartition::shape_for_topokey(
        body->geom, binding.topo_key);
    if (shape.IsNull()) return bind_error("REF_UNRESOLVED", "topoKey not found", index);

    // SCHEMA §7.3 gear referenceability (kernel-hardening WP-I). This is the MINT
    // — `AcquireElementIds` mints nothing and passes the pick through unchanged,
    // so the by-name refusal a client sees at pick time is the Bind that follows.
    // Nothing is staged and the head is untouched. Faces, edges and vertices all
    // go through the one classifier; a single Bind classifies ONE shape, so the
    // throwaway adjacency map it builds is the right trade here.
    if (const auto gear = gear_bodies.find(binding.body_id);
        gear != gear_bodies.end() &&
        !ops::gear_element_referenceable(gear->second, body->geom, shape)) {
        return gear_bind_refusal(binding, gear->second.gear_op_id, shape, index);
    }

    const auto kind = elementmap::ElementMapPartition::kind_from_name(binding.kind);
    const auto* by_id = staged.find(binding.element_id);
    const auto* by_topo = binding_at(staged, binding.body_id, binding.topo_key);
    if (by_id && (by_id->body_id != binding.body_id || by_id->topo_key != binding.topo_key ||
                  by_id->kind != kind))
        return bind_error("REF_UNRESOLVED", "elementId conflicts with existing binding", index);
    if (by_topo && by_topo->element_id != binding.element_id)
        return bind_error("REF_UNRESOLVED", "topoKey conflicts with existing binding", index);
    if (!by_id) staged.mint(binding.body_id, binding.element_id, kind, shape, body->geom,
                            binding.anchor);
    return std::nullopt;
}
}  // namespace

void Session::open(std::string document_id, std::uint64_t document_revision,
                   std::uint64_t worker_epoch, std::string mode) {
    std::lock_guard<std::mutex> lk(mu_);
    open_ = true;
    document_id_ = std::move(document_id);
    document_revision_ = document_revision;
    worker_epoch_ = worker_epoch;
    snapshot_id_ = 0;
    history_prefix_hash_ = kEmptyPrefixHash;  // fresh document ⇒ empty-prefix anchor
    mode_ = std::move(mode);
    bodies_ = BodyStore{};
    partition_ = elementmap::ElementMapPartition{};
    gear_bodies_.clear();
    sketches_.clear();
    scratch_.reset();
    snapshot_counter_ = 0;
    checkpoints_.clear();
}

void Session::close() {
    std::lock_guard<std::mutex> lk(mu_);
    open_ = false;
    // Fencing tokens left as last-seen so a late stamp stays consistent; a fresh
    // OpenSession resets everything.
}

std::uint64_t Session::reset() {
    std::lock_guard<std::mutex> lk(mu_);
    // Drop all session + scratch state; bump the epoch; keep the process alive.
    open_ = false;
    document_id_.clear();
    document_revision_ = 0;
    snapshot_id_ = 0;
    history_prefix_hash_ = kEmptyPrefixHash;
    bodies_ = BodyStore{};
    partition_ = elementmap::ElementMapPartition{};
    gear_bodies_.clear();
    sketches_.clear();
    scratch_.reset();
    snapshot_counter_ = 0;
    checkpoints_.clear();  // in-session cache dropped on restart (Invariant 7 replay)
    worker_epoch_ += 1;  // Rust echoes the new epoch in subsequent requests.
    return worker_epoch_;
}

bool Session::is_open() const {
    std::lock_guard<std::mutex> lk(mu_);
    return open_;
}

protocol::Stamp Session::head_stamp() const {
    std::lock_guard<std::mutex> lk(mu_);
    protocol::Stamp s;
    s.document_revision = document_revision_;
    s.worker_epoch = worker_epoch_;
    s.snapshot_id = snapshot_id_;
    return s;
}

WorkerHead Session::head() const {
    std::lock_guard<std::mutex> lk(mu_);
    WorkerHead h;
    h.document_revision = document_revision_;
    h.worker_epoch = worker_epoch_;
    h.snapshot_id = snapshot_id_;
    h.history_prefix_hash = history_prefix_hash_;
    h.has_scratch = scratch_.has_value();
    return h;
}

bool Session::has_scratch() const {
    std::lock_guard<std::mutex> lk(mu_);
    return scratch_.has_value();
}

FenceOutcome Session::fence_and_clone(std::uint64_t job_id,
                                      std::uint64_t /*document_revision*/,  // D4: advisory, not fenced
                                      std::uint64_t worker_epoch,
                                      const std::string& expected_base_hash) {
    std::lock_guard<std::mutex> lk(mu_);
    FenceOutcome out;

    if (!open_) {
        out.status = FenceOutcome::Status::Error;
        out.error = protocol_error("ExecutePlan: no open session", nlohmann::json::object());
        return out;
    }

    // One scratch at a time (SCHEMA §7.2). A re-sent SAME jobId while prepared is
    // idempotent (re-return the cached PlanPrepared); a DIFFERENT jobId is a
    // PROTOCOL_ERROR (reject-and-report — see the W-WP4 report / SCHEMA changelog).
    if (scratch_.has_value()) {
        if (scratch_->job_id == job_id) {
            out.status = FenceOutcome::Status::IdempotentPrepared;
            out.idempotent_result = scratch_->prepared_result;
            return out;
        }
        out.status = FenceOutcome::Status::Error;
        out.error = protocol_error(
            "ExecutePlan: a plan is already prepared; accept or discard it first",
            nlohmann::json{{"preparedJobId", scratch_->job_id}, {"requestedJobId", job_id}});
        return out;
    }

    // Fencing: ONLY workerEpoch gates a plan (D4). documentRevision is a Rust-owned
    // advisory stamp (an edit counter) — the worker MUST NOT reject on it, because a
    // post-edit regen legitimately carries a documentRevision ahead of the worker's
    // last-accepted head. The plan's documentRevision is stored in the scratch and
    // adopted as the head at AcceptPrepared. Epoch mismatch ⇒ PROTOCOL_ERROR (Rust
    // reconciles via GetWorkerHead / restart).
    if (worker_epoch != worker_epoch_) {
        out.status = FenceOutcome::Status::Error;
        out.error = protocol_error(
            "ExecutePlan: workerEpoch fencing mismatch",
            nlohmann::json{{"headEpoch", worker_epoch_}, {"planEpoch", worker_epoch}});
        return out;
    }

    // Fencing: expectedBaseHash must equal the head's historyPrefixHash — EXCEPT for
    // a from-0 plan (D5). A from-0 plan is one with NO base checkpoint AND
    // expectedBaseHash == the empty-prefix anchor (kEmptyPrefixHash).
    //
    // `ExecutePlan` does not read `baseCheckpoint` — a restore is a SEPARATE verb
    // (io/Checkpoint.cpp `RestoreCheckpoint`, which Rust calls before the plan) —
    // so "no base checkpoint" is not something this function can observe and a
    // from-0 plan is identified here purely by its empty-anchor expectedBaseHash.
    // (An earlier comment claimed checkpoints were UNSUPPORTED in V1. They are not:
    // they are plumbed end to end, and a plan whose restore failed arrives here as
    // an ordinary from-0 plan carrying `checkpointFallbackReplay` — see §7.2.)
    //
    // D5: a from-0 plan is ALWAYS base-valid — its base IS empty by definition, so
    // the precondition is satisfiable regardless of the head. The RegenPlanner always
    // emits full-replay-from-0 plans; after the first AcceptPrepared the head token is
    // nonzero, so the strict head-hash fence would reject every subsequent regen (the
    // sequential-regen blocker). For a from-0 plan the worker SKIPS the head-hash
    // comparison and clones an EMPTY base below (discarding the prior head's bodies /
    // partition from the scratch's starting state); accept then REPLACES the head
    // wholesale. Incremental plans (expectedBaseHash != empty anchor) keep the strict
    // head-hash fence exactly as before. workerEpoch fencing (above) and all
    // AcceptPrepared/DiscardPrepared fencing are unchanged. Detail carries
    // {expected, actual} for Rust reconciliation (SCHEMA §7.2).
    const bool from_zero = (expected_base_hash == kEmptyPrefixHash);
    if (!from_zero && expected_base_hash != history_prefix_hash_) {
        out.status = FenceOutcome::Status::Error;
        out.error = protocol_error(
            "ExecutePlan: expectedBaseHash mismatch",
            nlohmann::json{{"expected", expected_base_hash}, {"actual", history_prefix_hash_}});
        return out;
    }

    // Clone the base state for lock-free execution on the kernel lane. A from-0 plan
    // (D5) starts from a GENUINELY EMPTY base — full replay + wholesale publish — so
    // no prior-head body survives into the scratch's starting state. An incremental
    // plan clones the live head (BodyStore + partition value-copied — TopoDS_Shape /
    // handle copies).
    out.status = FenceOutcome::Status::Ok;
    if (from_zero) {
        out.cloned_bodies = BodyStore{};                          // empty base (D5)
        out.cloned_partition = elementmap::ElementMapPartition{};  // empty base (D5)
        out.cloned_gear_bodies.clear();                            // empty base (D5)
    } else {
        out.cloned_bodies = bodies_;        // value copy of the live head
        out.cloned_partition = partition_;  // value copy of the live head
        out.cloned_gear_bodies = gear_bodies_;
    }
    out.prepared_snapshot_id = ++snapshot_counter_;
    return out;
}

void Session::store_prepared(ScratchJob job) {
    std::lock_guard<std::mutex> lk(mu_);
    scratch_ = std::move(job);
}

AcceptOutcome Session::accept_prepared(std::uint64_t job_id,
                                       std::uint64_t /*document_revision*/,  // D4: advisory
                                       std::uint64_t worker_epoch) {
    std::lock_guard<std::mutex> lk(mu_);
    AcceptOutcome out;

    if (!scratch_.has_value()) {
        out.error = protocol_error("AcceptPrepared: no prepared plan", nlohmann::json::object());
        return out;
    }
    if (scratch_->job_id != job_id) {
        out.error = protocol_error(
            "AcceptPrepared: jobId does not match the prepared plan",
            nlohmann::json{{"preparedJobId", scratch_->job_id}, {"requestedJobId", job_id}});
        return out;
    }
    // Re-fence at accept time on workerEpoch ONLY (D4): documentRevision is advisory
    // and never fences (a restart between prepare and accept bumps the epoch — that
    // Rust catches here; a Rust-owned revision bump does not invalidate the publish).
    if (worker_epoch != worker_epoch_) {
        out.error = protocol_error(
            "AcceptPrepared: stale workerEpoch",
            nlohmann::json{{"headEpoch", worker_epoch_}, {"acceptEpoch", worker_epoch}});
        return out;
    }

    // Atomic publish: REPLACE the head wholesale (D4/D5). Move-assigning the scratch
    // BodyStore + partition swaps the whole containers in, so NO stale body from the
    // previous head survives — for a from-0 plan (D5) the scratch was built from an
    // empty base, so the published set is exactly this plan's output; for an
    // incremental plan it is the cloned head mutated by the plan. Then adopt the
    // opaque head token + bump the snapshotId. (Sketches materialized by the plan are
    // intra-plan only — the solver lane owns sketch authoring; not republished here.)
    bodies_ = std::move(scratch_->bodies);
    partition_ = std::move(scratch_->partition);
    // SCHEMA §7.3 (WP-I): the gear-body map is PLAN-DERIVED, so it is rebuilt
    // here from the ACCEPTED plan against the freshly published bodies — no
    // per-body state to persist, and every replay reproduces it exactly.
    gear_bodies_ = ops::gear_body_infos(scratch_->plan, bodies_);
    history_prefix_hash_ = scratch_->history_prefix_hash;  // opaque; never recomputed
    snapshot_id_ = scratch_->prepared_snapshot_id;
    // D4: ADOPT the accepted plan's documentRevision as the head (Rust-owned edit
    // counter), instead of incrementing a worker-owned accept counter. Head stamps
    // thereafter echo this revision.
    document_revision_ = scratch_->plan_document_revision;

    out.ok = true;
    out.snapshot_id = snapshot_id_;
    out.document_revision = document_revision_;
    scratch_.reset();
    return out;
}

BodyStore Session::bodies_copy() const {
    std::lock_guard<std::mutex> lk(mu_);
    return bodies_;  // value copy (handle copies)
}

elementmap::ElementMapPartition Session::partition_copy() const {
    std::lock_guard<std::mutex> lk(mu_);
    return partition_;  // value copy
}

std::map<std::string, ops::GearBodyInfo> Session::gear_bodies_copy() const {
    std::lock_guard<std::mutex> lk(mu_);
    return gear_bodies_;  // value copy
}

std::uint64_t Session::current_snapshot_id() const {
    std::lock_guard<std::mutex> lk(mu_);
    return snapshot_id_;
}

std::optional<PublishedStateSnapshot> Session::published_state_at(
    std::optional<std::uint64_t> expected_snapshot_id, std::uint64_t* head_snapshot_id) const {
    std::lock_guard<std::mutex> lk(mu_);
    if (head_snapshot_id) *head_snapshot_id = snapshot_id_;
    if (expected_snapshot_id && snapshot_id_ != *expected_snapshot_id) return std::nullopt;
    return PublishedStateSnapshot{snapshot_id_, bodies_, partition_};
}

BindElementsOutcome Session::bind_element_ids(
    std::uint64_t expected_snapshot_id, const std::vector<ElementBindingInput>& bindings) {
    std::lock_guard<std::mutex> lk(mu_);
    BindElementsOutcome out;
    WLOG_DEBUG("ref_identity verb=BindElementIds requested=%llu head=%llu batch=%zu phase=start",
               static_cast<unsigned long long>(expected_snapshot_id),
               static_cast<unsigned long long>(snapshot_id_), bindings.size());
    if (snapshot_id_ != expected_snapshot_id) {
        out.error = ErrorInfo{
            "REF_UNRESOLVED", "BindElementIds: stale snapshot", false,
            nlohmann::json{{"requested", expected_snapshot_id}, {"head", snapshot_id_}}};
        WLOG_DEBUG(
            "ref_identity verb=BindElementIds requested=%llu head=%llu batch=%zu "
            "outcome=ref_unresolved reason=stale-snapshot",
            static_cast<unsigned long long>(expected_snapshot_id),
            static_cast<unsigned long long>(snapshot_id_), bindings.size());
        return out;
    }
    elementmap::ElementMapPartition staged = partition_;
    for (std::size_t i = 0; i < bindings.size(); ++i) {
        if (auto error = stage_binding(bodies_, staged, gear_bodies_, bindings[i], i)) {
            out.error = std::move(*error);
            WLOG_DEBUG(
                "ref_identity verb=BindElementIds requested=%llu head=%llu batch=%zu "
                "outcome=rejected bindingIndex=%zu code=%s",
                static_cast<unsigned long long>(expected_snapshot_id),
                static_cast<unsigned long long>(snapshot_id_), bindings.size(), i,
                out.error.code.c_str());
            return out;
        }
    }
    partition_ = std::move(staged);
    out.ok = true;
    for (const auto& binding : bindings) {
        out.bound.push_back(BoundElement{binding.body_id, binding.topo_key, binding.element_id,
                                         binding.kind});
    }
    WLOG_DEBUG("ref_identity verb=BindElementIds requested=%llu head=%llu batch=%zu outcome=bound",
               static_cast<unsigned long long>(expected_snapshot_id),
               static_cast<unsigned long long>(snapshot_id_), bindings.size());
    return out;
}

bool Session::discard_prepared(std::uint64_t /*job_id*/) {
    std::lock_guard<std::mutex> lk(mu_);
    // Best-effort: only one scratch exists; the jobId is advisory (Rust's discard
    // is best-effort and never changes the outcome).
    if (!scratch_.has_value()) return false;
    scratch_.reset();
    return true;
}

CheckpointState Session::save_checkpoint(std::uint64_t step) {
    std::lock_guard<std::mutex> lk(mu_);
    CheckpointState st{bodies_, partition_, history_prefix_hash_, gear_bodies_};
    checkpoints_[step] = st;  // supersede any earlier checkpoint at this step
    return st;
}

RestoreOutcome Session::restore_checkpoint(std::uint64_t step, const std::string& expected_hash) {
    std::lock_guard<std::mutex> lk(mu_);
    RestoreOutcome out;
    auto it = checkpoints_.find(step);
    if (it == checkpoints_.end()) {
        out.restored = false;  // absent (e.g. post-restart) ⇒ Rust replays from 0
        return out;
    }
    const CheckpointState& st = it->second;
    out.stored_hash = st.history_prefix_hash;
    // Staleness: the checkpoint's stored hash must match the base the plan expects.
    if (!expected_hash.empty() && expected_hash != st.history_prefix_hash) {
        out.restored = false;
        out.drift_detected = true;
        return out;
    }
    // Install the checkpoint state as the head (bump snapshotId, set the opaque hash).
    bodies_ = st.bodies;
    partition_ = st.partition;
    gear_bodies_ = st.gear_bodies;
    history_prefix_hash_ = st.history_prefix_hash;
    snapshot_id_ = ++snapshot_counter_;
    out.restored = true;
    out.snapshot_id = snapshot_id_;
    return out;
}

}  // namespace onecad::session
