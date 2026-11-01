// test_blend_overflow.cpp — overflow fillet (cliff) / chamfer (extended plane).
//
// Fixture: 20x30x4 box, seed = the edge along y at x=0,z=4. Native OCCT refuses
// fillet r >= 4 and chamfer d >= 4 here; the overflow builder must reproduce the
// closed-form retained volumes, and refuse everything outside its envelope.
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include <BRepAdaptor_Surface.hxx>
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRep_Builder.hxx>
#include <TopoDS_Compound.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakePolygon.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepFilletAPI_MakeChamfer.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <Standard_Failure.hxx>
#include <BRepGProp.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRep_Tool.hxx>
#include <GProp_GProps.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <NCollection_IndexedMap.hxx>
#include <TopTools_ShapeMapHasher.hxx>
#include <TopoDS.hxx>
#include <gp_Ax1.hxx>
#include <gp_Ax2.hxx>
#include <gp_Trsf.hxx>

#include "kernel/fillet/BlendOverflow.h"
#include "util/Cancel.h"

namespace kf = onecad::kernel::fillet;

namespace {

constexpr double kPi = 3.14159265358979323846;
int failures = 0;

void check(bool ok, const std::string &msg) {
  if (ok)
    return;
  std::fprintf(stderr, "FAIL: %s\n", msg.c_str());
  ++failures;
}

TopoDS_Shape make_box() { return BRepPrimAPI_MakeBox(20.0, 30.0, 4.0).Shape(); }

// The line edge with both vertices at x=0, z=4 (the test body keeps that edge).
TopoDS_Edge seed_at(const TopoDS_Shape &body, double x, double z) {
  for (TopExp_Explorer ex(body, TopAbs_EDGE); ex.More(); ex.Next()) {
    const TopoDS_Edge e = TopoDS::Edge(ex.Current());
    TopoDS_Vertex a, b;
    TopExp::Vertices(e, a, b);
    if (a.IsNull() || b.IsNull())
      continue;
    const gp_Pnt p = BRep_Tool::Pnt(a), q = BRep_Tool::Pnt(b);
    if (std::abs(p.X() - x) < 1e-9 && std::abs(q.X() - x) < 1e-9 && std::abs(p.Z() - z) < 1e-9 &&
        std::abs(q.Z() - z) < 1e-9 && std::abs(p.Y() - q.Y()) > 1.0)
      return e;
  }
  return TopoDS_Edge();
}

TopoDS_Edge seed_of(const TopoDS_Shape &body) { return seed_at(body, 0.0, 4.0); }

double volume(const TopoDS_Shape &s) {
  GProp_GProps g;
  BRepGProp::VolumeProperties(s, g);
  return g.Mass();
}

int count(const TopoDS_Shape &s, TopAbs_ShapeEnum t) {
  NCollection_IndexedMap<TopoDS_Shape, TopTools_ShapeMapHasher> m;
  TopExp::MapShapes(s, t, m);
  return m.Extent();
}

kf::OverflowRequest request(const TopoDS_Shape &body, kf::EdgeOpMode mode, double v) {
  kf::OverflowRequest q;
  q.body = body;
  q.seed = seed_of(body);
  q.mode = mode;
  q.value = v;
  q.value2 = v;
  return q;
}

void print_line(const char *label, const kf::OverflowResult &r) {
  std::printf("%-34s ok=%d code=%-34s V=%.9f setback=%.9f faces=%d %s\n", label, r.ok ? 1 : 0,
              r.reason_code.empty() ? "-" : r.reason_code.c_str(),
              r.ok ? volume(r.shape) : 0.0, r.setback, r.ok ? count(r.shape, TopAbs_FACE) : 0,
              r.message.c_str());
}

// Prism of a closed xz polygon along +y (30 mm), forced to a positive solid.
TopoDS_Shape prism_xz(const std::vector<std::pair<double, double>> &pts) {
  BRepBuilderAPI_MakePolygon poly;
  for (const auto &p : pts)
    poly.Add(gp_Pnt(p.first, 0.0, p.second));
  poly.Close();
  const TopoDS_Face face = BRepBuilderAPI_MakeFace(poly.Wire(), true).Face();
  TopoDS_Shape solid = BRepPrimAPI_MakePrism(face, gp_Vec(0.0, 30.0, 0.0)).Shape();
  if (volume(solid) < 0.0)
    solid.Reverse();
  return solid;
}

// Quadrilateral whose edge at (0,4) has interior dihedral `deg` against the 20 mm
// top face; the leaning side face is 4 mm wide.
TopoDS_Shape slanted_body(double deg) {
  const double a = deg * kPi / 180.0;
  const double px = 4.0 * std::cos(a), pz = 4.0 - 4.0 * std::sin(a);
  return prism_xz({{0.0, 4.0}, {20.0, 4.0}, {20.0, pz}, {px, pz}});
}

// Native volume, or a negative number when OCCT refuses. The independent oracle.
double native_volume(const TopoDS_Shape &body, const TopoDS_Edge &seed, bool chamfer, double v) {
  try {
    if (!chamfer) {
      BRepFilletAPI_MakeFillet mk(body);
      mk.Add(v, seed);
      mk.Build();
      return mk.IsDone() ? volume(mk.Shape()) : -1.0;
    }
    NCollection_IndexedDataMap<TopoDS_Shape, NCollection_List<TopoDS_Shape>, TopTools_ShapeMapHasher>
        ef;
    TopExp::MapShapesAndAncestors(body, TopAbs_EDGE, TopAbs_FACE, ef);
    BRepFilletAPI_MakeChamfer mk(body);
    mk.Add(v, v, seed, TopoDS::Face(ef.FindFromKey(seed).First()));
    mk.Build();
    return mk.IsDone() ? volume(mk.Shape()) : -1.0;
  } catch (Standard_Failure &) {
    return -1.0;
  }
}

// Continuity against native OCCT across the threshold where it starts refusing.
void continuity(const char *label, double deg, bool chamfer) {
  const TopoDS_Shape body = slanted_body(deg);
  const TopoDS_Edge seed = seed_of(body);
  double lo = 0.05, hi = 40.0; // lo: native ok, hi: native refuses
  for (int i = 0; i < 60 && hi - lo > 1e-7; ++i) {
    const double mid = 0.5 * (lo + hi);
    (native_volume(body, seed, chamfer, mid) >= 0.0 ? lo : hi) = mid;
  }
  const double th = 0.5 * (lo + hi);
  kf::OverflowRequest q = request(body, chamfer ? kf::EdgeOpMode::Chamfer : kf::EdgeOpMode::Fillet,
                                  th + 1e-4);
  const kf::OverflowResult over = kf::build_overflow(q);
  const double vn = native_volume(body, seed, chamfer, th - 1e-4);
  // The two probes sit 2e-4 apart and V' is ~100 mm^3/mm, so the raw gap is a
  // genuine ~0.02 mm^3 slope, not a discontinuity. Extrapolate the native branch
  // across the gap with its own slope (measured one step further in) and compare.
  const double vn2 = native_volume(body, seed, chamfer, th - 2e-4);
  const double v_extrap = vn + (vn - vn2) * 2.0;
  print_line(label, over);
  std::printf("  %s threshold=%.7f V_native(th-1e-4)=%.9f V_native_extrap=%.9f V_overflow(th+1e-4)=%.9f\n",
              label, th, vn, v_extrap,
              over.ok ? volume(over.shape) : 0.0);
  check(over.ok && vn > 0.0, std::string(label) + ": overflow ok at threshold (" + over.reason_code +
                                 " " + over.message + ")");
  if (over.ok && vn > 0.0)
    check(std::abs(volume(over.shape) - v_extrap) < 5e-3, std::string(label) + ": continuity");
  q.value = q.value2 = th * 1.5;
  const kf::OverflowResult far = kf::build_overflow(q);
  print_line(label, far);
  check(far.ok && count(far.shape, TopAbs_SOLID) == 1,
        std::string(label) + ": ok at 1.5x threshold (" + far.reason_code + " " + far.message + ")");
}

void expect_ok(const char *label, kf::EdgeOpMode mode, double v, double retained) {
  const TopoDS_Shape body = make_box();
  const kf::OverflowRequest q = request(body, mode, v);
  const kf::OverflowResult r = kf::build_overflow(q);
  print_line(label, r);
  check(r.ok, std::string(label) + ": ok (" + r.reason_code + " " + r.message + ")");
  if (!r.ok)
    return;
  const bool fillet = mode == kf::EdgeOpMode::Fillet;
  check(std::abs(volume(r.shape) - retained) < 1e-6, std::string(label) + ": retained volume");
  check(count(r.shape, TopAbs_FACE) == 6, std::string(label) + ": 6 faces");
  check(count(r.shape, TopAbs_SOLID) == 1, std::string(label) + ": 1 solid");
  check(r.kind == (fillet ? kf::OverflowKind::Cliff : kf::OverflowKind::ExtendedPlane),
        std::string(label) + ": kind");
  BRepAdaptor_Surface cons(r.consumed_face);
  check(cons.GetType() == GeomAbs_Plane && std::abs(cons.Plane().Location().X()) < 1e-9 &&
            std::abs(std::abs(cons.Plane().Axis().Direction().X()) - 1.0) < 1e-9,
        std::string(label) + ": consumed face is x=0");
  BRepAdaptor_Surface bl(r.blend_face);
  if (fillet)
    check(bl.GetType() == GeomAbs_Cylinder && std::abs(bl.Cylinder().Radius() - v) < 1e-9,
          std::string(label) + ": cylinder radius");
  else
    check(bl.GetType() == GeomAbs_Plane, std::string(label) + ": planar blend");
  check(std::string(kf::overflow_kind_name(r.kind)) == (fillet ? "cliff" : "extendedPlane"),
        std::string(label) + ": kind name");
  check(!r.tool.IsNull() && !r.tool_blend_face.IsNull() && r.cut != nullptr,
        std::string(label) + ": history carried");
}

void expect_refused(const char *label, const kf::OverflowRequest &q, const char *code) {
  const kf::OverflowResult r = kf::build_overflow(q);
  print_line(label, r);
  check(!r.ok, std::string(label) + ": refused");
  if (code != nullptr)
    check(r.reason_code == code, std::string(label) + ": code " + r.reason_code + " != " + code);
}

void refusals() {
  const TopoDS_Shape box = make_box();
  expect_refused("fillet r=2 not overflowing", request(box, kf::EdgeOpMode::Fillet, 2.0),
                 "OVERFLOW_NOT_OVERFLOWING");
  expect_refused("fillet r=52.5 both consumed", request(box, kf::EdgeOpMode::Fillet, 52.5),
                 "OVERFLOW_BOTH_SUPPORTS_CONSUMED");
  const TopoDS_Shape cube = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
  kf::OverflowRequest cq = request(cube, kf::EdgeOpMode::Fillet, 11.0);
  // Top edge along y at x=0, z=10.
  for (TopExp_Explorer ex(cube, TopAbs_EDGE); ex.More(); ex.Next()) {
    TopoDS_Vertex a, b;
    TopExp::Vertices(TopoDS::Edge(ex.Current()), a, b);
    const gp_Pnt p = BRep_Tool::Pnt(a), q = BRep_Tool::Pnt(b);
    if (std::abs(p.X()) < 1e-9 && std::abs(q.X()) < 1e-9 && std::abs(p.Z() - 10.0) < 1e-9 &&
        std::abs(q.Z() - 10.0) < 1e-9)
      cq.seed = TopoDS::Edge(ex.Current());
  }
  expect_refused("cube r=11 both consumed", cq, "OVERFLOW_BOTH_SUPPORTS_CONSUMED");
  expect_refused("invalid value", request(box, kf::EdgeOpMode::Fillet, 0.0),
                 "OVERFLOW_INVALID_VALUE");
  // d in [w - res, w): a sub-resolution sliver, refused as REMNANT (never snapped).
  expect_refused("chamfer d=4-5e-4 within resolution of width",
                 request(box, kf::EdgeOpMode::Chamfer, 4.0 - 5e-4), "OVERFLOW_REMNANT");
  {
    // Not a single solid: the input audit must refuse before any geometry runs.
    BRep_Builder bb;
    TopoDS_Compound two;
    bb.MakeCompound(two);
    bb.Add(two, box);
    gp_Trsf mv;
    mv.SetTranslation(gp_Vec(100.0, 0.0, 0.0));
    bb.Add(two, BRepBuilderAPI_Transform(box, mv, true).Shape());
    kf::OverflowRequest iq = request(two, kf::EdgeOpMode::Fillet, 11.2);
    iq.seed = request(box, kf::EdgeOpMode::Fillet, 11.2).seed;
    expect_refused("two-solid input refused", iq, "OVERFLOW_INPUT_INVALID");
  }

  // Through-hole along y inside the envelope.
  TopoDS_Shape hole = BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(1.0, -1.0, 2.0), gp_Dir(0, 1, 0)),
                                               0.5, 32.0).Shape();
  const TopoDS_Shape holed = BRepAlgoAPI_Cut(box, hole).Shape();
  expect_refused("through-hole r=6.2", request(holed, kf::EdgeOpMode::Fillet, 6.2),
                 "OVERFLOW_TOOL_TOUCHED_BODY");

  // Rib on the top face: top is no longer a quad.
  const TopoDS_Shape ribbed =
      BRepAlgoAPI_Fuse(box, BRepPrimAPI_MakeBox(gp_Pnt(3, 0, 4), gp_Pnt(4, 30, 5)).Shape()).Shape();
  expect_refused("ribbed top r=6.2", request(ribbed, kf::EdgeOpMode::Fillet, 6.2),
                 "OVERFLOW_NOT_PRISMATIC");

  // 30 deg oblique cap at y=0: tilt a half-space about z so the plane crosses x=0.
  gp_Trsf tilt;
  tilt.SetRotation(gp_Ax1(gp_Pnt(10, 0, 0), gp_Dir(0, 0, 1)), -30.0 * kPi / 180.0);
  const TopoDS_Shape slab = BRepPrimAPI_MakeBox(gp_Pnt(-50, -40, -10), gp_Pnt(50, 0, 20)).Shape();
  const TopoDS_Shape tool = BRepBuilderAPI_Transform(slab, tilt, true).Shape();
  const TopoDS_Shape oblique = BRepAlgoAPI_Cut(box, tool).Shape();
  expect_refused("oblique cap r=6.2", request(oblique, kf::EdgeOpMode::Fillet, 6.2),
                 "OVERFLOW_CAP_NOT_PERPENDICULAR");

  const TopoDS_Shape lshape = prism_xz({{0, 0}, {20, 0}, {20, 4}, {4, 4}, {4, 20}, {0, 20}});
  kf::OverflowRequest lq = request(lshape, kf::EdgeOpMode::Fillet, 6.2);
  lq.seed = seed_at(lshape, 4.0, 4.0);
  expect_refused("concave L inner edge", lq, "OVERFLOW_DIHEDRAL_OUT_OF_RANGE");

  onecad::CancelToken token;
  token.cancel();
  const kf::OverflowResult c = kf::build_overflow(request(box, kf::EdgeOpMode::Fillet, 6.2), &token);
  print_line("cancelled token", c);
  check(c.cancelled && !c.ok && c.reason_code == "OVERFLOW_CANCELLED",
        "cancelled token: cancelled flag + OVERFLOW_CANCELLED");
}

void determinism() {
  const TopoDS_Shape a = make_box(), b = make_box();
  const kf::OverflowResult ra = kf::build_overflow(request(a, kf::EdgeOpMode::Fillet, 11.2));
  const kf::OverflowResult rb = kf::build_overflow(request(b, kf::EdgeOpMode::Fillet, 11.2));
  check(ra.ok && rb.ok, "determinism: both ok");
  if (!(ra.ok && rb.ok))
    return;
  check(count(ra.shape, TopAbs_FACE) == count(rb.shape, TopAbs_FACE), "determinism: faces");
  check(volume(ra.shape) == volume(rb.shape), "determinism: volume bit-identical");
  check(ra.setback == rb.setback, "determinism: setback bit-identical");
}

} // namespace

int main() {
  using M = kf::EdgeOpMode;
  expect_ok("chamfer d=4.0005", M::Chamfer, 4.0005, 2159.94);
  expect_ok("chamfer d=6.2", M::Chamfer, 6.2, 1896.0);
  expect_ok("chamfer d=19", M::Chamfer, 19.0, 360.0);
  expect_ok("fillet r=4.0", M::Fillet, 4.0, 2296.991118431);
  expect_ok("fillet r=4.5", M::Fillet, 4.5, 2273.112217);
  expect_ok("fillet r=6.2", M::Fillet, 6.2, 2209.692130);
  expect_ok("fillet r=11.2", M::Fillet, 11.2, 2085.812877);
  expect_ok("fillet r=30", M::Fillet, 30.0, 1818.269025);
  {
    const TopoDS_Shape body = make_box();
    const kf::OverflowResult r = kf::build_overflow(request(body, M::Fillet, 11.2));
    check(r.ok && std::abs(r.setback - std::sqrt(2.0 * 11.2 * 4.0 - 16.0)) < 1e-9,
          "fillet r=11.2 setback == 8.579044236");
  }
  {
    const TopoDS_Shape body = make_box();
    kf::OverflowRequest q = request(body, M::Chamfer, 5.0);
    q.value2 = 2.0;
    for (TopExp_Explorer ex(body, TopAbs_FACE); ex.More(); ex.Next()) {
      BRepAdaptor_Surface s(TopoDS::Face(ex.Current()));
      if (s.GetType() == GeomAbs_Plane && std::abs(s.Plane().Location().X()) < 1e-9 &&
          std::abs(std::abs(s.Plane().Axis().Direction().X()) - 1.0) < 1e-9)
        q.reference_face = TopoDS::Face(ex.Current());
    }
    const kf::OverflowResult r = kf::build_overflow(q);
    print_line("asymmetric chamfer 5(side)/2(top)", r);
    check(r.ok && std::abs(volume(r.shape) - 2256.0) < 1e-6, "asymmetric chamfer: retained 2256");
    if (r.ok) {
      check(count(r.shape, TopAbs_FACE) == 6, "asymmetric chamfer: 6 faces");
      BRepAdaptor_Surface cons(r.consumed_face);
      check(std::abs(cons.Plane().Location().X()) < 1e-9, "asymmetric chamfer: consumed is x=0");
    }
  }
  continuity("fillet 60deg continuity", 60.0, false);
  continuity("fillet 120deg continuity", 120.0, false);
  continuity("chamfer 60deg continuity", 60.0, true);
  continuity("chamfer 120deg continuity", 120.0, true);
  refusals();
  determinism();
  if (failures != 0) {
    std::fprintf(stderr, "%d failure(s)\n", failures);
    return 1;
  }
  std::printf("test_blend_overflow: all checks passed\n");
  return 0;
}
