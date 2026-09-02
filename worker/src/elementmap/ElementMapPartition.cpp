// ElementMapPartition.cpp — see ElementMapPartition.h.
#include "elementmap/ElementMapPartition.h"

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <vector>

#include <BRepAdaptor_Curve.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRepBndLib.hxx>
#include <BRepBuilderAPI_MakeVertex.hxx>
#include <BRepExtrema_DistShapeShape.hxx>
#include <BRepTools.hxx>
#include <BRep_Tool.hxx>
#include <Bnd_Box.hxx>
#include <GeomAPI_ProjectPointOnSurf.hxx>
#include <Geom_Surface.hxx>
#include <Standard_Failure.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <gp_Dir.hxx>
#include <gp_Vec.hxx>
#include <gp_XYZ.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ListOfShape.hxx>

#include "elementmap/Scoring.h"

namespace onecad::elementmap {

namespace {

TopAbs_ShapeEnum topabs_of(km::ElementKind kind) {
    switch (kind) {
        case km::ElementKind::Face: return TopAbs_FACE;
        case km::ElementKind::Edge: return TopAbs_EDGE;
        case km::ElementKind::Vertex: return TopAbs_VERTEX;
        default: return TopAbs_SHAPE;
    }
}

char topokey_prefix(km::ElementKind kind) {
    switch (kind) {
        case km::ElementKind::Face: return 'f';
        case km::ElementKind::Edge: return 'e';
        case km::ElementKind::Vertex: return 'v';
        case km::ElementKind::Body: return 'b';
        default: return '?';
    }
}

// Parse "<prefix>:<index>" → {prefix, 1-based index}. Returns false on garbage.
bool parse_topokey(const std::string& tk, char& prefix, int& index) {
    const std::size_t colon = tk.find(':');
    if (colon == std::string::npos || colon == 0) return false;
    prefix = tk[0];
    try {
        index = std::stoi(tk.substr(colon + 1));
    } catch (...) {
        return false;
    }
    return index >= 1;
}

km::ElementKind kind_of_prefix(char p) {
    switch (p) {
        case 'f': return km::ElementKind::Face;
        case 'e': return km::ElementKind::Edge;
        case 'v': return km::ElementKind::Vertex;
        case 'b': return km::ElementKind::Body;
        default: return km::ElementKind::Unknown;
    }
}

}  // namespace

// --- statics ---------------------------------------------------------------

std::string ElementMapPartition::kind_name(km::ElementKind kind) {
    switch (kind) {
        case km::ElementKind::Face: return "face";
        case km::ElementKind::Edge: return "edge";
        case km::ElementKind::Vertex: return "vertex";
        case km::ElementKind::Body: return "body";
        default: return "unknown";
    }
}

km::ElementKind ElementMapPartition::kind_from_name(const std::string& s) {
    if (s == "face") return km::ElementKind::Face;
    if (s == "edge") return km::ElementKind::Edge;
    if (s == "vertex") return km::ElementKind::Vertex;
    if (s == "body") return km::ElementKind::Body;
    return km::ElementKind::Unknown;
}

km::ElementDescriptor ElementMapPartition::describe(const TopoDS_Shape& shape) {
    // REUSE the kernel descriptor verbatim: register into a throwaway ElementMap
    // (which runs the exact private computeDescriptor + quantization constants),
    // then read the stored descriptor back. This forks NO constant and never
    // touches the header the parity gate pins.
    km::ElementMap tmp;
    const km::ElementId id = km::ElementId::From("__describe__");
    tmp.registerElement(id, km::ElementKind::Unknown, shape);
    if (const km::Entry* e = tmp.find(id)) return e->descriptor;
    return km::ElementDescriptor{};
}

namespace {

// Outward unit normal of `face` (as oriented in its body) at the surface point
// nearest `near` — the surface normal flipped for a REVERSED face.
bool face_outward_at(const TopoDS_Face& face, const gp_Pnt& near, gp_Dir& out) {
    try {
        const Handle(Geom_Surface) surface = BRep_Tool::Surface(face);
        if (surface.IsNull()) return false;
        double u = 0.0, v = 0.0;
        GeomAPI_ProjectPointOnSurf projector(near, surface);
        if (projector.IsDone() && projector.NbPoints() > 0) {
            projector.LowerDistanceParameters(u, v);
        } else {
            double umin, umax, vmin, vmax;
            BRepTools::UVBounds(face, umin, umax, vmin, vmax);
            u = 0.5 * (umin + umax);
            v = 0.5 * (vmin + vmax);
        }
        gp_Pnt p;
        gp_Vec du, dv;
        surface->D1(u, v, p, du, dv);
        gp_Vec n = du.Crossed(dv);
        if (n.Magnitude() <= 1e-12) return false;
        if (face.Orientation() == TopAbs_REVERSED) n.Reverse();
        out = gp_Dir(n);
        return true;
    } catch (const Standard_Failure&) {
        return false;
    }
}

// The v4 SIDEDNESS evidence: a face's outward normal at its UV centre; an edge's
// normalized sum of the outward normals of its adjacent faces, each evaluated at
// the point nearest the edge midpoint. Deterministic (pure geometry, indexed maps).
bool outward_of(const TopoDS_Shape& shape_in, const TopoDS_Shape& body_shape, gp_Dir& out) {
    if (shape_in.IsNull()) return false;
    try {
        // Orientation is a property of the shape's OCCURRENCE in the body: history
        // lists (Modified/Generated) hand back neutral instances, so the sign is
        // taken from the body's own instance of the same TShape. `FindIndex` matches
        // by IsSame (orientation-blind) and returns the body-oriented shape.
        TopoDS_Shape shape = shape_in;
        if (!body_shape.IsNull()) {
            TopTools_IndexedMapOfShape located;
            TopExp::MapShapes(body_shape, shape_in.ShapeType(), located);
            const int index = located.FindIndex(shape_in);
            if (index > 0) shape = located(index);
        }
        if (shape.ShapeType() == TopAbs_FACE) {
            const TopoDS_Face face = TopoDS::Face(shape);
            double umin, umax, vmin, vmax;
            BRepTools::UVBounds(face, umin, umax, vmin, vmax);
            const Handle(Geom_Surface) surface = BRep_Tool::Surface(face);
            if (surface.IsNull()) return false;
            gp_Pnt mid;
            gp_Vec du, dv;
            surface->D1(0.5 * (umin + umax), 0.5 * (vmin + vmax), mid, du, dv);
            return face_outward_at(face, mid, out);
        }
        if (shape.ShapeType() == TopAbs_EDGE && !body_shape.IsNull()) {
            const TopoDS_Edge edge = TopoDS::Edge(shape);
            BRepAdaptor_Curve curve(edge);
            const gp_Pnt mid = curve.Value(0.5 * (curve.FirstParameter() + curve.LastParameter()));
            TopTools_IndexedDataMapOfShapeListOfShape edge_faces;
            TopExp::MapShapesAndAncestors(body_shape, TopAbs_EDGE, TopAbs_FACE, edge_faces);
            const int index = edge_faces.FindIndex(edge);
            if (index == 0) return false;
            gp_Vec sum(0.0, 0.0, 0.0);
            std::vector<TopoDS_Shape> seen;
            for (TopTools_ListIteratorOfListOfShape it(edge_faces(index)); it.More(); it.Next()) {
                const TopoDS_Shape& f = it.Value();
                bool duplicate = false;
                for (const TopoDS_Shape& s : seen) duplicate = duplicate || s.IsSame(f);
                if (duplicate) continue;
                seen.push_back(f);
                gp_Dir n;
                if (face_outward_at(TopoDS::Face(f), mid, n)) sum += gp_Vec(n);
            }
            if (sum.Magnitude() <= 1e-9) return false;
            out = gp_Dir(sum);
            return true;
        }
    } catch (const Standard_Failure&) {
        return false;
    }
    return false;
}

}  // namespace

km::ElementDescriptor ElementMapPartition::describe(const TopoDS_Shape& shape,
                                                    const TopoDS_Shape& body_shape) {
    km::ElementDescriptor d = describe(shape);
    gp_Dir outward;
    if (outward_of(shape, body_shape, outward)) {
        d.outward = outward;
        d.hasOutward = true;
    }
    return d;
}

std::string ElementMapPartition::topokey_for_shape(const TopoDS_Shape& body_shape,
                                                   const TopoDS_Shape& sub_shape,
                                                   km::ElementKind kind) {
    if (body_shape.IsNull() || sub_shape.IsNull()) return "";
    const TopAbs_ShapeEnum type = topabs_of(kind);
    if (type == TopAbs_SHAPE) return "";
    TopTools_IndexedMapOfShape map;
    TopExp::MapShapes(body_shape, type, map);
    const int idx = map.FindIndex(sub_shape);  // 1-based; 0 if absent (IsSame match)
    if (idx <= 0) return "";
    return std::string(1, topokey_prefix(kind)) + ":" + std::to_string(idx);
}

std::string ElementMapPartition::topokey_for_element_in_body(const ElementMapPartition& partition,
                                                             const std::string& element_id,
                                                             const std::string& body_id) {
    if (element_id.empty() || body_id.empty()) return "";
    const PartitionEntry* entry = partition.find(element_id);
    // A foreign entry is NOT an error here — the caller falls through to the
    // descriptor ladder, which owns the bind-or-NeedsRepair decision.
    if (entry == nullptr || entry->body_id != body_id) return "";
    return entry->topo_key;
}

TopoDS_Shape ElementMapPartition::shape_for_topokey(const TopoDS_Shape& body_shape,
                                                    const std::string& topo_key) {
    char prefix = 0;
    int index = 0;
    if (body_shape.IsNull() || !parse_topokey(topo_key, prefix, index)) return TopoDS_Shape();
    const km::ElementKind kind = kind_of_prefix(prefix);
    const TopAbs_ShapeEnum type = topabs_of(kind);
    if (type == TopAbs_SHAPE) return TopoDS_Shape();
    TopTools_IndexedMapOfShape map;
    TopExp::MapShapes(body_shape, type, map);
    if (index < 1 || index > map.Extent()) return TopoDS_Shape();
    return map(index);
}

TopoDS_Shape ElementMapPartition::nearest_subshape(const TopoDS_Shape& body_shape,
                                                   km::ElementKind kind, double wx, double wy,
                                                   double wz) {
    if (body_shape.IsNull()) return TopoDS_Shape();
    const TopAbs_ShapeEnum type = topabs_of(kind);
    if (type == TopAbs_SHAPE) return TopoDS_Shape();
    TopTools_IndexedMapOfShape map;
    TopExp::MapShapes(body_shape, type, map);
    TopoDS_Shape best;
    double best_d2 = -1.0;
    for (int i = 1; i <= map.Extent(); ++i) {
        const km::ElementDescriptor d = describe(map(i));
        const double dx = d.center.X() - wx, dy = d.center.Y() - wy, dz = d.center.Z() - wz;
        const double d2 = dx * dx + dy * dy + dz * dz;
        if (best_d2 < 0.0 || d2 < best_d2) {
            best_d2 = d2;
            best = map(i);
        }
    }
    return best;
}

nlohmann::json ElementMapPartition::descriptor_to_json(const km::ElementDescriptor& d) {
    nlohmann::json out = nlohmann::json{
        {"shapeType", static_cast<int>(d.shapeType)},
        {"center", {d.center.X(), d.center.Y(), d.center.Z()}},
        {"size", d.size},
        {"magnitude", d.magnitude},
        {"surfaceType", static_cast<int>(d.surfaceType)},
        {"curveType", static_cast<int>(d.curveType)},
        {"normal", {d.normal.X(), d.normal.Y(), d.normal.Z()}},
        {"tangent", {d.tangent.X(), d.tangent.Y(), d.tangent.Z()}},
        {"hasNormal", d.hasNormal},
        {"hasTangent", d.hasTangent},
        // adjacencyHash is a 64-bit value → hex string (SCHEMA §2 hash wire form).
        {"adjacencyHash", [&] {
             char buf[17];
             std::snprintf(buf, sizeof(buf), "%016llx",
                           static_cast<unsigned long long>(d.adjacencyHash));
             return std::string(buf);
         }()},
    };
    // v4 sidedness rides ADDITIVELY: absent on a descriptor computed without a body
    // (and on every pre-v4 record), so old readers and old fixtures are untouched.
    if (d.hasOutward) out["outward"] = {d.outward.X(), d.outward.Y(), d.outward.Z()};
    return out;
}

km::ElementDescriptor ElementMapPartition::descriptor_from_json(const nlohmann::json& j) {
    km::ElementDescriptor d;
    if (!j.is_object()) return d;
    auto num = [](const nlohmann::json& v, double dflt) {
        return v.is_number() ? v.get<double>() : dflt;
    };
    auto vec3 = [&](const char* key, double dx, double dy, double dz) -> gp_XYZ {
        if (j.contains(key) && j[key].is_array() && j[key].size() >= 3) {
            const nlohmann::json& a = j[key];
            return gp_XYZ(num(a[0], dx), num(a[1], dy), num(a[2], dz));
        }
        return gp_XYZ(dx, dy, dz);
    };
    if (j.contains("shapeType") && j["shapeType"].is_number())
        d.shapeType = static_cast<TopAbs_ShapeEnum>(j["shapeType"].get<int>());
    if (j.contains("surfaceType") && j["surfaceType"].is_number())
        d.surfaceType = static_cast<GeomAbs_SurfaceType>(j["surfaceType"].get<int>());
    if (j.contains("curveType") && j["curveType"].is_number())
        d.curveType = static_cast<GeomAbs_CurveType>(j["curveType"].get<int>());
    const gp_XYZ c = vec3("center", 0, 0, 0);
    d.center = gp_Pnt(c.X(), c.Y(), c.Z());
    if (j.contains("size")) d.size = num(j["size"], 0.0);
    if (j.contains("magnitude")) d.magnitude = num(j["magnitude"], 0.0);
    d.hasNormal = j.value("hasNormal", false);
    d.hasTangent = j.value("hasTangent", false);
    if (d.hasNormal) {
        const gp_XYZ n = vec3("normal", 0, 0, 1);
        if (n.Modulus() > 1e-12) d.normal = gp_Dir(n);
    }
    if (d.hasTangent) {
        const gp_XYZ t = vec3("tangent", 1, 0, 0);
        if (t.Modulus() > 1e-12) d.tangent = gp_Dir(t);
    }
    if (j.contains("adjacencyHash") && j["adjacencyHash"].is_string()) {
        d.adjacencyHash =
            std::strtoull(j["adjacencyHash"].get<std::string>().c_str(), nullptr, 16);
    }
    if (j.contains("outward") && j["outward"].is_array() && j["outward"].size() >= 3 &&
        j["outward"][0].is_number() && j["outward"][1].is_number() && j["outward"][2].is_number()) {
        const gp_XYZ o(j["outward"][0].get<double>(), j["outward"][1].get<double>(),
                       j["outward"][2].get<double>());
        if (o.Modulus() > 1e-12) {
            d.outward = gp_Dir(o);
            d.hasOutward = true;
        }
    }
    return d;
}

// --- queries ---------------------------------------------------------------

const PartitionEntry* ElementMapPartition::find(const std::string& element_id) const {
    auto it = entries_.find(element_id);
    return it != entries_.end() ? &it->second : nullptr;
}

bool ElementMapPartition::contains(const std::string& element_id) const {
    return entries_.count(element_id) != 0;
}

std::vector<const PartitionEntry*> ElementMapPartition::entries_for_body(
    const std::string& body_id) const {
    std::vector<const PartitionEntry*> out;
    for (const auto& [id, e] : entries_) {
        if (e.body_id == body_id) out.push_back(&e);
    }
    return out;
}

// --- minting ---------------------------------------------------------------

DeltaEntry ElementMapPartition::mint(const std::string& body_id, const std::string& element_id,
                                     km::ElementKind kind, const TopoDS_Shape& sub_shape,
                                     const TopoDS_Shape& body_shape, nlohmann::json anchor) {
    PartitionEntry& e = entries_[element_id];
    e.element_id = element_id;
    e.body_id = body_id;
    e.kind = kind;
    e.shape = sub_shape;
    e.topo_key = topokey_for_shape(body_shape, sub_shape, kind);
    e.descriptor = describe(sub_shape, body_shape);
    if (!anchor.is_null()) e.anchor = std::move(anchor);
    return DeltaEntry{element_id, e.topo_key, kind_name(kind), body_id};
}

// --- history application ---------------------------------------------------

namespace {

double body_diag_of(const TopoDS_Shape& shape) {
    if (shape.IsNull()) return 1.0;
    Bnd_Box box;
    BRepBndLib::Add(shape, box);
    if (box.IsVoid()) return 1.0;
    Standard_Real xmin, ymin, zmin, xmax, ymax, zmax;
    box.Get(xmin, ymin, zmin, xmax, ymax, zmax);
    const double dx = xmax - xmin, dy = ymax - ymin, dz = zmax - zmin;
    const double diag = std::sqrt(dx * dx + dy * dy + dz * dz);
    return diag > 1e-9 ? diag : 1.0;
}

// A world-point AnchorEvidence parsed from an entry's stored anchor echo (if any).
AnchorEvidence anchor_of(const nlohmann::json& anchor) {
    AnchorEvidence a;
    if (anchor.is_object() && anchor.contains("worldPoint") && anchor["worldPoint"].is_array() &&
        anchor["worldPoint"].size() >= 3) {
        const nlohmann::json& wp = anchor["worldPoint"];
        if (wp[0].is_number() && wp[1].is_number() && wp[2].is_number()) {
            a.has_world_point = true;
            a.world_point = gp_Pnt(wp[0].get<double>(), wp[1].get<double>(), wp[2].get<double>());
        }
    }
    return a;
}

struct HistorySuccessor {
    TopoDS_Shape shape;
    bool modified = false;
};

// Modified is direct successor evidence; Generated may also be the only lineage
// available. Keep both and deduplicate by TopoDS identity.
std::vector<HistorySuccessor> history_successors(BRepBuilderAPI_MakeShape& hist,
                                                 const TopoDS_Shape& old) {
    std::vector<HistorySuccessor> successors;
    const auto append_unique = [&](const TopTools_ListOfShape& images, bool modified) {
        for (TopTools_ListIteratorOfListOfShape it(images); it.More(); it.Next()) {
            auto duplicate = std::find_if(successors.begin(), successors.end(), [&](auto& seen) {
                return seen.shape.IsSame(it.Value());
            });
            if (duplicate != successors.end()) {
                duplicate->modified = duplicate->modified || modified;
            } else {
                successors.push_back(HistorySuccessor{it.Value(), modified});
            }
        }
    };
    append_unique(hist.Modified(old), true);
    append_unique(hist.Generated(old), false);
    return successors;
}

// Direct Modified lineage supplies part of the locked 0.10 margin. The value is
// deliberately below the margin by itself: an otherwise tied Modified/Generated
// pair still refuses, while descriptor separation must supply the remainder.
constexpr double kModifiedHistoryConfidence = 0.09;

}  // namespace

void ElementMapPartition::apply_history(const std::string& body_id,
                                        const TopoDS_Shape& new_body_shape,
                                        BRepBuilderAPI_MakeShape& hist, ElementMapDelta& delta,
                                        std::vector<nlohmann::json>* needs_repair_out) {
    const double body_diag = body_diag_of(new_body_shape);

    // Collect the entries of this body up front (we mutate the map below).
    std::vector<std::string> ids;
    for (const auto& [id, e] : entries_) {
        if (e.body_id == body_id) ids.push_back(id);
    }

    auto emit_no_candidates = [&](const std::string& id) {
        if (needs_repair_out) {
            needs_repair_out->push_back(nlohmann::json{
                {"refId", id},
                {"elementId", id},
                {"ladderFailed", "history"},
                {"reason", "no-candidates"},
                {"scoringVersion", kResolverVersion},
                {"candidates", nlohmann::json::array()},
                {"anchor", nlohmann::json::object()},
                {"uiLabel", "unresolved element on " + body_id}});
        }
    };

    for (const std::string& id : ids) {
        PartitionEntry& e = entries_[id];
        const TopoDS_Shape old = e.shape;

        // Deleted by the operation → the element no longer exists (definitive).
        if (!old.IsNull() && hist.IsDeleted(old)) {
            delta.removed.push_back(id);
            entries_.erase(id);
            continue;
        }

        // Ladder level 1 (SCHEMA §10): consult the union of both OCCT history
        // channels. Some builders expose same-kind successors only as Generated().
        // Cross-kind images (a face's section EDGES, an edge's split VERTICES) can
        // never be bound to this entry — `topokey_for_shape(kind)` would find no
        // ordinal — so they are dropped BEFORE scoring rather than allowed to turn
        // a unique same-kind image into an "ambiguous" split (v4).
        std::vector<HistorySuccessor> successors;
        if (!old.IsNull()) {
            const TopAbs_ShapeEnum want = old.ShapeType();
            for (HistorySuccessor& s : history_successors(hist, old)) {
                if (!s.shape.IsNull() && s.shape.ShapeType() == want) successors.push_back(s);
            }
        }

        TopoDS_Shape image;
        if (successors.empty()) {
            // Not deleted, not modified: does the old shape survive verbatim?
            image = old;
        } else if (successors.size() == 1) {
            // UNIQUE image → auto-bind (the fillet-survives-edit path).
            image = successors.front().shape;
        } else {
            // SPLIT: >1 images. EXPLICIT LINEAGE — score every image candidate
            // against the entry's frozen descriptor + anchor and gate on confidence
            // (W-WP6, closes review finding 2 — no forced Modified().First()).
            // The anchor is measured to the image CENTROID here (v3 form), not to
            // the sub-shape: the images are successors of ONE element, and a pick
            // point the op itself carved away (a hole drilled AT the pick) would be
            // equidistant from the modified face and the wall generated inside it.
            const AnchorEvidence anchor = anchor_of(e.anchor);
            std::vector<TopoDS_Shape> imgs;
            std::vector<double> scores;
            std::vector<km::ElementDescriptor> descs;
            for (const HistorySuccessor& successor : successors) {
                imgs.push_back(successor.shape);
                descs.push_back(describe(successor.shape, new_body_shape));
            }
            for (std::size_t k = 0; k < imgs.size(); ++k) {
                const ScoreResult sr = score_candidate(e.descriptor, /*has_intent_descriptor=*/true,
                                                       anchor, descs[k], body_diag);
                double score = sr.score;
                if (successors[k].modified) score += kModifiedHistoryConfidence;
                if (std::getenv("ONECAD_LADDER_DEBUG")) {
                    std::string contrib;
                    for (const auto& [n, v] : sr.contributions) contrib += n + "=" + std::to_string(v) + " ";
                    std::fprintf(stderr, "[ladder] %s image %zu kind=%d modified=%d score=%.4f {%s}\n",
                                 id.c_str(), k, static_cast<int>(imgs[k].ShapeType()),
                                 successors[k].modified ? 1 : 0, score, contrib.c_str());
                }
                scores.push_back(std::min(score, 1.0));
            }
            // best / runner-up (deterministic tie-break by list order).
            int best = 0;
            for (int i = 1; i < static_cast<int>(scores.size()); ++i)
                if (scores[i] > scores[best]) best = i;
            double runner_up = 0.0;
            for (int i = 0; i < static_cast<int>(scores.size()); ++i)
                if (i != best) runner_up = std::max(runner_up, scores[i]);
            const double margin = scores[best] - runner_up;

            if (scores[best] >= kAutoBindMinScore && margin >= kAutoBindMinMargin) {
                image = imgs[best];  // confident unique successor
            } else {
                // Ambiguous / symmetric split ⇒ NeedsRepair (never a guess).
                if (needs_repair_out) {
                    nlohmann::json cands = nlohmann::json::array();
                    // Rank images desc by score for the evidence payload.
                    std::vector<int> order(imgs.size());
                    for (int i = 0; i < static_cast<int>(order.size()); ++i) order[i] = i;
                    std::sort(order.begin(), order.end(), [&](int a, int b) {
                        if (scores[a] != scores[b]) return scores[a] > scores[b];
                        return a < b;
                    });
                    for (int k = 0; k < static_cast<int>(order.size()); ++k) {
                        const int i = order[k];
                        const std::string tk = topokey_for_shape(new_body_shape, imgs[i], e.kind);
                        const double next = (k + 1 < static_cast<int>(order.size()))
                                                ? scores[order[k + 1]]
                                                : scores[i];
                        cands.push_back(nlohmann::json{
                            {"topoKey", tk},
                            {"score", scores[i]},
                            {"margin", scores[i] - next},
                            {"worldPos", {descs[i].center.X(), descs[i].center.Y(), descs[i].center.Z()}},
                            {"summary", "split image of " + id}});
                    }
                    needs_repair_out->push_back(nlohmann::json{
                        {"refId", id},
                        {"elementId", id},
                        {"ladderFailed", "history"},
                        {"reason", "ambiguous"},
                        {"scoringVersion", kResolverVersion},
                        {"candidates", std::move(cands)},
                        {"anchor", e.anchor.is_null() ? nlohmann::json::object() : e.anchor},
                        {"uiLabel", "ambiguous split of element on " + body_id}});
                }
                entries_.erase(id);  // cannot confidently rebind
                continue;
            }
        }

        const std::string new_key = topokey_for_shape(new_body_shape, image, e.kind);
        if (new_key.empty()) {
            // No identifiable successor in the new body → NeedsRepair "no-candidates".
            emit_no_candidates(id);
            entries_.erase(id);
            continue;
        }

        const bool changed = (new_key != e.topo_key) || !image.IsSame(e.shape);
        e.shape = image;
        e.topo_key = new_key;
        e.descriptor = describe(image, new_body_shape);
        if (changed) {
            delta.relabeled.push_back(DeltaEntry{id, new_key, kind_name(e.kind), body_id});
        }
    }
}

// --- rigid placement --------------------------------------------------------

namespace {

// True iff `j[key]` is a 3-number array (a world point / direction triple).
bool is_vec3(const nlohmann::json& j, const char* key) {
    return j.is_object() && j.contains(key) && j[key].is_array() && j[key].size() >= 3 &&
           j[key][0].is_number() && j[key][1].is_number() && j[key][2].is_number();
}

// Move a stored POINT triple by the FULL transformation (rotation + translation).
void move_point(nlohmann::json& holder, const char* key, const gp_Trsf& trsf) {
    if (!is_vec3(holder, key)) return;
    const nlohmann::json& a = holder[key];
    gp_Pnt p(a[0].get<double>(), a[1].get<double>(), a[2].get<double>());
    p.Transform(trsf);
    holder[key] = {p.X(), p.Y(), p.Z()};
}

// Move a stored DIRECTION triple by the ROTATION only. `gp_Vec::Transform` applies
// the linear part of the trsf and ignores its translation by construction — a free
// vector has no position — so this needs no separate rotation-only gp_Trsf.
void move_direction(nlohmann::json& holder, const char* key, const gp_Trsf& trsf) {
    if (!is_vec3(holder, key)) return;
    const nlohmann::json& a = holder[key];
    gp_Vec v(a[0].get<double>(), a[1].get<double>(), a[2].get<double>());
    v.Transform(trsf);
    holder[key] = {v.X(), v.Y(), v.Z()};
}

}  // namespace

void ElementMapPartition::apply_placement(const std::string& body_id, const gp_Trsf& trsf) {
    if (trsf.Form() == gp_Identity) return;  // a no-op placement moves no evidence
    for (auto& [id, e] : entries_) {
        if (e.body_id != body_id || !e.anchor.is_object()) continue;
        move_point(e.anchor, "worldPoint", trsf);
        if (e.anchor.contains("localFrame") && e.anchor["localFrame"].is_object()) {
            nlohmann::json& lf = e.anchor["localFrame"];
            move_point(lf, "origin", trsf);
            move_direction(lf, "x", trsf);
            move_direction(lf, "y", trsf);
            move_direction(lf, "z", trsf);
        }
        // `surfaceUv` (parametric) and `adjacencyHint` (topology hash) are
        // placement-invariant — deliberately NOT rewritten.
    }
}

void ElementMapPartition::remove_body(const std::string& body_id, ElementMapDelta& delta) {
    for (auto it = entries_.begin(); it != entries_.end();) {
        if (it->second.body_id == body_id) {
            delta.removed.push_back(it->first);
            it = entries_.erase(it);
        } else {
            ++it;
        }
    }
}

}  // namespace onecad::elementmap
