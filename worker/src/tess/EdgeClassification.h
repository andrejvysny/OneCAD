// EdgeClassification.h — per-edge display semantics for the feature-edge policy
// (VP-HARDENING WP09, design NUM §8.1/§8.2). INTERNAL for now: the bitset and the
// incidence list are computed here and carried on `BodyMesh`; WP10 publishes them
// as MESH1 v2 sections 16 (`EDGE_FLAGS`), 19 (`FACE_SOLID_IDS`), 20/21
// (`EDGE_FACE_OFFSETS`/`EDGE_FACE_ORDINALS`).
//
// The rule that governs everything here (NUM §8.2): sampling is a DIAGNOSTIC
// SCREEN, never permission to hide an edge. An edge is upgraded to `tangent` only
// from trustworthy continuity metadata whose orientation/location consistency has
// been checked, or from an analytic supporting-surface equivalence that holds over
// the whole edge. A join that merely LOOKS smooth at every sample stays `unknown`
// and stays visible — samples can miss a localized crease exactly as they can miss
// a spline excursion.
#pragma once

#include <cstdint>
#include <vector>

#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>

namespace onecad::tess {

// NUM §8.1 bit assignments. Bits 7-31 are reserved and MUST stay zero.
inline constexpr std::uint32_t kEdgeOpen = 1U << 0;
inline constexpr std::uint32_t kEdgeHard = 1U << 1;
inline constexpr std::uint32_t kEdgeTangent = 1U << 2;
inline constexpr std::uint32_t kEdgeSeam = 1U << 3;
inline constexpr std::uint32_t kEdgeDegenerate = 1U << 4;
inline constexpr std::uint32_t kEdgeNonmanifold = 1U << 5;
inline constexpr std::uint32_t kEdgeUnknown = 1U << 6;

// The dihedral above which a MEASURED join is a real crease rather than sampling
// noise. It is NUM §7.4's tangent-seam agreement window (0.2 degrees), NOT the
// mesher's 5 degree angular deflection: "Never classify a shallow but real 1 deg
// crease as tangent merely because the surface mesher uses a 5 deg angular
// deflection" (NUM §8.2). A join measured below it is still only `unknown`.
inline constexpr double kEdgeCreaseRad = 3.4906585039886593e-3;  // 0.2 degrees

// Interior parameters at which both faces' oriented normals are compared.
inline constexpr int kEdgeContinuitySamples = 3;

// WP09 Astra F6. What the three-sample screen actually established across one
// edge. `evaluable` counts only samples where BOTH faces returned a normal no
// derivative-error budget REFUTES (SurfaceNormals.h): a normal a budget bounds
// away from usefulness is no evidence, and the retired code simply skipped such
// a sample and then treated the remainder as if the whole edge had been
// screened. A representation for which no enclosure exists refutes nothing, so
// its samples still count (R1(c) MINOR 6).
struct EdgeSampleAgreement {
    int required = kEdgeContinuitySamples;
    int evaluable = 0;
    double maxDihedralRad = 0.0;
    bool complete() const { return required > 0 && evaluable == required; }
};

// The agreement measured across `edge` between its two incident faces.
EdgeSampleAgreement sample_edge_agreement(const TopoDS_Face& f1, const TopoDS_Face& f2,
                                          const TopoDS_Edge& edge);

// NUM §8.2's tangency decision, separated from its evidence so the "a required
// sample was not evaluable" branch is testable without a pathological OCCT face.
//
// Astra F6: tangency now needs (i) trusted `GeomAbs_G1`-or-better continuity
// metadata on the edge, or (ii) the two faces sharing the SAME supporting surface
// handle at the SAME location — AND, in both cases, every required sample
// evaluable with a bounded normal and consistent within kEdgeCreaseRad. Two
// r = 0.1 mm spheres whose centres are 0.0001 mm apart used to pass the retired
// "tolerance-close analytic parameters" equivalence while their real dihedral is
// 0.0573 degrees, so distinct intersecting spheres became tangent. A measured
// crease above kEdgeCreaseRad is still definitive evidence of a discontinuity
// even when other samples failed, so it answers `hard`.
std::uint32_t decide_edge_continuity(bool trustedG1, bool sameSupportingSurface,
                                     const EdgeSampleAgreement& agreement);

// `FACE_SOLID_IDS` (NUM §9.2) for a face that belongs to no independently
// verified closed solid partition.
inline constexpr std::uint32_t kNoSolidOrdinal = 0xFFFFFFFFU;

struct EdgeClass {
    std::uint32_t flags = 0;
    // Mesh-local face ordinals (0-based indices into the same `TopExp::MapShapes`
    // face map the mesh is labelled by), sorted and unique. A seam names its one
    // supporting face once; a free wire edge names none (NUM §9.2).
    std::vector<std::uint32_t> faceOrdinals;
};

// One entry per edge of `edges`, in that map's order. `faces` and `edges` must be
// the maps the caller labelled the mesh with, so the ordinals agree with
// FACE_RANGES/EDGE_RANGES without any renumbering of the topology.
std::vector<EdgeClass> classify_edges(const TopoDS_Shape& shape,
                                      const TopTools_IndexedMapOfShape& faces,
                                      const TopTools_IndexedMapOfShape& edges);

// One local solid-partition ordinal per face, or kNoSolidOrdinal when the face is
// outside every solid, shared by more than one, or inside a solid that is not
// verified closed. Cap eligibility is decided from this, so an unverified
// partition must never receive an ordinal (NUM §9.2).
std::vector<std::uint32_t> face_solid_ordinals(const TopoDS_Shape& shape,
                                               const TopTools_IndexedMapOfShape& faces);

}  // namespace onecad::tess
