// SolverLane.cpp — see SolverLane.h. SCHEMA §7.4 solver-lane verb handlers.
#include "protocol/SolverLane.h"

#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstring>
#include <optional>
#include <unordered_set>
#include <utility>

#include "loop/LoopDetector.h"
#include "loop/PolygonFill.h"
#include "loop/RegionTable.h"
#include "loop/RegionUtils.h"
#include "sketch/SketchPoint.h"

namespace onecad::protocol {

namespace sk = onecad::core::sketch;
namespace loop = onecad::core::loop;
using nlohmann::json;

namespace {

constexpr double kPosEpsilon = 1e-7;  // "changed point" threshold (mm)

Envelope err(const Envelope& req, const char* code, const std::string& msg) {
    // §8: OP_FAILED / REF_UNRESOLVED are recoverable (session intact); retriable
    // is false — these are input/state errors, not transient failures.
    return Envelope::error_response(req.id,
                                    ErrorInfo{code, msg, /*retriable=*/false});
}

bool read_target(const json& p, double& x, double& y) {
    if (!p.contains("target")) return false;
    const json& t = p["target"];
    if (!t.is_array() || t.size() < 2 || !t[0].is_number() || !t[1].is_number()) return false;
    x = t[0].get<double>();
    y = t[1].get<double>();
    return std::isfinite(x) && std::isfinite(y);
}

std::uint64_t u64(const json& p, const char* key) {
    if (p.is_object() && p.contains(key) && p[key].is_number()) return p[key].get<std::uint64_t>();
    return 0;
}

// Snapshot every Point entity's position by internal id.
std::unordered_map<sk::EntityID, std::pair<double, double>> collect_positions(
    const sk::Sketch& sketch) {
    std::unordered_map<sk::EntityID, std::pair<double, double>> out;
    for (const auto& e : sketch.getAllEntities()) {
        if (e && e->type() == sk::EntityType::Point) {
            const auto* p = dynamic_cast<const sk::SketchPoint*>(e.get());
            if (p) out[p->id()] = {p->position().X(), p->position().Y()};
        }
    }
    return out;
}

// {handle: [x,y]} for points whose position moved beyond eps vs `prev`.
json changed_positions(const std::unordered_map<sk::EntityID, std::pair<double, double>>& prev,
                       const std::unordered_map<sk::EntityID, std::pair<double, double>>& cur,
                       const wire::WireIndex& index) {
    json out = json::object();
    for (const auto& [id, pos] : cur) {
        auto it = prev.find(id);
        const bool moved =
            it == prev.end() ||
            std::abs(pos.first - it->second.first) > kPosEpsilon ||
            std::abs(pos.second - it->second.second) > kPosEpsilon;
        if (!moved) continue;
        const std::string handle = index.handle_for(id);
        if (!handle.empty()) out[handle] = json::array({pos.first, pos.second});
    }
    return out;
}

std::vector<std::string> map_conflicting(const wire::WireIndex& index,
                                         const std::vector<sk::ConstraintID>& internal) {
    std::vector<std::string> out;
    out.reserve(internal.size());
    for (const auto& cid : internal) {
        auto it = index.internal_constraint_to_wire.find(cid);
        out.push_back(it != index.internal_constraint_to_wire.end() ? it->second : cid);
    }
    return out;
}

// SketchUpsert / gesture state ∈ UnderConstrained|FullyConstrained|OverConstrained|Conflicting
// (SCHEMA §7.4). PlaneGCS distinguishes genuine conflicts (no solution) from
// benign redundancy — extra constraints that remove no DOF (corpus g), e.g. a
// duplicate dimension. Priority, highest first:
//   Conflicting                    — genuine conflict (no solution exists)
//   OverConstrained  (redundant && !conflicting) — solvable but carries a benign
//                                     redundant constraint; a warning, not an error
//   FullyConstrained (dof == 0)    — exactly determined, no redundancy
//   UnderConstrained               — remaining DOF
// OverConstrained deliberately outranks FullyConstrained: a solvable sketch with
// one redundant dimension is DOF-preserving (its dof may still be 0), and the
// warning must surface (Rust/dto + the dimension tool treat OverConstrained like
// Conflicting for auto-reject). HISTORY: earlier revisions collapsed redundancy
// into FullyConstrained and cited corpus g as a reason to hide OverConstrained;
// that "documented deviation" is retired — the redundant system IS what §7.4
// means by OverConstrained, so it is now surfaced.
std::string upsert_state(int dof, bool conflicting, bool redundant) {
    if (conflicting) return "Conflicting";
    if (redundant) return "OverConstrained";
    if (dof == 0) return "FullyConstrained";
    return "UnderConstrained";
}

// Preview triangulation lives in loop/PolygonFill.{h,cpp} — holes ARE subtracted
// so the fill matches the solid the kernel builds from the same region. See that
// header for the bridge/shared-index invariant the frontend ring derivation
// depends on.

void append_f32(std::vector<std::uint8_t>& buf, float f) {
    std::uint8_t tmp[4];
    std::memcpy(tmp, &f, 4);  // host is little-endian (asserted at startup)
    buf.insert(buf.end(), tmp, tmp + 4);
}

void append_u32(std::vector<std::uint8_t>& buf, std::uint32_t u) {
    std::uint8_t tmp[4];
    std::memcpy(tmp, &u, 4);
    buf.insert(buf.end(), tmp, tmp + 4);
}

}  // namespace

// --- verb registration ------------------------------------------------------

void SolverLane::register_verbs(Dispatcher& dispatcher) {
    dispatcher.register_solver_verb(
        "SketchUpsert",
        [this](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext&) {
            return on_upsert(r);
        });
    dispatcher.register_solver_verb(
        "BeginGesture",
        [this](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext&) {
            return on_begin(r);
        });
    dispatcher.register_solver_verb(
        "SolveDrag",
        [this](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext&) {
            return on_drag(r);
        });
    dispatcher.register_solver_verb(
        "EndGesture",
        [this](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext&) {
            return on_end(r);
        });
    dispatcher.register_solver_verb(
        "SketchRegions",
        [this](const Envelope& r, const std::vector<std::uint8_t>&, HandlerContext&) {
            return on_regions(r);
        });
}

// --- SketchUpsert -----------------------------------------------------------

Envelope SolverLane::on_upsert(const Envelope& req) {
    const json& args = req.args;
    const std::string sketch_id = args.value("sketchId", std::string{});
    if (sketch_id.empty()) return err(req, "OP_FAILED", "SketchUpsert: missing sketchId");

    wire::TranslateResult tr = wire::translate(args);
    if (!tr.ok) return err(req, "OP_FAILED", "SketchUpsert: " + tr.error);

    const sk::SolveResult solve = tr.sketch->solve();
    const int dof = tr.sketch->getDegreesOfFreedom();
    const auto conflicting = tr.sketch->getConflictingConstraints();
    const bool redundant = tr.sketch->hasRedundantConstraints();
    const std::string state = upsert_state(dof, !conflicting.empty(), redundant);

    json stored_args = args;
    if (solve.success) {
        wire::apply_solved_positions(stored_args, *tr.sketch, tr.index);
    }
    const std::uint64_t revision = store_.upsert(sketch_id, std::move(stored_args));

    json result = {
        {"upserted", true},
        {"sketchId", sketch_id},
        {"sketchRevision", revision},
        {"dof", dof},
        {"state", state},
        // Per-constraint conflict ids (SCHEMA §7.4): the constraints PlaneGCS reports
        // as mutually unsatisfiable, wire-mapped (empty when the sketch is solvable).
        {"conflicting", map_conflicting(tr.index, conflicting)},
    };
    return Envelope::ok_response(req.id, std::move(result));
}

// --- BeginGesture -----------------------------------------------------------

Envelope SolverLane::on_begin(const Envelope& req) {
    const json& args = req.args;
    const std::string sketch_id = args.value("sketchId", std::string{});
    const std::uint64_t gesture_id = u64(args, "gestureId");

    std::optional<session::StoredSketch> stored = store_.snapshot(sketch_id);
    if (!stored) return err(req, "REF_UNRESOLVED", "BeginGesture: unknown sketch " + sketch_id);
    if (args.contains("sketchRevision") &&
        u64(args, "sketchRevision") != stored->revision) {
        return err(req, "REF_UNRESOLVED", "BeginGesture: stale sketchRevision");
    }

    wire::TranslateResult tr = wire::translate(stored->wire_args);
    if (!tr.ok) return err(req, "OP_FAILED", "BeginGesture: " + tr.error);

    // Resolve the drag point handle ("drag":{"pointId"} or "pointId").
    std::string point_id;
    if (args.contains("drag") && args["drag"].is_object()) {
        point_id = args["drag"].value("pointId", std::string{});
    }
    if (point_id.empty()) point_id = args.value("pointId", std::string{});
    sk::EntityID drag_internal;
    {
        auto it = tr.index.handle_to_point.find(point_id);
        if (it != tr.index.handle_to_point.end()) drag_internal = it->second;
    }
    if (drag_internal.empty()) {
        return err(req, "REF_UNRESOLVED", "BeginGesture: unknown drag point '" + point_id + "'");
    }

    // Build + diagnose the GCS system ONCE. Capture the sketch's inherent
    // redundancy HERE, before beginPointDrag() adds drag-fix constraints: a drag
    // pins every non-dragged point, which makes the committed constraints look
    // redundant against those pins, so the per-drag solve cannot tell inherent
    // redundancy apart from drag-induced redundancy. The committed sketch's
    // redundancy is fixed for the whole gesture (same as g.conflicting).
    tr.sketch->solve();
    const int dof = tr.sketch->getDegreesOfFreedom();
    const auto conflicting_internal = tr.sketch->getConflictingConstraints();
    const bool redundant = tr.sketch->hasRedundantConstraints();

    tr.sketch->beginPointDrag(drag_internal);  // drag-fix strategy + rollback snapshot

    Gesture g;
    g.id = gesture_id;
    g.sketch_id = sketch_id;
    g.sketch_revision = stored->revision;
    g.drag_point = drag_internal;
    g.dof = dof;
    g.redundant = redundant;
    g.conflicting = map_conflicting(tr.index, conflicting_internal);
    g.baseline = collect_positions(*tr.sketch);
    g.last_reported = g.baseline;
    g.sketch = std::move(tr.sketch);
    g.index = std::move(tr.index);
    gestures_[gesture_id] = std::move(g);

    json result = {{"gestureId", gesture_id}, {"ready", true}};
    return Envelope::ok_response(req.id, std::move(result));
}

// --- SolveDrag --------------------------------------------------------------

Envelope SolverLane::on_drag(const Envelope& req) {
    const json& args = req.args;
    const std::uint64_t gesture_id = u64(args, "gestureId");
    const std::uint64_t seq = u64(args, "seq");

    auto git = gestures_.find(gesture_id);
    if (git == gestures_.end()) {
        return err(req, "REF_UNRESOLVED", "SolveDrag: unknown or ended gesture");
    }
    Gesture& g = git->second;

    double tx, ty;
    if (!read_target(args, tx, ty)) return err(req, "OP_FAILED", "SolveDrag: invalid target");

    const auto before = g.last_reported;  // deltas are reported incrementally
    const auto t0 = std::chrono::steady_clock::now();
    sk::SolveResult r = g.sketch->solveWithDrag(g.drag_point, sk::Vec2d{tx, ty});
    const auto t1 = std::chrono::steady_clock::now();
    const auto solve_micros =
        std::chrono::duration_cast<std::chrono::microseconds>(t1 - t0).count();

    const auto cur = collect_positions(*g.sketch);

    // Status precedence (SCHEMA §7.4: success | partial | conflicting | redundant):
    // conflicting > redundant > success, with partial for a non-converged solve.
    // Redundancy is the gesture-fixed g.redundant diagnosed at BeginGesture, NOT
    // r.redundantConstraints (drag pins make the committed constraints look
    // redundant — see on_begin).
    std::string status;
    std::vector<std::string> conflicting;
    if (!r.conflictingConstraints.empty()) {
        status = "conflicting";
        conflicting = map_conflicting(g.index, r.conflictingConstraints);
    } else if (!g.conflicting.empty()) {
        status = "conflicting";
        conflicting = g.conflicting;
    } else if (r.success) {
        status = g.redundant ? "redundant" : "success";
    } else {
        status = "partial";
    }

    json positions = changed_positions(before, cur, g.index);
    g.last_reported = cur;
    g.last_success = r.success;

    json result = {
        {"gestureId", gesture_id},
        {"seq", seq},
        {"status", status},
        {"dof", g.dof},
        {"conflicting", conflicting},
        {"positions", std::move(positions)},
        {"solveMicros", solve_micros},
    };
    return Envelope::ok_response(req.id, std::move(result));
}

// --- EndGesture -------------------------------------------------------------

Envelope SolverLane::on_end(const Envelope& req) {
    const json& args = req.args;
    const std::uint64_t gesture_id = u64(args, "gestureId");

    auto git = gestures_.find(gesture_id);
    if (git == gestures_.end()) {
        return err(req, "REF_UNRESOLVED", "EndGesture: unknown or ended gesture");
    }
    Gesture g = std::move(git->second);
    gestures_.erase(git);

    // Final EXACT solve: to the committed pointer-up target if provided, else a
    // plain full solve. endPointDrag() rolls back to the drag-start pose on a
    // failed gesture (rollback determinism, corpus g).
    sk::SolveResult r;
    bool did_final_drag = false;
    if (args.contains("commit") && args["commit"].is_object() &&
        args["commit"].contains("finalTarget")) {
        const json ft = args["commit"]["finalTarget"];
        if (ft.is_array() && ft.size() >= 2 && ft[0].is_number() && ft[1].is_number()) {
            r = g.sketch->solveWithDrag(g.drag_point, sk::Vec2d{ft[0].get<double>(),
                                                                ft[1].get<double>()});
            did_final_drag = true;
        }
    }
    g.sketch->endPointDrag();
    if (!did_final_drag) r = g.sketch->solve();

    const int dof = g.dof;
    const auto cur = collect_positions(*g.sketch);

    // Status precedence matches on_drag (SCHEMA §7.4): conflicting > redundant >
    // success; partial otherwise. Redundancy is the gesture-fixed g.redundant.
    std::string status;
    if (!r.conflictingConstraints.empty() || !g.conflicting.empty()) status = "conflicting";
    else if (r.success) status = g.redundant ? "redundant" : "success";
    else status = "partial";

    // Per-constraint conflict ids (SCHEMA §7.4), same precedence on_drag uses: the
    // final drag solve's conflicts, else the gesture-fixed set diagnosed at
    // BeginGesture (g.conflicting is already wire-mapped, empty when none).
    std::vector<std::string> conflicting = !r.conflictingConstraints.empty()
                                               ? map_conflicting(g.index, r.conflictingConstraints)
                                               : g.conflicting;

    // Commit into the session store: bump revision + write back solved positions.
    const std::uint64_t new_rev = g.sketch_revision + 1;
    if (std::optional<session::StoredSketch> s = store_.snapshot(g.sketch_id)) {
        wire::apply_solved_positions(s->wire_args, *g.sketch, g.index);
        store_.put(g.sketch_id, std::move(s->wire_args), new_rev);
    }

    json result = {
        {"gestureId", gesture_id},
        {"status", status},
        {"dof", dof},
        {"conflicting", conflicting},
        {"positions", changed_positions(g.baseline, cur, g.index)},
        {"sketchRevision", new_rev},
    };
    return Envelope::ok_response(req.id, std::move(result));
}

// --- SketchRegions ----------------------------------------------------------

Envelope SolverLane::on_regions(const Envelope& req) {
    const json& args = req.args;
    const std::string sketch_id = args.value("sketchId", std::string{});
    std::optional<session::StoredSketch> stored = store_.snapshot(sketch_id);
    if (!stored) return err(req, "REF_UNRESOLVED", "SketchRegions: unknown sketch " + sketch_id);

    wire::TranslateResult tr = wire::translate(stored->wire_args);
    if (!tr.ok) return err(req, "OP_FAILED", "SketchRegions: " + tr.error);
    const sk::SolveResult solve = tr.sketch->solve();
    if (!solve.success) {
        const std::string detail =
            solve.errorMessage.empty() ? "constraint solve did not converge" : solve.errorMessage;
        return err(req, "OP_FAILED", "SketchRegions: solve failed: " + detail);
    }

    loop::LoopDetector detector;
    detector.setConfig(loop::makeRegionDetectionConfig());
    const loop::LoopDetectionResult det = detector.detect(*tr.sketch);
    const auto map_edge = [&](const sk::EntityID& internalId) {
        const auto it = tr.index.internal_edge_to_wire.find(internalId);
        return it != tr.index.internal_edge_to_wire.end() ? it->second : internalId;
    };
    const loop::RegionTable table = loop::buildRegionTable(
        det, map_edge, sk::constants::COINCIDENCE_TOLERANCE);
    if (!table.success) {
        return err(req, "OP_FAILED", "SketchRegions: " + table.errorMessage);
    }

    json regions = json::array();
    std::vector<std::uint8_t> tail;
    json bin_sections = json::array();
    std::unordered_set<std::string> section_names;

    for (const loop::RegionDefinition& region_def : table.regions) {
        json holes = json::array();
        for (const std::vector<std::string>& hole : region_def.holeWireEdges) {
            holes.push_back(hole);
        }

        std::vector<std::vector<sk::Vec2d>> hole_polys;
        hole_polys.reserve(region_def.holes.size());
        for (const loop::Loop& hole : region_def.holes) {
            hole_polys.push_back(hole.polygon);
        }

        const loop::RegionFill fill =
            loop::fill_region(region_def.outerLoop.polygon, hole_polys);
        if (fill.holes_subtracted != region_def.holes.size()) {
            return err(req, "OP_FAILED",
                       "SketchRegions: failed to triangulate every hole for region " +
                           region_def.id);
        }
        // Fail closed on a stalled ear clip: a partial triangle list reads as a
        // wrong boundary downstream (the frontend recovers extrusion rings from
        // single-use triangulation edges), never publish it.
        if (!fill.complete) {
            return err(req, "OP_FAILED",
                       "SketchRegions: incomplete triangulation for region " + region_def.id);
        }
        const std::vector<std::uint32_t>& indices = fill.indices;
        const std::size_t vertex_count = fill.verts.size();

        const std::uint64_t off = tail.size();
        for (std::size_t i = 0; i < vertex_count; ++i) {
            append_f32(tail, static_cast<float>(fill.verts[i].x));
            append_f32(tail, static_cast<float>(fill.verts[i].y));
            append_f32(tail, 0.0f);
        }
        for (std::uint32_t idx : indices) append_u32(tail, idx);
        const std::uint64_t len = tail.size() - off;

        const std::string section = "region:" + region_def.id;
        if (!section_names.insert(section).second) {
            return err(req, "OP_FAILED", "SketchRegions: duplicate binary section " + section);
        }
        bin_sections.push_back({{"name", section}, {"off", off}, {"len", len}});

        json region = {
            {"regionId", region_def.id},
            {"outerLoop", region_def.outerWireEdges},
            {"holes", holes},
            {"previewTriangles",
             {{"format", "f32xyz+u32idx"},
              {"vertexCount", vertex_count},
              {"triangleCount", indices.size() / 3},
              {"holesSubtracted", fill.holes_subtracted},
              {"bin", section}}},
        };
        regions.push_back(std::move(region));
    }

    json result = {
        {"sketchId", sketch_id},
        {"sketchRevision", stored->revision},
        {"regions", std::move(regions)},
    };
    Envelope resp = Envelope::ok_response(req.id, std::move(result));
    // Attach the binary tail + section table.
    for (const auto& s : bin_sections) {
        resp.bin.push_back(BinSection{s["name"].get<std::string>(),
                                      s["off"].get<std::uint64_t>(),
                                      s["len"].get<std::uint64_t>()});
    }
    resp.out_bin = std::move(tail);
    return resp;
}

}  // namespace onecad::protocol
