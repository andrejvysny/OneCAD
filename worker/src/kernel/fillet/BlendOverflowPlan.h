// BlendOverflowPlan.h — INTERNAL to BlendOverflow: qualification of the seed edge
// and the 2D section geometry of the blend. Not a public API; the only consumer is
// BlendOverflow.cpp, which owns the tool prism, the Cut and the certification.
#pragma once

#include <NCollection_IndexedDataMap.hxx>
#include <NCollection_List.hxx>
#include <TopTools_ShapeMapHasher.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <gp_Pnt.hxx>
#include <gp_Vec.hxx>
#include <gp_XY.hxx>

#include "kernel/fillet/BlendOverflow.h"

namespace onecad::kernel::fillet::overflow_detail {

constexpr double kPi = 3.14159265358979323846;
constexpr double kParallel = 1.0e-9; // 1 - |cos| bound for "parallel/perpendicular"

using AncestorMap = NCollection_IndexedDataMap<TopoDS_Shape, NCollection_List<TopoDS_Shape>,
                                               TopTools_ShapeMapHasher>;

struct Support {
  TopoDS_Face face;
  gp_Vec n, t, m; // outward normal, inward in-face dir, in-section inward normal
  gp_Pnt centroid;
  TopoDS_Edge far_edge;
  double width = 0.0;
  double setback = 0.0;
};

// Everything the construction needs, measured once from the body.
struct Frame {
  gp_Pnt origin;
  gp_Vec e;
  double length = 0.0;
  double res = 1.0e-3;
  double alpha = 0.0; // interior dihedral angle, rad
  Support s[2];
  AncestorMap edge_faces, vertex_faces;
};

// The section plane through `origin`, in the basis (u = surviving support's
// inward tangent, v = its inward normal). All blend geometry is 2D in it.
struct Section {
  gp_Pnt o;
  gp_Vec u, v;
  gp_XY xy(const gp_Vec &d) const { return gp_XY(d * u, d * v); }
  gp_Pnt pt(const gp_XY &p) const { return o.Translated(u * p.X() + v * p.Y()); }
};

struct Plan {
  OverflowKind kind = OverflowKind::Cliff;
  int consumed = 0;                // index into Frame::s
  double pad = 1.0;
  double expected_area = 0.0;
  double setback = 0.0;
  gp_Pnt line_a, line_b;           // chamfer: the two setback points on the plane
  gp_Pnt blend_mid;                // section-plane point on the blend profile edge
  TopoDS_Face profile;             // closed tool profile at the section through origin
};

bool fail(OverflowResult &r, const char *code, std::string msg);

// Value check, edge/support/cap/quad qualification and side choice (steps 1-6).
bool qualify(const OverflowRequest &q, Frame &f, Plan &p, OverflowResult &r);

// Solves the blend in the section plane and builds the tool profile.
bool plan_blend(const OverflowRequest &q, const Frame &f, Plan &p, OverflowResult &r);

} // namespace onecad::kernel::fillet::overflow_detail
