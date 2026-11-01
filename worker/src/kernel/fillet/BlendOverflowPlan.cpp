#include "kernel/fillet/BlendOverflowPlan.h"

#include <algorithm>
#include <cmath>
#include <vector>

#include <BRepAdaptor_Curve.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakePolygon.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepGProp.hxx>
#include <BRepTools.hxx>
#include <BRep_Tool.hxx>
#include <GC_MakeArcOfCircle.hxx>
#include <GProp_GProps.hxx>
#include <Geom_TrimmedCurve.hxx>
#include <Precision.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Wire.hxx>

namespace onecad::kernel::fillet::overflow_detail {

bool fail(OverflowResult &r, const char *code, std::string msg) {
  r.ok = false;
  r.reason_code = code;
  r.message = std::move(msg);
  return false;
}

namespace {

bool is_line(const TopoDS_Edge &edge) {
  return BRepAdaptor_Curve(edge).GetType() == GeomAbs_Line;
}

gp_Pnt vertex_point(const TopoDS_Vertex &v) { return BRep_Tool::Pnt(v); }

gp_Vec face_normal(const TopoDS_Face &face) {
  BRepAdaptor_Surface surf(face);
  auto mid = [](double a, double b) {
    return (Precision::IsInfinite(a) || Precision::IsInfinite(b)) ? 0.0 : 0.5 * (a + b);
  };
  gp_Pnt p;
  gp_Vec du, dv;
  surf.D1(mid(surf.FirstUParameter(), surf.LastUParameter()),
          mid(surf.FirstVParameter(), surf.LastVParameter()), p, du, dv);
  gp_Vec n = du.Crossed(dv);
  n.Normalize();
  return face.Orientation() == TopAbs_REVERSED ? n.Reversed() : n;
}

// Convex iff each face's inward tangent lies below the other face's plane.
bool convex_pair(const gp_Vec &na, const gp_Vec &ta, const gp_Vec &nb, const gp_Vec &tb) {
  return na * tb < -kParallel && nb * ta < -kParallel;
}

// Inward in-face direction perpendicular to `e`, pointing toward the face
// centroid as seen from `from`.
gp_Vec inward_tangent(const gp_Vec &n, const gp_Vec &e, const gp_Pnt &centroid,
                      const gp_Pnt &from) {
  gp_Vec t = n.Crossed(e);
  t.Normalize();
  return gp_Vec(from, centroid) * t < 0.0 ? t.Reversed() : t;
}

bool valid_values(const OverflowRequest &q, double res, OverflowResult &r) {
  const bool chamfer = q.mode == EdgeOpMode::Chamfer;
  auto bad = [&](double v) { return !std::isfinite(v) || v < res; };
  if (bad(q.value) || (chamfer && bad(q.value2)))
    return fail(r, "OVERFLOW_INVALID_VALUE", "blend value must be finite and >= resolution");
  if (chamfer && q.reference_face.IsNull() && q.value2 != q.value)
    return fail(r, "OVERFLOW_INVALID_VALUE", "equal-leg chamfer requires value2 == value");
  return true;
}

bool make_support(const TopoDS_Face &face, Frame &f, Support &s) {
  s.face = face;
  s.n = face_normal(face);
  GProp_GProps g;
  BRepGProp::SurfaceProperties(face, g);
  s.centroid = g.CentreOfMass();
  s.t = inward_tangent(s.n, f.e, s.centroid, f.origin);
  s.m = s.n.Reversed();
  return true;
}

// Seed -> frame, supports, convexity, dihedral range.
bool qualify_edge(const OverflowRequest &q, Frame &f, OverflowResult &r) {
  if (q.seed.IsNull() || BRep_Tool::Degenerated(q.seed) || !is_line(q.seed))
    return fail(r, "OVERFLOW_NOT_ISOLATED", "seed is not a straight BRep line edge");
  TopExp::MapShapesAndAncestors(q.body, TopAbs_EDGE, TopAbs_FACE, f.edge_faces);
  TopExp::MapShapesAndAncestors(q.body, TopAbs_VERTEX, TopAbs_FACE, f.vertex_faces);
  const int idx = f.edge_faces.FindIndex(q.seed);
  if (idx == 0 || f.edge_faces(idx).Extent() != 2)
    return fail(r, "OVERFLOW_NON_PLANAR_SUPPORT", "seed does not have exactly two faces");
  const TopoDS_Face fa = TopoDS::Face(f.edge_faces(idx).First());
  const TopoDS_Face fb = TopoDS::Face(f.edge_faces(idx).Last());
  if (fa.IsSame(fb) || BRepAdaptor_Surface(fa).GetType() != GeomAbs_Plane ||
      BRepAdaptor_Surface(fb).GetType() != GeomAbs_Plane)
    return fail(r, "OVERFLOW_NON_PLANAR_SUPPORT", "supports must be two distinct planes");
  TopoDS_Vertex v1, v2;
  TopExp::Vertices(q.seed, v1, v2, true);
  f.origin = vertex_point(v1);
  gp_Vec d(f.origin, vertex_point(v2));
  f.length = d.Magnitude();
  if (f.length < f.res)
    return fail(r, "OVERFLOW_NOT_ISOLATED", "seed edge is degenerate");
  f.e = d.Normalized();
  make_support(fa, f, f.s[0]);
  make_support(fb, f, f.s[1]);
  const double c = f.s[0].t * f.s[1].t;
  f.alpha = std::acos(std::max(-1.0, std::min(1.0, c)));
  const double deg = f.alpha * 180.0 / kPi;
  if (!convex_pair(f.s[0].n, f.s[0].t, f.s[1].n, f.s[1].t) || deg < 20.0 || deg > 160.0)
    return fail(r, "OVERFLOW_DIHEDRAL_OUT_OF_RANGE",
                "edge must be convex with dihedral in [20, 160] deg; got " + std::to_string(deg));
  return true;
}

// Exactly one planar cap face per seed vertex, perpendicular to the seed. Checked
// BEFORE the prismatic test: a planar cap that meets two rectangle supports is
// perpendicular by construction, so run after it this refusal would be unreachable.
bool check_caps(const OverflowRequest &q, const Frame &f, OverflowResult &r) {
  TopoDS_Vertex v[2];
  TopExp::Vertices(q.seed, v[0], v[1], true);
  for (const TopoDS_Vertex &vx : v) {
    const int idx = f.vertex_faces.FindIndex(vx);
    std::vector<TopoDS_Face> caps;
    for (const TopoDS_Shape &sh : f.vertex_faces(idx))
      // The ancestor list repeats a face once per incident edge; dedupe.
      if (!sh.IsSame(f.s[0].face) && !sh.IsSame(f.s[1].face) &&
          std::none_of(caps.begin(), caps.end(),
                       [&](const TopoDS_Face &c) { return c.IsSame(sh); }))
        caps.push_back(TopoDS::Face(sh));
    if (caps.size() > 1)
      return fail(r, "OVERFLOW_NOT_ISOLATED", "more than one cap face at a seed vertex");
    if (caps.empty() || BRepAdaptor_Surface(caps[0]).GetType() != GeomAbs_Plane ||
        std::abs(face_normal(caps[0]) * f.e) < 1.0 - kParallel)
      return fail(r, "OVERFLOW_CAP_NOT_PERPENDICULAR", "cap is missing, non-planar or oblique");
  }
  return true;
}

// A support must be a rectangle spanning the seed: 4 line edges, one hole-free
// wire, far edge parallel to the seed and projecting onto parameters 0 and L.
bool check_quad(const OverflowRequest &q, const Frame &f, Support &s, OverflowResult &r) {
  auto refuse = [&](const char *why) { return fail(r, "OVERFLOW_NOT_PRISMATIC", why); };
  int wires = 0;
  for (TopExp_Explorer ex(s.face, TopAbs_WIRE); ex.More(); ex.Next())
    ++wires;
  if (wires != 1)
    return refuse("support has inner wires");
  std::vector<TopoDS_Edge> edges;
  for (TopExp_Explorer ex(BRepTools::OuterWire(s.face), TopAbs_EDGE); ex.More(); ex.Next())
    edges.push_back(TopoDS::Edge(ex.Current()));
  TopoDS_Vertex sv1, sv2;
  TopExp::Vertices(q.seed, sv1, sv2);
  bool seed_in = false;
  int far_count = 0;
  for (const TopoDS_Edge &ed : edges) {
    if (!is_line(ed))
      return refuse("support has a non-line edge");
    seed_in = seed_in || ed.IsSame(q.seed);
    TopoDS_Vertex a, b;
    TopExp::Vertices(ed, a, b);
    if (!a.IsSame(sv1) && !a.IsSame(sv2) && !b.IsSame(sv1) && !b.IsSame(sv2)) {
      s.far_edge = ed;
      ++far_count;
    }
  }
  if (edges.size() != 4 || !seed_in || far_count != 1)
    return refuse("support is not a four-edge quad containing the seed");
  TopoDS_Vertex a, b;
  TopExp::Vertices(s.far_edge, a, b);
  const gp_Pnt pa = vertex_point(a), pb = vertex_point(b);
  if (std::abs(gp_Vec(pa, pb).Normalized() * f.e) < 1.0 - kParallel)
    return refuse("far edge is not parallel to the seed");
  const double sa = gp_Vec(f.origin, pa) * f.e, sb = gp_Vec(f.origin, pb) * f.e;
  const bool spans = (std::abs(sa) <= f.res && std::abs(sb - f.length) <= f.res) ||
                     (std::abs(sb) <= f.res && std::abs(sa - f.length) <= f.res);
  const double wa = gp_Vec(f.origin, pa) * s.t, wb = gp_Vec(f.origin, pb) * s.t;
  if (!spans || std::abs(wa - wb) > f.res || 0.5 * (wa + wb) <= f.res)
    return refuse("far edge does not span the seed or has no positive width");
  s.width = 0.5 * (wa + wb);
  return true;
}

// Per-support setbacks and which support the blend consumes.
bool choose_sides(const OverflowRequest &q, Frame &f, Plan &p, OverflowResult &r) {
  if (q.mode == EdgeOpMode::Chamfer) {
    double d0 = q.value, d1 = q.value;
    if (!q.reference_face.IsNull()) {
      const bool first = q.reference_face.IsSame(f.s[0].face);
      if (!first && !q.reference_face.IsSame(f.s[1].face))
        return fail(r, "OVERFLOW_INVALID_VALUE", "reference face is not a support of the seed");
      d0 = first ? q.value : q.value2;
      d1 = first ? q.value2 : q.value;
    }
    f.s[0].setback = d0;
    f.s[1].setback = d1;
  } else {
    // Native setback; used only to classify. The true tangency offset is solved
    // later against the cliff point.
    f.s[0].setback = f.s[1].setback = q.value / std::tan(0.5 * f.alpha);
  }
  const bool over0 = f.s[0].setback >= f.s[0].width - f.res;
  const bool over1 = f.s[1].setback >= f.s[1].width - f.res;
  if (!over0 && !over1)
    return fail(r, "OVERFLOW_NOT_OVERFLOWING", "blend fits inside both supports; use the native path");
  if (over0 && over1) {
    // The narrower face is the one the blend swallows; the wider is checked
    // against the solved setback later. Equal widths leave no survivor.
    if (std::abs(f.s[0].width - f.s[1].width) <= f.res)
      return fail(r, "OVERFLOW_BOTH_SUPPORTS_CONSUMED", "both supports are equally narrow");
    p.consumed = f.s[0].width < f.s[1].width ? 0 : 1;
  } else {
    p.consumed = over0 ? 0 : 1;
  }
  // [w - res, w) classifies as overflow but is a sub-resolution sliver of the
  // consumed face, not a consumption; never snapped.
  const Support &c = f.s[p.consumed];
  if (q.mode == EdgeOpMode::Chamfer && c.setback < c.width)
    return fail(r, "OVERFLOW_REMNANT",
                "chamfer distance is within authoring resolution of the consumed face width " +
                    std::to_string(c.width));
  return true;
}

bool intersect(const gp_XY &p, const gp_XY &d, const gp_XY &q, const gp_XY &e, double &t) {
  const double den = d.X() * e.Y() - d.Y() * e.X();
  if (std::abs(den) < 1.0e-12)
    return false;
  t = ((q.X() - p.X()) * e.Y() - (q.Y() - p.Y()) * e.X()) / den;
  return true;
}

// The cliff face: the other face on the consumed face's far edge, convex with it.
bool find_cliff(const Frame &f, const Plan &p, TopoDS_Face &cliff, gp_Vec &n_c, gp_Vec &t_c,
                OverflowResult &r) {
  const Support &c = f.s[p.consumed];
  const int idx = f.edge_faces.FindIndex(c.far_edge);
  if (idx == 0 || f.edge_faces(idx).Extent() != 2)
    return fail(r, "OVERFLOW_NOT_PRISMATIC", "consumed far edge has no unique cliff face");
  for (const TopoDS_Shape &sh : f.edge_faces(idx))
    if (!sh.IsSame(c.face))
      cliff = TopoDS::Face(sh);
  if (cliff.IsNull() || BRepAdaptor_Surface(cliff).GetType() != GeomAbs_Plane)
    return fail(r, "OVERFLOW_NOT_PRISMATIC", "cliff face is not planar");
  int wires = 0;
  for (TopExp_Explorer ex(cliff, TopAbs_WIRE); ex.More(); ex.Next())
    ++wires;
  if (wires != 1)
    return fail(r, "OVERFLOW_NOT_PRISMATIC", "cliff face has inner wires");
  GProp_GProps g;
  BRepGProp::SurfaceProperties(cliff, g);
  const gp_Pnt far_pt = f.origin.Translated(c.t * c.width);
  n_c = face_normal(cliff);
  t_c = inward_tangent(n_c, f.e, g.CentreOfMass(), far_pt);
  if (!convex_pair(c.n, c.t.Reversed(), n_c, t_c))
    return fail(r, "OVERFLOW_NOT_PRISMATIC", "cliff face is not convex with the consumed face");
  return true;
}

double polygon_area(const std::vector<gp_XY> &v) {
  double a = 0.0;
  for (size_t i = 0; i < v.size(); ++i) {
    const gp_XY &p = v[i], &n = v[(i + 1) % v.size()];
    a += p.X() * n.Y() - n.X() * p.Y();
  }
  return std::abs(0.5 * a);
}

TopoDS_Face profile_from_wire(BRepBuilderAPI_MakeWire &mw) {
  if (!mw.IsDone())
    return TopoDS_Face();
  BRepBuilderAPI_MakeFace mf(mw.Wire(), true);
  return mf.IsDone() ? mf.Face() : TopoDS_Face();
}

bool plan_chamfer(const Frame &f, const Section &sec, const gp_XY &ti, const gp_XY &ni,
                  const TopoDS_Face &cliff, const gp_Vec &t_c, Plan &p, OverflowResult &r) {
  const Support &sj = f.s[1 - p.consumed], &si = f.s[p.consumed];
  if (sj.setback > sj.width - f.res)
    return fail(r, "OVERFLOW_BOTH_SUPPORTS_CONSUMED", "chamfer distance consumes both supports");
  const gp_XY A(sj.setback, 0.0), B = ti * si.setback, P = ti * si.width;
  const gp_XY tc = sec.xy(t_c), lc = B - A;
  double mu = 0.0, lam = 0.0;
  if (!intersect(P, tc, A, lc, mu) || mu < 0.0)
    return fail(r, "OVERFLOW_AMBIGUOUS", "chamfer plane does not meet the cliff face ahead of P");
  p.expected_area = polygon_area({gp_XY(0, 0), A, P + tc * mu, P});
  p.setback = sj.setback;
  const gp_XY j0(0.0, -p.pad), jd(1.0, 0.0), i0 = ni * p.pad;
  gp_XY v1, v2, v3;
  if (!intersect(j0, jd, i0, ti, lam))
    return fail(r, "OVERFLOW_AMBIGUOUS", "padded offset lines are parallel");
  v1 = j0 + jd * lam;
  if (!intersect(A, lc, j0, jd, lam))
    return fail(r, "OVERFLOW_AMBIGUOUS", "chamfer plane parallel to support");
  v2 = A + lc * lam;
  if (!intersect(A, lc, i0, ti, lam))
    return fail(r, "OVERFLOW_AMBIGUOUS", "chamfer plane parallel to consumed face");
  v3 = A + lc * lam;
  BRepBuilderAPI_MakePolygon poly(sec.pt(v1), sec.pt(v2), sec.pt(v3), true);
  if (!poly.IsDone())
    return fail(r, "OVERFLOW_BOOLEAN_FAILED", "chamfer tool profile wire failed");
  p.profile = BRepBuilderAPI_MakeFace(poly.Wire(), true).Face();
  p.blend_mid = sec.pt((v2 + v3) * 0.5);
  p.line_a = sec.pt(A);
  p.line_b = sec.pt(B);
  (void)cliff;
  return true;
}

// Nine interior arc samples must stay inside the wedge: not above the surviving
// support (y >= 0, within res) and on the inward side of the consumed carrier.
bool arc_inside_wedge(const gp_XY &A, const gp_XY &C, const gp_XY &P, double rad,
                      const gp_XY &mi, double res) {
  const gp_XY ua = (A - C) / rad, up = (P - C) / rad;
  const double delta = std::atan2(ua.X() * up.Y() - ua.Y() * up.X(), ua * up);
  const double phi0 = std::atan2(ua.Y(), ua.X());
  for (int k = 1; k <= 9; ++k) {
    const double phi = phi0 + delta * (k / 10.0);
    const gp_XY q = C + gp_XY(std::cos(phi), std::sin(phi)) * rad;
    if (q.Y() < -res || q * mi < -res)
      return false;
  }
  return true;
}

bool plan_fillet(const OverflowRequest &q, const Frame &f, const Section &sec, const gp_XY &ti,
                 const gp_XY &mi, Plan &p, OverflowResult &r) {
  const Support &sj = f.s[1 - p.consumed], &si = f.s[p.consumed];
  const double rad = q.value;
  const gp_XY P = ti * si.width;
  const double disc = 2.0 * rad * P.Y() - P.Y() * P.Y();
  if (disc <= 0.0)
    return fail(r, "OVERFLOW_AMBIGUOUS", "no circle of this radius is tangent to the support and passes the cliff point");
  // ALWAYS the + root: in the overflow regime s grows with r and meets the native
  // setback at the threshold. The - root puts the circle centre on the wrong side
  // of the consumed face (or, at alpha = 60 deg, degenerates to tangency at the
  // seed itself), which is not the cliff blend.
  const double s = P.X() + std::sqrt(disc);
  if (s > sj.width - f.res)
    return fail(r, "OVERFLOW_BOTH_SUPPORTS_CONSUMED", "tangency offset " + std::to_string(s) + " reaches the far edge of the surviving support");
  if (s - P.X() < f.res)
    return fail(r, "OVERFLOW_AMBIGUOUS", "arc is degenerate");
  const gp_XY A(s, 0.0), C(s, rad);
  gp_XY bis = gp_XY(0.0, -1.0) + (P - C) / rad;
  if (bis.Modulus() < 1.0e-9)
    return fail(r, "OVERFLOW_AMBIGUOUS", "arc is a semicircle");
  bis.Normalize();
  const gp_XY M = C + bis * rad;
  const double chord = (A - P).Modulus();
  const double theta = 2.0 * std::asin(std::min(1.0, chord / (2.0 * rad)));
  if (!arc_inside_wedge(A, C, P, rad, mi, f.res))
    return fail(r, "OVERFLOW_AMBIGUOUS", "blend arc leaves the wedge between the supports");
  p.expected_area = 0.5 * s * P.Y() - 0.5 * rad * rad * (theta - std::sin(theta));
  p.setback = s;
  Handle(Geom_TrimmedCurve) arc = GC_MakeArcOfCircle(sec.pt(A), sec.pt(M), sec.pt(P)).Value();
  const gp_XY vc = P + (ti - mi) * p.pad, v1 = gp_XY(0.0, -p.pad) - mi * p.pad,
              v2(s, -p.pad);
  BRepBuilderAPI_MakeWire mw;
  mw.Add(BRepBuilderAPI_MakeEdge(arc).Edge());
  const gp_XY pts[] = {P, vc, v1, v2, A};
  for (int i = 0; i < 4; ++i)
    mw.Add(BRepBuilderAPI_MakeEdge(sec.pt(pts[i]), sec.pt(pts[i + 1])).Edge());
  p.profile = profile_from_wire(mw);
  p.blend_mid = sec.pt(M);
  if (p.profile.IsNull())
    return fail(r, "OVERFLOW_BOOLEAN_FAILED", "fillet tool profile wire failed");
  return true;
}


} // namespace

bool qualify(const OverflowRequest &q, Frame &f, Plan &p, OverflowResult &r) {
  return valid_values(q, f.res, r) && qualify_edge(q, f, r) && check_caps(q, f, r) &&
         check_quad(q, f, f.s[0], r) && check_quad(q, f, f.s[1], r) && choose_sides(q, f, p, r);
}

// Geometry stage: solves the blend in the section plane and builds the profile.
bool plan_blend(const OverflowRequest &q, const Frame &f, Plan &p, OverflowResult &r) {
  const Support &sj = f.s[1 - p.consumed], &si = f.s[p.consumed];
  p.pad = std::max(1.0, 0.1 * std::max({si.width, sj.width, f.length}));
  TopoDS_Face cliff;
  gp_Vec n_c, t_c;
  if (!find_cliff(f, p, cliff, n_c, t_c, r))
    return false;
  r.cliff_face = cliff;
  const Section sec{f.origin, sj.t, sj.m};
  const gp_XY ti = sec.xy(si.t), mi = sec.xy(si.m), ni = sec.xy(si.n);
  p.kind = q.mode == EdgeOpMode::Chamfer ? OverflowKind::ExtendedPlane : OverflowKind::Cliff;
  return p.kind == OverflowKind::ExtendedPlane
             ? plan_chamfer(f, sec, ti, ni, cliff, t_c, p, r)
             : plan_fillet(q, f, sec, ti, mi, p, r);
}
} // namespace onecad::kernel::fillet::overflow_detail
