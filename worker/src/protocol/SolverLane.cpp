// SolverLane.cpp — see SolverLane.h. SCHEMA §7.4 solver-lane verb handlers.
#include "protocol/SolverLane.h"

#include <algorithm>
#include <cctype>
#include <chrono>
#include <cmath>
#include <cstring>
#include <numbers>
#include <optional>
#include <unordered_set>
#include <utility>

#include "loop/LoopDetector.h"
#include "loop/PolygonFill.h"
#include "loop/RegionTable.h"
#include "loop/RegionUtils.h"
#include "sketch/SketchArc.h"
#include "sketch/SketchCircle.h"
#include "sketch/SketchPoint.h"

namespace onecad::protocol {

namespace sk = onecad::core::sketch;
namespace loop = onecad::core::loop;
using nlohmann::json;

namespace {

constexpr double kPosEpsilon = 1e-7;    // "changed point" threshold (mm)
constexpr double kAngleEpsilon = 1e-9;  // "changed angle" threshold (rad)

// SCHEMA §7.4 arc sweep floor. Lane-owned (like the MIN_GEOMETRY_SIZE radius
// floor): the solver hands a target to PlaneGCS verbatim, the DEGENERATE guards
// are the caller's, because only the caller knows the step is a user gesture it
// may refuse and re-offer.
constexpr double kMinArcSweep = 1e-3;  // rad

using PointPosMap = std::unordered_map<sk::EntityID, std::pair<double, double>>;

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

// SCHEMA §7.4 `drag.kind` tokens, matched EXACTLY: they are camelCase on the
// wire and the Rust side emits them verbatim, so case-folding here would only
// widen what counts as "known". An unknown token is OP_FAILED at the caller.
bool parse_drag_kind(const std::string& token, DragKind& out) {
    if (token == "point") {
        out = DragKind::Point;
    } else if (token == "arcEnd") {
        out = DragKind::ArcEnd;
    } else if (token == "radius") {
        out = DragKind::Radius;
    } else if (token == "entityBody") {
        out = DragKind::EntityBody;
    } else {
        return false;
    }
    return true;
}

// Role tokens ARE case-folded — `WireIndex::resolve_point` already lowercases
// them, so "Start" and "start" must not disagree between the two.
std::string lower(std::string s) {
    std::transform(s.begin(), s.end(), s.begin(),
                   [](unsigned char c) { return static_cast<char>(std::tolower(c)); });
    return s;
}

// `drag.grab` — the pointer-down position in sketch-plane coordinates.
bool read_grab(const json& drag, double& x, double& y) {
    if (!drag.contains("grab")) return false;
    const json& g = drag["grab"];
    if (!g.is_array() || g.size() < 2 || !g[0].is_number() || !g[1].is_number()) return false;
    x = g[0].get<double>();
    y = g[1].get<double>();
    return std::isfinite(x) && std::isfinite(y);
}

// Center point + radius of a Circle/Arc. False for anything else (a `radius`
// gesture on a non-curve is REF_UNRESOLVED, §7.4).
bool curve_center_and_radius(const sk::Sketch& sketch, const sk::EntityID& curve,
                             sk::EntityID& center, double& radius) {
    if (const auto* c = sketch.getEntityAs<sk::SketchCircle>(curve)) {
        center = c->centerPointId();
        radius = c->radius();
        return true;
    }
    if (const auto* a = sketch.getEntityAs<sk::SketchArc>(curve)) {
        center = a->centerPointId();
        radius = a->radius();
        return true;
    }
    return false;
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

// Every SOLVER-REGISTERED curve's parameters by internal id (the `curves`
// mirror of collect_positions). An Ellipse is deliberately absent: it is not
// registered with PlaneGCS, so no drag can move it and SCHEMA §7.4 forbids
// reporting one.
CurveMap collect_curves(const sk::Sketch& sketch) {
    CurveMap out;
    for (const auto& e : sketch.getAllEntities()) {
        if (!e) continue;
        if (e->type() == sk::EntityType::Circle) {
            const auto* c = dynamic_cast<const sk::SketchCircle*>(e.get());
            if (c) out[c->id()] = CurveParams{c->radius(), 0.0, 0.0, /*has_angles=*/false};
        } else if (e->type() == sk::EntityType::Arc) {
            const auto* a = dynamic_cast<const sk::SketchArc*>(e.get());
            if (a) {
                out[a->id()] = CurveParams{a->radius(), a->startAngle(), a->endAngle(),
                                           /*has_angles=*/true};
            }
        }
    }
    return out;
}

// {wireEntityId: {changed members}} — the SCHEMA §7.4 `curves` channel, under
// the same incremental discipline as changed_positions: CHANGED members only,
// and an entity with nothing changed is omitted entirely. Emitted for EVERY
// kind, not only `radius`: a Tangent propagates even a plain point drag into a
// neighbouring curve's radius, and before this channel existed that change
// reached the worker's store but never Rust (silent revert on the next upsert).
json changed_curves(const CurveMap& prev, const CurveMap& cur, const wire::WireIndex& index) {
    json out = json::object();
    for (const auto& [id, params] : cur) {
        const auto it = prev.find(id);
        const bool fresh = it == prev.end();
        json members = json::object();
        if (fresh || std::abs(params.radius - it->second.radius) > kPosEpsilon) {
            members["radius"] = params.radius;
        }
        if (params.has_angles) {
            if (fresh || std::abs(params.start_angle - it->second.start_angle) > kAngleEpsilon) {
                members["startAngle"] = params.start_angle;
            }
            if (fresh || std::abs(params.end_angle - it->second.end_angle) > kAngleEpsilon) {
                members["endAngle"] = params.end_angle;
            }
        }
        if (members.empty()) continue;
        const auto wit = index.internal_edge_to_wire.find(id);
        if (wit != index.internal_edge_to_wire.end()) out[wit->second] = std::move(members);
    }
    return out;
}

// Put the sketch back to a pose it previously REPORTED (positions + curves).
// PlaneGCS binds its parameters to the live entity fields (see
// ConstraintSolver::makePoint/makeArc), so writing them here is exactly what the
// solver's own drag rollback does — and by the time a step returns the tag(−1)
// drives are already cleared, so nothing stale is left pointing at them. Values
// are written through the MUTABLE references, not the setters, so an angle is
// restored verbatim instead of being re-normalized into a different double.
void restore_pose(sk::Sketch& sketch, const PointPosMap& positions, const CurveMap& curves) {
    for (const auto& [id, pos] : positions) {
        if (auto* p = sketch.getEntityAs<sk::SketchPoint>(id)) p->setPosition(pos.first, pos.second);
    }
    for (const auto& [id, params] : curves) {
        if (auto* c = sketch.getEntityAs<sk::SketchCircle>(id)) {
            c->radius() = params.radius;
            continue;
        }
        if (auto* a = sketch.getEntityAs<sk::SketchArc>(id)) {
            a->radius() = params.radius;
            a->startAngle() = params.start_angle;
            a->endAngle() = params.end_angle;
        }
    }
}

// SCHEMA §7.4 arc sweep floor: |endAngle − startAngle| below MIN_ARC_SWEEP.
// `sweepAngle()` is the CCW extent in [0, 2pi), and the angles are normalized to
// (−pi, pi], so "the two angles are numerically equal" surfaces at EITHER end of
// that range — a hair above 0 (collapsed to a point) or a hair below 2pi (the
// same collapse taken the other way round the circle). Testing the extent rather
// than the raw difference catches the wrap case too, which the raw difference
// misses whenever the pair straddles ±pi.
bool arc_sweep_collapsed(const sk::SketchArc& arc) {
    constexpr double kTwoPi = 2.0 * std::numbers::pi_v<double>;
    const double sweep = arc.sweepAngle();
    return sweep < kMinArcSweep || sweep > kTwoPi - kMinArcSweep;
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

    // --- SCHEMA §7.4 drag target ---------------------------------------------
    // The `drag` object declares WHAT the pointer grabbed:
    // { kind?, entity?, role?, grab?, pointId? }. An absent kind means `point`;
    // an unknown one is a hard failure (degrading to `point` would move a handle
    // the user never grabbed).
    const json drag = (args.contains("drag") && args["drag"].is_object())
                          ? args["drag"]
                          : json::object();
    DragKind kind = DragKind::Point;
    const std::string kind_token = drag.value("kind", std::string{});
    if (!kind_token.empty() && !parse_drag_kind(kind_token, kind)) {
        return err(req, "OP_FAILED", "BeginGesture: unknown drag kind '" + kind_token + "'");
    }
    const std::string entity_id = drag.value("entity", std::string{});
    const std::string role = lower(drag.value("role", std::string{}));

    sk::EntityID drag_internal;  // the point a Point/ArcEnd gesture moves
    sk::EntityID drag_entity;    // the entity an ArcEnd/Radius/EntityBody gesture owns
    std::vector<sk::EntityID> body_points;

    switch (kind) {
        case DragKind::Point: {
            // The pre-SP-2 forms resolve FIRST and outrank `entity`: a request
            // carrying both a legacy `pointId` and an `entity` means the point
            // (§7.4 "pointId wins"). Only for THIS kind — a `pointId` riding
            // along with kind=radius must never silently become a point drag.
            std::string point_id = drag.value("pointId", std::string{});
            if (point_id.empty()) point_id = args.value("pointId", std::string{});
            if (!point_id.empty()) {
                const auto it = tr.index.handle_to_point.find(point_id);
                if (it != tr.index.handle_to_point.end()) drag_internal = it->second;
            } else if (!entity_id.empty()) {
                drag_internal = tr.index.resolve_point(entity_id, role);
            }
            if (drag_internal.empty()) {
                const std::string named =
                    !point_id.empty() ? point_id
                                      : entity_id + (role.empty() ? "" : "." + role);
                return err(req, "REF_UNRESOLVED",
                           "BeginGesture: unknown drag point '" + named + "'");
            }
            break;
        }
        case DragKind::ArcEnd: {
            if (role != "start" && role != "end") {
                return err(req, "REF_UNRESOLVED",
                           "BeginGesture: arcEnd requires role start|end, got '" + role + "'");
            }
            const auto it = tr.index.wire_to_internal.find(entity_id);
            const sk::SketchArc* arc =
                it == tr.index.wire_to_internal.end()
                    ? nullptr
                    : tr.sketch->getEntityAs<sk::SketchArc>(it->second);
            // An arc with DERIVED endpoints (§7.3) has no point to drag: its
            // start/end exist only as computations, so there is nothing the
            // solver could move. That is unresolvable, not a silent no-op.
            if (!arc || !arc->hasEndpointPoints()) {
                return err(req, "REF_UNRESOLVED",
                           "BeginGesture: arcEnd needs an Arc with endpoint points, got '" +
                               entity_id + "'");
            }
            drag_entity = it->second;
            drag_internal = role == "start" ? arc->startPointId() : arc->endPointId();
            break;
        }
        case DragKind::Radius: {
            const auto it = tr.index.wire_to_internal.find(entity_id);
            sk::EntityID center;
            double radius = 0.0;
            if (it == tr.index.wire_to_internal.end() ||
                !curve_center_and_radius(*tr.sketch, it->second, center, radius)) {
                return err(req, "REF_UNRESOLVED",
                           "BeginGesture: radius needs a Circle or Arc, got '" + entity_id + "'");
            }
            drag_entity = it->second;
            break;
        }
        case DragKind::EntityBody: {
            const auto it = tr.index.wire_to_internal.find(entity_id);
            if (it != tr.index.wire_to_internal.end()) {
                body_points = tr.sketch->entityPointIds(it->second);
            }
            if (body_points.empty()) {
                return err(req, "REF_UNRESOLVED",
                           "BeginGesture: entityBody needs an entity owning at least one point, "
                           "got '" + entity_id + "'");
            }
            drag_entity = it->second;
            break;
        }
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

    // ONLY the Point kind opens a point drag: `beginPointDrag` pins every OTHER
    // point, which is right for "one handle to one position" and wrong for all
    // three new kinds (it over-determines an arc reshape and freezes the
    // geometry a body drag has to carry). They pin per-kind in `run_step`.
    if (kind == DragKind::Point) {
        tr.sketch->beginPointDrag(drag_internal);  // drag-fix strategy + rollback snapshot
    }

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
    g.kind = kind;
    g.drag_entity = drag_entity;
    g.body_points = std::move(body_points);

    // The grab-derived offsets are captured HERE, once, from the POST-diagnosis
    // pose (the same pose `baseline` records) — SCHEMA §7.4: re-deriving them per
    // step would let the gesture drift under the cursor.
    g.has_grab = read_grab(drag, g.grab.first, g.grab.second);
    if (kind == DragKind::EntityBody) {
        for (const sk::EntityID& pid : g.body_points) {
            const auto it = g.baseline.find(pid);
            if (it != g.baseline.end()) g.body_baseline[pid] = it->second;
        }
        // No grab ⇒ anchor on the FIRST owned point in handle order, which
        // teleports that point to the cursor (§7.4).
        if (!g.has_grab) {
            const auto it = g.body_baseline.find(g.body_points.front());
            if (it != g.body_baseline.end()) g.grab = it->second;
        }
    } else if (kind == DragKind::Radius && g.has_grab) {
        sk::EntityID center;
        double radius = 0.0;
        if (curve_center_and_radius(*tr.sketch, g.drag_entity, center, radius)) {
            const auto it = g.baseline.find(center);
            if (it != g.baseline.end()) {
                g.radius_offset = std::hypot(g.grab.first - it->second.first,
                                             g.grab.second - it->second.second) -
                                  radius;
            }
        }
    }

    g.baseline_curves = collect_curves(*tr.sketch);
    g.last_reported_curves = g.baseline_curves;
    g.sketch = std::move(tr.sketch);
    g.index = std::move(tr.index);
    gestures_[gesture_id] = std::move(g);

    json result = {{"gestureId", gesture_id}, {"ready", true}};
    return Envelope::ok_response(req.id, std::move(result));
}

// --- one drag step, per SCHEMA §7.4 target kind -----------------------------
//
// The pin set is the whole design. There is no single set that serves all four
// kinds, which is exactly why `Sketch::solveWithTargets` takes an EXPLICIT one:
//   * Point      — `beginPointDrag`'s pin-everything strategy (unchanged).
//   * ArcEnd     — everything except the arc's two endpoints.
//   * Radius     — the curve's center only.
//   * EntityBody — nothing at all.
// Each case documents why below. Targets always reach PlaneGCS as tag(−1)
// drives, so a committed constraint outranks them and an entity that lands away
// from the cursor is a SUCCESS reporting the solved pose (§7.4).
sk::SolveResult SolverLane::run_step(Gesture& g, double tx, double ty) {
    sk::Sketch& sketch = *g.sketch;

    switch (g.kind) {
        case DragKind::Point:
            // Byte-identical to the pre-SP-2 lane: one point, one position,
            // `beginPointDrag`'s pins, `solveWithDrag`.
            return sketch.solveWithDrag(g.drag_point, sk::Vec2d{tx, ty});

        case DragKind::ArcEnd: {
            // Pin every point EXCEPT the arc's own two endpoints. The CENTER is
            // in that set on purpose — a reshape must not translate the arc —
            // while the SIBLING endpoint is deliberately left free: the four
            // internal arc rules already tie both endpoints to
            // center+radius+angles, so pinning the sibling over-determines the
            // system and the solve could only refuse. Letting it float is what
            // turns the drag into a reshape (radius + both angles follow).
            const auto* arc = sketch.getEntityAs<sk::SketchArc>(g.drag_entity);
            std::unordered_set<sk::EntityID> pins;
            for (const auto& e : sketch.getAllEntities()) {
                if (!e || e->type() != sk::EntityType::Point) continue;
                if (arc && (e->id() == arc->startPointId() || e->id() == arc->endPointId())) {
                    continue;
                }
                pins.insert(e->id());
            }
            sk::SolveResult r = sketch.solveWithTargets({{g.drag_point, sk::Vec2d{tx, ty}}}, {},
                                                        pins);
            // §7.4 arc sweep floor: a collapsed arc cannot be recovered by
            // dragging further, so the pose is never entered. Restore what was
            // last REPORTED (positions AND curves, so the incremental deltas
            // stay honest) and answer partial with the gesture still OPEN.
            // (`arc` survives the solve — solving never reallocates entities.)
            if (r.success && arc && arc_sweep_collapsed(*arc)) {
                restore_pose(sketch, g.last_reported, g.last_reported_curves);
                r.success = false;
                r.errorMessage = "Arc sweep below MIN_ARC_SWEEP";
            }
            return r;
        }

        case DragKind::Radius: {
            // Pin the center ONLY: the cursor sets the radius and the curve must
            // not slide sideways, but everything else stays free so a Tangent (or
            // any other coupling) can propagate the new radius outward.
            sk::EntityID center;
            double radius = 0.0;
            if (!curve_center_and_radius(sketch, g.drag_entity, center, radius)) {
                sk::SolveResult r;
                r.success = false;
                r.errorMessage = "Drag target curve not found";
                return r;
            }
            const auto* center_point = sketch.getEntityAs<sk::SketchPoint>(center);
            const double cx = center_point ? center_point->position().X() : 0.0;
            const double cy = center_point ? center_point->position().Y() : 0.0;
            // CURRENT center, not the baseline one: a constrained center that
            // moved during the gesture would otherwise shear the radius by
            // exactly how far it moved.
            const double target = std::max(
                std::hypot(tx - cx, ty - cy) - g.radius_offset,
                sk::constants::MIN_GEOMETRY_SIZE);  // §7.4 floor: never zero/negative
            std::unordered_set<sk::EntityID> pins;
            if (!center.empty()) pins.insert(center);
            return sketch.solveWithTargets({}, {{g.drag_entity, target}}, pins);
        }

        case DragKind::EntityBody: {
            // Pin NOTHING. Whatever is welded or constrained to this entity has
            // to be free to follow, and pinning "the rest of the sketch" would
            // refuse the drag outright. Targets come from the BeginGesture
            // baseline plus the cursor delta — always from the baseline, never
            // accumulated, so a dropped frame costs nothing (latest-wins).
            const double dx = tx - g.grab.first;
            const double dy = ty - g.grab.second;
            std::unordered_map<sk::EntityID, sk::Vec2d> targets;
            targets.reserve(g.body_points.size());
            for (const sk::EntityID& pid : g.body_points) {
                const auto it = g.body_baseline.find(pid);
                if (it == g.body_baseline.end()) continue;
                targets[pid] = sk::Vec2d{it->second.first + dx, it->second.second + dy};
            }
            return sketch.solveWithTargets(targets, {}, {});
        }
    }

    sk::SolveResult unreachable;
    unreachable.success = false;
    unreachable.errorMessage = "Unknown drag kind";
    return unreachable;
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
    const auto before_curves = g.last_reported_curves;
    const auto t0 = std::chrono::steady_clock::now();
    sk::SolveResult r = run_step(g, tx, ty);
    const auto t1 = std::chrono::steady_clock::now();
    const auto solve_micros =
        std::chrono::duration_cast<std::chrono::microseconds>(t1 - t0).count();

    const auto cur = collect_positions(*g.sketch);
    const auto cur_curves = collect_curves(*g.sketch);

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
    json curves = changed_curves(before_curves, cur_curves, g.index);
    g.last_reported = cur;
    g.last_reported_curves = cur_curves;
    g.last_success = r.success;

    json result = {
        {"gestureId", gesture_id},
        {"seq", seq},
        {"status", status},
        {"dof", g.dof},
        {"conflicting", conflicting},
        {"positions", std::move(positions)},
        // Additive §7.4 channel: the curve members `positions` structurally
        // cannot carry. Emitted for every kind (an absent one parses as {}).
        {"curves", std::move(curves)},
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
            // Same per-kind dispatch the drag steps used — the committed pose
            // must be produced by the same pin sets that produced the preview.
            r = run_step(g, ft[0].get<double>(), ft[1].get<double>());
            did_final_drag = true;
        }
    }
    // Only the Point kind ever opened a point drag (see on_begin), and
    // endPointDrag() also owns the failed-gesture rollback — calling it for a
    // kind that never began one would be a rollback of nothing at best.
    if (g.kind == DragKind::Point) g.sketch->endPointDrag();
    if (!did_final_drag) r = g.sketch->solve();

    const int dof = g.dof;
    const auto cur = collect_positions(*g.sketch);
    const auto cur_curves = collect_curves(*g.sketch);

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
        // Same additive §7.4 channel as SolveDrag, baselined at BeginGesture.
        {"curves", changed_curves(g.baseline_curves, cur_curves, g.index)},
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
        {"regionIdentityVersion", 2},
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
