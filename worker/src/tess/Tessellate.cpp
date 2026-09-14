// Tessellate.cpp — see Tessellate.h.
#include "tess/Tessellate.h"

#include <algorithm>
#include <cmath>
#include <cstddef>
#include <cstdio>
#include <exception>
#include <functional>
#include <limits>
#include <map>

#include <BRepAdaptor_Curve.hxx>
#include <BRepBndLib.hxx>
#include <BRepMesh_IncrementalMesh.hxx>
#include <BRep_Tool.hxx>
#include <Bnd_Box.hxx>
#include <Poly_Triangle.hxx>
#include <Poly_Triangulation.hxx>
#include <Standard_Failure.hxx>
#include <TopAbs_Orientation.hxx>
#include <TopExp.hxx>
#include <TopLoc_Location.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <gp_Pnt.hxx>
#include <gp_Vec.hxx>

#include "tess/CurveSampler.h"
#include "tess/Mesh1.h"
#include "tess/SurfaceNormals.h"
#include "util/Log.h"

namespace onecad::tess {

namespace km = onecad::kernel::elementmap;

namespace {

std::uint16_t lod_code(const std::string& lod) {
    if (lod == "fine") return 2;
    if (lod == "medium") return 1;
    return 0;  // coarse
}

// Display tessellation policy, in model units (mm for native documents).
//
// A relative-only deflection makes a small fillet on a large body visibly
// faceted. Keep the tier relationship, but clamp each tier to an absolute
// window. Fine is the committed display/export quality: its 5 degree angular
// cap controls round silhouettes even when the linear cap is reached.
void deflections(const std::string& lod, double diag, double& lin, double& ang) {
    double rel = 0.01;
    double min_lin = 0.01;
    double max_lin = 0.5;
    double a = 0.5;  // coarse: transient interaction quality
    if (lod == "medium") {
        rel = 0.0025;
        min_lin = 0.002;
        max_lin = 0.15;
        a = 0.21;
    } else if (lod == "fine") {
        rel = 0.0005;
        min_lin = 0.0001;
        max_lin = 0.05;
        a = 0.08726646259971647;  // 5 degrees
    }
    lin = std::clamp(diag * rel, min_lin, max_lin);
    ang = a;
}

// NUM §2.5 initial per-body edge budget, spent as a RUNNING total.
constexpr std::size_t kBodyEdgeSegmentBudget = 2000000;

// The live budget. Equal to kBodyEdgeSegmentBudget in every production build and
// every production call; `detail::set_body_edge_segment_budget_for_test` is the
// only writer and no shipped code path reaches it.
std::size_t& body_edge_segment_budget_slot() {
    static std::size_t budget = kBodyEdgeSegmentBudget;
    return budget;
}

std::string num_text(double v) {
    char buf[32];
    std::snprintf(buf, sizeof(buf), "%.6g", v);
    return std::string(buf);
}

bool all_points_finite(const std::vector<gp_Pnt>& points) {
    for (const gp_Pnt& p : points) {
        if (!std::isfinite(p.X()) || !std::isfinite(p.Y()) || !std::isfinite(p.Z())) return false;
    }
    return true;
}

// Last rung of the edge policy, and the answer for an edge that arrives after
// the body's segment budget is gone: the two analytic endpoints, always logged.
// Never a silent straight chord.
CurveSampleResult endpoints_only(const BRepAdaptor_Curve& curve, const std::string& label,
                                 double tolerance, const std::string& reason,
                                 const char* headline) {
    CurveSampleResult out;
    try {
        const gp_Pnt first = curve.Value(curve.FirstParameter());
        const gp_Pnt last = curve.Value(curve.LastParameter());
        if (!std::isfinite(first.X()) || !std::isfinite(first.Y()) || !std::isfinite(first.Z()) ||
            !std::isfinite(last.X()) || !std::isfinite(last.Y()) || !std::isfinite(last.Z())) {
            out.certification = CurveCertification::Failed;
            out.diagnostic = "the edge's analytic endpoints are not finite";
            return out;
        }
        out.points.push_back(first);
        out.points.push_back(last);
        out.certification = CurveCertification::QualityLimited;
        out.qualityLimited = true;
        out.diagnostic = reason;
        WLOG_WARN("tessellate: edge %s display quality limited at %.6g mm, %s (%s); emitting "
                  "endpoints only",
                  label.c_str(), tolerance, headline, reason.c_str());
    } catch (const Standard_Failure& failure) {
        out.points.clear();
        out.certification = CurveCertification::Failed;
        out.diagnostic = std::string("endpoint evaluation failed: ") + failure.GetMessageString();
    }
    return out;
}

// WP08 edge policy (NUM §2.5 / VP11). A WORK cap (depth, per-edge segments,
// per-body segments) is not an acceptance condition, so a work-capped edge is
// re-sampled against a deliberately relaxed budget and the achieved tolerance is
// logged; a still-capped edge falls back to its two analytic endpoints rather
// than to a chord that pretends to be in budget. A REPRESENTATION-limited result
// (QualityLimited WITH points) is NOT re-sampled: the polyline is already
// certified in double and is the best float32 world storage can carry, so
// relaxing the tolerance would change nothing but the label.
//
// Cancellation: the sampler polls a callback once per span. No display job is
// cancellable yet, so `{}` is passed here; WP12 (worker display jobs) wires the
// real token through without touching the sampler.
// TopoKey → minted ElementId lookup for one body (empty map when no partition).
std::map<std::string, std::string> minted_ids(const elementmap::ElementMapPartition* partition,
                                              const std::string& body_id) {
    std::map<std::string, std::string> out;
    if (!partition) return out;
    for (const elementmap::PartitionEntry* e : partition->entries_for_body(body_id)) {
        if (!e->topo_key.empty()) out[e->topo_key] = e->element_id;
    }
    return out;
}

}  // namespace

namespace detail {

void set_body_edge_segment_budget_for_test(std::size_t budget) {
    body_edge_segment_budget_slot() = budget;
}

void clear_body_edge_segment_budget_for_test() {
    body_edge_segment_budget_slot() = kBodyEdgeSegmentBudget;
}

std::size_t body_edge_segment_budget() { return body_edge_segment_budget_slot(); }

CurveSampleResult sample_edge_polyline(const BRepAdaptor_Curve& curve, const std::string& label,
                                       const CurveSampleRequest& request,
                                       const CurveSampleLimits& limits) {
    CurveSampleResult result = sample_edge_curve(curve, request, limits, {});
    if (result.certification != CurveCertification::QualityLimited || !result.points.empty()) {
        return result;
    }

    constexpr int kMaxDoublings = 4;
    constexpr double kMaxAngularRad = 1.5707963267948966;  // 90 degrees
    for (int doubling = 1; doubling <= kMaxDoublings; ++doubling) {
        const double factor = static_cast<double>(1U << doubling);
        CurveSampleRequest relaxed = request;
        relaxed.chordToleranceMm = request.chordToleranceMm * factor;
        relaxed.angularToleranceRad = std::min(request.angularToleranceRad * factor, kMaxAngularRad);
        CurveSampleResult retry = sample_edge_curve(curve, relaxed, limits, {});
        if (!retry.points.empty() && retry.certification != CurveCertification::Failed) {
            // PR-07 / D9, sharpened by R1(c) MINOR 7. The polyline is real, but
            // what it ACHIEVED is whatever the retry itself certified — not the
            // budget the retry was handed. A retry that is only QualityLimited
            // did not meet the relaxed tolerance either, so it reports no
            // achieved tolerance at all; a retry that came through the
            // approximate fallback keeps its PROVENANCE and carries the quality
            // limitation in `qualityLimited` instead of having `KernelEstimated`
            // overwritten. What must never happen either way is the retry's own
            // `Certified` reaching a consumer as if the request had been met.
            const bool certified_at_relaxed =
                retry.certification == CurveCertification::Certified;
            retry.qualityLimited = true;
            retry.requestedChordToleranceMm = request.chordToleranceMm;
            retry.requestedAngularToleranceRad = request.angularToleranceRad;
            std::string achieved_text;
            if (certified_at_relaxed) {
                retry.certification = CurveCertification::QualityLimited;
                retry.certifiedChordBoundMm = -1;
                retry.achievedChordToleranceMm = relaxed.chordToleranceMm;
                retry.achievedAngularToleranceRad = relaxed.angularToleranceRad;
                achieved_text = "achieved " + num_text(relaxed.chordToleranceMm) + " mm";
            } else {
                // Provenance (KernelEstimated) or a further shortfall
                // (QualityLimited) is preserved exactly as the retry reported it.
                retry.achievedChordToleranceMm = -1;
                retry.achievedAngularToleranceRad = -1;
                achieved_text = "achieved nothing certified even at " +
                                num_text(relaxed.chordToleranceMm) + " mm";
            }
            retry.diagnostic = "display quality limited: requested " +
                               num_text(request.chordToleranceMm) + " mm, " + achieved_text +
                               " after " + std::to_string(doubling) + " tolerance doubling" +
                               (doubling == 1 ? "" : "s") + " (" + result.diagnostic + ")" +
                               (retry.diagnostic.empty() ? std::string()
                                                         : "; retry reported: " + retry.diagnostic);
            WLOG_WARN(
                "tessellate: edge %s display quality limited at %.6g mm (%s); emitting the "
                "%.6g mm polyline instead after %d doubling(s)",
                label.c_str(), request.chordToleranceMm, result.diagnostic.c_str(),
                relaxed.chordToleranceMm, doubling);
            return retry;
        }
    }

    return endpoints_only(curve, label, request.chordToleranceMm, result.diagnostic,
                          "still limited after 4 tolerance doublings");
}

}  // namespace detail

BodyMesh tessellate_body(const TopoDS_Shape& shape, const std::string& body_id,
                         const std::string& lod, bool include_edges,
                         const elementmap::ElementMapPartition* partition,
                         const std::vector<std::uint32_t>* face_colors) {
    BodyMesh out;
    out.body_id = body_id;
    if (shape.IsNull()) return out;

    Bnd_Box box;
    BRepBndLib::Add(shape, box);
    double diag = 1.0;
    if (!box.IsVoid()) {
        Standard_Real xmin, ymin, zmin, xmax, ymax, zmax;
        box.Get(xmin, ymin, zmin, xmax, ymax, zmax);
        diag = gp_Pnt(xmin, ymin, zmin).Distance(gp_Pnt(xmax, ymax, zmax));
        out.body_id = body_id;
    }
    double lin = 0.1, ang = 0.5;
    deflections(lod, diag, lin, ang);

    // Mesh (single-threaded for determinism; the ids/ordinal are threading-
    // independent regardless — Invariant 5).
    //
    // No `mesher.Perform()` follows: the four-argument constructor "Automatically
    // calls method Perform" (BRepMesh_IncrementalMesh.hxx) and the second call was
    // a no-op. Measured on the F08 gallery (box, cylinder, sphere, torus, cone,
    // all-edge-filleted box) at coarse/medium/fine: every MESH1 blob is
    // byte-identical with and without it (WP09 step 6). Do not re-add it.
    //
    // HAZARD, recorded for WP12: OCCT caches the triangulation ON the shape, and
    // callers hand us a `BodyStore` VALUE copy whose `TopoDS_Shape`s are handle
    // copies of the session's own TShapes (`Session::bodies_copy`,
    // `PlanExecutor::attach_tessellate`, `PreviewOp`). A display job therefore
    // REPLACES the triangulation the document and the STL/OBJ exporters see. That
    // is the existing behaviour. Nothing races TODAY only because every geometry
    // verb (ExecutePlan / Tessellate / ExportStl / ExportObj) runs on the one
    // kernel lane thread — the solver lane never touches body geometry — so the
    // mutation is merely serialised, not isolated. Isolating display meshing onto
    // scratch geometry is WP12's job, not a change to make here.
    BRepMesh_IncrementalMesh mesher(shape, lin, Standard_False, ang, Standard_False);

    const std::map<std::string, std::string> ids = minted_ids(partition, body_id);
    auto label = [&](char prefix, int index) {
        const std::string topo = std::string(1, prefix) + ":" + std::to_string(index);
        auto it = ids.find(topo);
        if (it != ids.end()) return std::make_pair(it->second, true);
        return std::make_pair(topo, false);
    };

    Mesh1Input mi;
    mi.lod = lod_code(lod);
    mi.has_normals = true;

    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(shape, TopAbs_FACE, faces);

    std::uint32_t tri_cursor = 0;
    bool any_elementid = false;

    for (int fi = 1; fi <= faces.Extent(); ++fi) {
        const TopoDS_Face face = TopoDS::Face(faces(fi));
        const std::string face_key = "f:" + std::to_string(fi);
        TopLoc_Location loc;
        Handle(Poly_Triangulation) tri = BRep_Tool::Triangulation(face, loc);
        const std::uint32_t first_tri = tri_cursor;
        const auto lbl = label('f', fi);
        any_elementid = any_elementid || lbl.second;

        // WP09 (VP10, NUM §7): the shading normal of every node comes from the
        // SUPPORTING SURFACE at that node's UV, with the face location applied
        // exactly once and the reflection handled on the winding, not twice on the
        // normal. Singular nodes follow NUM §7.3's analytic / fallback / split
        // ladder; none of them may answer world +Z.
        FaceNormalResult fn =
            compute_face_normals(face, tri, loc, face.Orientation() == TopAbs_REVERSED);
        out.completeness.normalFallbackNodes += fn.fallbackCount;
        out.completeness.singularSplitNodes += fn.splitCount;
        out.completeness.normalUnresolvedNodes += fn.unresolvedCount;
        out.completeness.normalMissingNodes += fn.missingCount;
        out.completeness.float32CollapsedTriangles += fn.float32CollapsedTriangles;
        if (fn.float32CollapsedTriangles > 0) {
            // Astra §4: a REPRESENTATION failure, never a degeneracy exemption.
            // `ulp32(8192 mm) = 2^-10 mm`, so a 0.0001 x 0.01 mm rectangle at
            // X = 8192 mm collapses in the emitted float32 while its BRep is
            // perfectly valid.
            WLOG_WARN("tessellate: body %s face %s has %u triangles with positive area that "
                      "collapse to zero in the emitted float32 positions",
                      body_id.c_str(), face_key.c_str(), fn.float32CollapsedTriangles);
        }

        if (!fn.complete) {
            // Spec §14: "Incomplete display tessellation must be diagnostic." A
            // face whose degeneracy is CERTIFIED (Astra §3) keeps its zero range
            // with the certificate as its reason; everything else — including an
            // uncertain sliver — is a MISSING nondegenerate face, invalidates the
            // COMPLETE claim, and now fails the body (D11). Neither invents
            // geometry. The degeneracy decision is face-local: nothing about the
            // rest of the body may excuse a face that is perfectly valid on its
            // own (a 0.005 x 0.005 mm face on a 10 m body used to be excused).
            const std::string why = fn.diagnostic.empty() ? "no drawable triangle" : fn.diagnostic;
            const FaceDegeneracy degeneracy = classify_face_degeneracy(face);
            if (degeneracy.certified()) {
                out.completeness.degenerateFaces.push_back(face_key);
                WLOG_DEBUG("tessellate: face %s is CERTIFIED degenerate (%s); zero-triangle "
                           "range (%s)",
                           face_key.c_str(), degeneracy.reason.c_str(), why.c_str());
            } else {
                out.completeness.allNondegenerateFacesCovered = false;
                out.completeness.missingFaces.push_back(face_key);
                WLOG_WARN("tessellate: body %s face %s produced no triangles and its degeneracy "
                          "is NOT certified (%s); the display tessellation is incomplete (%s)",
                          body_id.c_str(), face_key.c_str(), degeneracy.reason.c_str(),
                          why.c_str());
            }
            mi.face_ranges.emplace_back(first_tri, 0);
            mi.face_ids.push_back(lbl.first);
            continue;
        }

        // Vertices are NOT shared across faces (each face owns its node block, and
        // any singular-split copy is appended inside that same block), so normals
        // are smoothed WITHIN a face and hard-split at every face boundary
        // (crease-split — mesh_format.md). A split changes vertex indices only:
        // triangle order and the face's triangle count are untouched.
        const std::uint32_t base = static_cast<std::uint32_t>(mi.positions.size() / 3);
        for (std::size_t v = 0; v < fn.vertexNode.size(); ++v) {
            const gp_Pnt& p = fn.worldNodes[fn.vertexNode[v]];
            mi.positions.push_back(static_cast<float>(p.X()));
            mi.positions.push_back(static_cast<float>(p.Y()));
            mi.positions.push_back(static_cast<float>(p.Z()));
        }
        mi.normals.insert(mi.normals.end(), fn.normals.begin(), fn.normals.end());
        for (const std::uint32_t local : fn.triangleVertexIndices) {
            mi.indices.push_back(base + local);
        }

        const std::uint32_t face_tris =
            static_cast<std::uint32_t>(fn.triangleVertexIndices.size() / 3);
        if (fn.droppedTriangles > 0) {
            WLOG_DEBUG("tessellate: face %s dropped %u degenerate triangles", face_key.c_str(),
                       fn.droppedTriangles);
        }
        mi.face_ranges.emplace_back(first_tri, face_tris);
        tri_cursor += face_tris;
        mi.face_ids.push_back(lbl.first);
    }

    if (out.completeness.normalFallbackNodes > 0 || out.completeness.singularSplitNodes > 0 ||
        out.completeness.normalUnresolvedNodes > 0 || out.completeness.normalMissingNodes > 0) {
        WLOG_DEBUG("tessellate: body %s normals — %u triangulation fallbacks, %u singular splits, "
                   "%u unresolved, %u without a source",
                   body_id.c_str(), out.completeness.normalFallbackNodes,
                   out.completeness.singularSplitNodes, out.completeness.normalUnresolvedNodes,
                   out.completeness.normalMissingNodes);
    }
    // NUM §9.2 type 19: local solid-partition ordinals for per-solid caps. Local
    // display metadata, never a persistent topology id, and only for a partition
    // that is independently verified closed.
    out.face_solid_ordinals = face_solid_ordinals(shape, faces);

    // --- edges (polylines) ---
    if (include_edges) {
        mi.has_edges = true;
        TopTools_IndexedMapOfShape edges;
        TopExp::MapShapes(shape, TopAbs_EDGE, edges);
        // NUM §2.5: the per-body segment budget is a RUNNING total, never handed
        // to each edge as if it owned the whole thing. The walk order is the
        // MapShapes order, so the split across edges is deterministic.
        const std::size_t body_segment_budget = detail::body_edge_segment_budget();
        std::size_t body_segments_left = body_segment_budget;
        bool budget_exhausted_reported = false;
        for (int ei = 1; ei <= edges.Extent(); ++ei) {
            const TopoDS_Edge edge = TopoDS::Edge(edges(ei));
            const std::string topo_key = "e:" + std::to_string(ei);
            const std::uint32_t first_point = static_cast<std::uint32_t>(mi.edge_positions.size() / 3);
            std::uint32_t point_count = 0;
            if (BRep_Tool::Degenerated(edge)) {
                // A degenerate edge (a cone apex, a sphere pole) legitimately has
                // no 3D curve and contributes no drawable segment. It still gets
                // its range and id so the edge tables stay index-aligned.
                WLOG_DEBUG("tessellate: edge %s is degenerate; no polyline", topo_key.c_str());
            } else {
                if (body_segments_left == 0) {
                    // PR-07 / D9: the declared per-body cap is a HARD cap. An edge
                    // that arrives after it is spent gets a zero-point range and
                    // keeps its id and its slot in the range/id tables, so the
                    // edge tables stay index-aligned and no consumer can mistake
                    // the body for complete. It emits NO geometry — the retired
                    // two-endpoint fallback added one uncounted segment per
                    // remaining edge, so five extra edges took a 2 000 000-segment
                    // body to 2 000 005. One warning per body, not per edge.
                    out.completeness.budgetLimitedEdges.push_back(topo_key);
                    if (!budget_exhausted_reported) {
                        WLOG_WARN(
                            "tessellate: body %s exhausted its %zu-segment edge budget at "
                            "edge %s; the remaining edges keep their ids and emit no polyline",
                            body_id.c_str(), body_segment_budget, topo_key.c_str());
                        budget_exhausted_reported = true;
                    }
                } else {
                    try {
                        BRepAdaptor_Curve curve(edge);
                        CurveSampleRequest request;
                        request.chordToleranceMm = lin;
                        request.angularToleranceRad = ang;
                        // The float32 allowance is per span and per axis and the
                        // sampler computes it from each span's own poles, so the
                        // caller reserves nothing extra.
                        request.representationAllowanceMm = 0.0;
                        CurveSampleLimits limits;
                        limits.remainingBodySegments = body_segments_left;
                        const CurveSampleResult sampled =
                            detail::sample_edge_polyline(curve, topo_key, request, limits);
                        if (sampled.certification == CurveCertification::Failed ||
                            sampled.points.size() < 2) {
                            // D11 / Astra F1: an edge is display-only, not
                            // pickable topology, so a failed nondegenerate edge
                            // keeps its id and a WARNED zero-point range and is
                            // recorded for WP10 — it never affects `ok`.
                            out.completeness.missingEdges.push_back(topo_key);
                            WLOG_WARN("tessellate: edge %s produced no polyline (%s)", topo_key.c_str(),
                                      sampled.diagnostic.c_str());
                        } else if (!all_points_finite(sampled.points)) {
                            out.completeness.missingEdges.push_back(topo_key);
                            WLOG_WARN("tessellate: edge %s produced a non-finite point; dropping it",
                                      topo_key.c_str());
                        } else {
                            if (sampled.certification == CurveCertification::KernelEstimated) {
                                WLOG_DEBUG(
                                    "tessellate: edge %s used the approximate fallback; sampled max "
                                    "error %.6g mm against %.6g mm",
                                    topo_key.c_str(), sampled.sampledMaxErrorMm, lin);
                            } else if (sampled.certification == CurveCertification::QualityLimited) {
                                WLOG_WARN("tessellate: edge %s is representation limited (%s)",
                                          topo_key.c_str(), sampled.diagnostic.c_str());
                            }
                            for (const gp_Pnt& p : sampled.points) {
                                mi.edge_positions.push_back(static_cast<float>(p.X()));
                                mi.edge_positions.push_back(static_cast<float>(p.Y()));
                                mi.edge_positions.push_back(static_cast<float>(p.Z()));
                                ++point_count;
                            }
                            const std::size_t used = sampled.points.size() - 1;
                            body_segments_left = used >= body_segments_left ? 0 : body_segments_left - used;
                        }
                    } catch (const Standard_Failure& failure) {
                        out.completeness.missingEdges.push_back(topo_key);
                        WLOG_WARN("tessellate: edge %s sampling raised %s", topo_key.c_str(),
                                  failure.GetMessageString());
                    } catch (const std::exception& error) {
                        out.completeness.missingEdges.push_back(topo_key);
                        WLOG_WARN("tessellate: edge %s sampling raised %s", topo_key.c_str(),
                                  error.what());
                    }
                }
            }
            mi.edge_ranges.emplace_back(first_point, point_count);
            const auto lbl = label('e', ei);
            any_elementid = any_elementid || lbl.second;
            mi.edge_ids.push_back(lbl.first);
        }
        // NUM §8: the feature-edge bitset and the edge/face incidence, computed on
        // the SAME maps the mesh is labelled by so no ordinal is renumbered. Not on
        // the wire until WP10 adds MESH1 v2 sections 16 and 20/21.
        out.edge_classes = classify_edges(shape, faces, edges);
    }

    mi.ids_have_elementids = any_elementid;

    // The face loop above pushed exactly one `face_ranges` entry per face of the
    // SAME `TopExp::MapShapes` map the caller indexed its colors by, so the two
    // vectors are index-aligned by construction. `encode_mesh1` re-checks the length
    // and drops the section on a mismatch rather than colouring the wrong faces.
    if (face_colors != nullptr && face_colors->size() == mi.face_ranges.size()) {
        mi.face_colors = *face_colors;
    }

    // bbox
    if (!box.IsVoid()) {
        Standard_Real xmin, ymin, zmin, xmax, ymax, zmax;
        box.Get(xmin, ymin, zmin, xmax, ymax, zmax);
        mi.bbox_min[0] = static_cast<float>(xmin);
        mi.bbox_min[1] = static_cast<float>(ymin);
        mi.bbox_min[2] = static_cast<float>(zmin);
        mi.bbox_max[0] = static_cast<float>(xmax);
        mi.bbox_max[1] = static_cast<float>(ymax);
        mi.bbox_max[2] = static_cast<float>(zmax);
    }

    out.triangle_count = static_cast<std::uint32_t>(mi.indices.size() / 3);
    out.blob = encode_mesh1(mi);
    // D11 / Astra F1: an incomplete display tessellation IS a failed
    // tessellation. The retired code assigned `out.ok = true` unconditionally, so
    // `allNondegenerateFacesCovered = false` was recorded in a field no caller
    // read and the body published as if it were whole — exactly what spec §14
    // forbids. Edges are deliberately excluded: they are display-only and keep a
    // warned zero-point range instead.
    out.ok = out.completeness.allNondegenerateFacesCovered;
    if (!out.ok) {
        out.diagnostic = "the display tessellation is incomplete: " +
                         std::to_string(out.completeness.missingFaces.size()) +
                         " nondegenerate face(s) produced no triangles (";
        for (std::size_t i = 0; i < out.completeness.missingFaces.size(); ++i) {
            if (i >= 16) {
                out.diagnostic += ", ...";
                break;
            }
            if (i > 0) out.diagnostic += ", ";
            out.diagnostic += out.completeness.missingFaces[i];
        }
        out.diagnostic += ")";
    }
    return out;
}

RawMesh tessellate_raw(const TopoDS_Shape& shape, const std::string& lod) {
    RawMesh out;
    if (shape.IsNull()) return out;

    Bnd_Box box;
    BRepBndLib::Add(shape, box);
    double diag = 1.0;
    if (!box.IsVoid()) {
        Standard_Real xmin, ymin, zmin, xmax, ymax, zmax;
        box.Get(xmin, ymin, zmin, xmax, ymax, zmax);
        diag = gp_Pnt(xmin, ymin, zmin).Distance(gp_Pnt(xmax, ymax, zmax));
    }
    double lin = 0.1, ang = 0.5;
    deflections(lod, diag, lin, ang);

    // Single-threaded meshing for determinism (Invariant 5). Same params as
    // tessellate_body, so the produced triangle set is identical.
    BRepMesh_IncrementalMesh mesher(shape, lin, Standard_False, ang, Standard_False);
    mesher.Perform();

    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(shape, TopAbs_FACE, faces);

    for (int fi = 1; fi <= faces.Extent(); ++fi) {
        const TopoDS_Face face = TopoDS::Face(faces(fi));
        TopLoc_Location loc;
        Handle(Poly_Triangulation) tri = BRep_Tool::Triangulation(face, loc);
        if (tri.IsNull() || tri->NbNodes() < 3 || tri->NbTriangles() < 1) continue;
        const gp_Trsf trsf = loc.Transformation();
        const bool reversed = (face.Orientation() == TopAbs_REVERSED);

        const std::uint32_t base = static_cast<std::uint32_t>(out.positions.size() / 3);
        const int nb_nodes = tri->NbNodes();
        std::vector<gp_Vec> accum(static_cast<std::size_t>(nb_nodes), gp_Vec(0, 0, 0));
        std::vector<gp_Pnt> pts(static_cast<std::size_t>(nb_nodes));
        for (int i = 1; i <= nb_nodes; ++i) pts[static_cast<std::size_t>(i - 1)] = tri->Node(i).Transformed(trsf);

        for (int t = 1; t <= tri->NbTriangles(); ++t) {
            Standard_Integer n1, n2, n3;
            tri->Triangle(t).Get(n1, n2, n3);
            if (reversed) std::swap(n2, n3);  // outward winding
            const gp_Pnt& a = pts[static_cast<std::size_t>(n1 - 1)];
            const gp_Pnt& b = pts[static_cast<std::size_t>(n2 - 1)];
            const gp_Pnt& c = pts[static_cast<std::size_t>(n3 - 1)];
            gp_Vec normal = gp_Vec(a, b).Crossed(gp_Vec(a, c));  // area-weighted
            accum[static_cast<std::size_t>(n1 - 1)] += normal;
            accum[static_cast<std::size_t>(n2 - 1)] += normal;
            accum[static_cast<std::size_t>(n3 - 1)] += normal;
            out.indices.push_back(base + static_cast<std::uint32_t>(n1 - 1));
            out.indices.push_back(base + static_cast<std::uint32_t>(n2 - 1));
            out.indices.push_back(base + static_cast<std::uint32_t>(n3 - 1));
        }

        for (int i = 0; i < nb_nodes; ++i) {
            const gp_Pnt& p = pts[static_cast<std::size_t>(i)];
            out.positions.push_back(static_cast<float>(p.X()));
            out.positions.push_back(static_cast<float>(p.Y()));
            out.positions.push_back(static_cast<float>(p.Z()));
            gp_Vec n = accum[static_cast<std::size_t>(i)];
            if (n.Magnitude() > 1e-12) {
                n.Normalize();
            } else {
                n = gp_Vec(0, 0, 1);
            }
            out.normals.push_back(static_cast<float>(n.X()));
            out.normals.push_back(static_cast<float>(n.Y()));
            out.normals.push_back(static_cast<float>(n.Z()));
        }
    }

    out.triangle_count = static_cast<std::uint32_t>(out.indices.size() / 3);
    return out;
}

}  // namespace onecad::tess
