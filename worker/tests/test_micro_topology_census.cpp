// test_micro_topology_census.cpp — WP1-G3: does the micro/sliver detector fire
// on geometry that is FINE?
//
// WHY THIS EXISTS. `PublicationPolicy`'s micro-edge and sliver-face bounds are
// about to become enforceable. Before a single one is enabled, the detector has
// to be shown SILENT across the shapes OneCAD actually produces, at every scale
// it supports and both near and far from the origin. The previous definition
// would have failed that test twice over: it counted degenerate edges (so every
// sphere and every cone-apex revolve reported micro edges), and its thresholds
// were ratios to the global bounding-box diagonal (so no real defect at any
// supported scale could reach them).
//
// The census therefore does two things:
//
//  1. NEGATIVE — it walks a fixed catalog x 4 scales x 2 placements, PRINTS the
//     full table (the table is the deliverable; it goes in the gate row), and
//     asserts only that nothing in it is flagged, plus that the sphere and the
//     cone do report their degenerate edges through the separate counter.
//     Only a clean census licenses the next gate to enable the bounds. A dirty
//     one means the DEFINITION is still wrong — fix the definition, not the
//     threshold.
//
//  2. POSITIVE — two constructed defects the detector MUST catch, each paired
//     with the number the OLD ratio metric would have produced for the same
//     shape. That pairing is the load-bearing proof: it shows the redefinition
//     changed the METRIC, not merely the constant.
//
// `deep_audit` is deliberately not used to measure any of this: its output is
// inside kernelbench's normalized digest and `result-v1.schema.json` sets
// `additionalProperties: false` on `microTopology`, so one extra field there
// would move every pinned digest.
//
// No test framework (matches the surrounding style): exit code == failure count.
#include <algorithm>
#include <cmath>
#include <cstdio>
#include <filesystem>
#include <functional>
#include <string>
#include <system_error>
#include <vector>

#include <BRepAlgoAPI_Cut.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepFilletAPI_MakeChamfer.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <BRepGProp.hxx>
#include <BRepOffsetAPI_MakeThickSolid.hxx>
#include <BRepOffset_MakeOffset.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCone.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakeSphere.hxx>
#include <BRepPrimAPI_MakeTorus.hxx>
#include <BRep_Builder.hxx>
#include <BRep_Tool.hxx>
#include <GProp_GProps.hxx>
#include <Interface_Static.hxx>
#include <Standard_Failure.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Compound.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Vertex.hxx>
#include <gp_Ax2.hxx>
#include <gp_Dir.hxx>
#include <gp_Pln.hxx>
#include <gp_Pnt.hxx>
#include <gp_Trsf.hxx>

#include "io/StepRead.h"
#include "kernel/validation/GeometryPrecision.h"
#include "kernel/validation/ShapeAudit.h"
#include "ops/gear/GearTool.h"
#include "step_fixture_util.h"

namespace validation = onecad::kernel::validation;

namespace {

int g_failures = 0;

void check(bool condition, const std::string &message) {
  if (condition)
    return;
  std::fprintf(stderr, "FAIL: %s\n", message.c_str());
  ++g_failures;
}

// --- catalog builders (origin, characteristic dimension `s` mm) --------------

TopoDS_Shape make_box(double s) { return BRepPrimAPI_MakeBox(s, s, s).Shape(); }

TopoDS_Shape make_cylinder(double s) {
  return BRepPrimAPI_MakeCylinder(0.5 * s, s).Shape();
}

TopoDS_Shape make_sphere(double s) { return BRepPrimAPI_MakeSphere(0.5 * s).Shape(); }

// r2 == 0 puts a DEGENERATE apex edge on the shape — the exact topology the old
// detector mistook for a micro edge.
TopoDS_Shape make_cone(double s) {
  return BRepPrimAPI_MakeCone(0.5 * s, 0.0, s).Shape();
}

TopoDS_Shape make_torus(double s) {
  return BRepPrimAPI_MakeTorus(0.5 * s, 0.15 * s).Shape();
}

TopoDS_Shape make_filleted_box(double s) {
  const TopoDS_Shape box = make_box(s);
  BRepFilletAPI_MakeFillet fillet(box);
  for (TopExp_Explorer it(box, TopAbs_EDGE); it.More(); it.Next())
    fillet.Add(0.1 * s, TopoDS::Edge(it.Current()));
  fillet.Build();
  return fillet.IsDone() ? fillet.Shape() : TopoDS_Shape();
}

// The four vertical edges only: a box chamfered on all twelve produces corner
// cases whose failure would be a chamfer story, not a census one.
TopoDS_Shape make_chamfered_box(double s) {
  const TopoDS_Shape box = make_box(s);
  TopTools_IndexedDataMapOfShapeListOfShape edge_faces;
  TopExp::MapShapesAndAncestors(box, TopAbs_EDGE, TopAbs_FACE, edge_faces);
  BRepFilletAPI_MakeChamfer chamfer(box);
  for (int i = 1; i <= edge_faces.Extent(); ++i) {
    const TopoDS_Edge &edge = TopoDS::Edge(edge_faces.FindKey(i));
    TopoDS_Vertex first, last;
    TopExp::Vertices(edge, first, last, Standard_True);
    if (first.IsNull() || last.IsNull() || edge_faces(i).IsEmpty())
      continue;
    if (std::abs(BRep_Tool::Pnt(first).Z() - BRep_Tool::Pnt(last).Z()) < 0.5 * s)
      continue;  // not a vertical edge
    chamfer.Add(0.1 * s, 0.1 * s, edge, TopoDS::Face(edge_faces(i).First()));
  }
  chamfer.Build();
  return chamfer.IsDone() ? chamfer.Shape() : TopoDS_Shape();
}

TopoDS_Shape make_shelled_box(double s) {
  const TopoDS_Shape box = make_box(s);
  TopTools_ListOfShape removed;
  for (TopExp_Explorer it(box, TopAbs_FACE); it.More(); it.Next()) {
    GProp_GProps props;
    BRepGProp::SurfaceProperties(it.Current(), props);
    if (std::abs(props.CentreOfMass().Z() - s) < 1.0e-6 * s) {
      removed.Append(it.Current());
      break;
    }
  }
  BRepOffsetAPI_MakeThickSolid builder;
  // Scale-relative build tolerance: production's fixed 1e-3 mm is larger than
  // the whole part at the 0.01 mm end of the supported range.
  builder.MakeThickSolidByJoin(box, removed, -0.1 * s, 1.0e-4 * s, BRepOffset_Skin,
                               Standard_False, Standard_False, GeomAbs_Arc, Standard_False);
  builder.Build();
  return builder.IsDone() ? builder.Shape() : TopoDS_Shape();
}

TopoDS_Shape make_gear(double s) {
  onecad::ops::gear::GearBuildSpec spec;
  spec.involute.module = 0.1 * s;
  spec.involute.numTeeth = 12;
  spec.sampleCount = 12;
  // CLAMPED to the SCHEMA §7.3 height bound (`≤ 1000` mm, kernel-hardening
  // WP-I): at the top scale `0.5 * s` would be 5000 mm, which `build_gear_solid`
  // now refuses BY NAME as out of range. The clamp keeps this row a SCALE
  // measurement — the tooth profile, which is what the micro-edge and
  // sliver-face metrics read, still scales with `s`.
  spec.height = std::min(0.5 * s, 1000.0);
  const onecad::ops::gear::GearBuildResult result =
      onecad::ops::gear::build_gear_solid(spec, gp_Ax2(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)));
  return result.ok ? result.shape : TopoDS_Shape();
}

TopoDS_Shape make_holed_plate(double s) {
  const TopoDS_Shape plate = BRepPrimAPI_MakeBox(s, s, 0.2 * s).Shape();
  const TopoDS_Shape drill =
      BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(0.5 * s, 0.5 * s, -0.1 * s), gp_Dir(0, 0, 1)),
                               0.2 * s, 0.4 * s)
          .Shape();
  BRepAlgoAPI_Cut cut(plate, drill);
  cut.Build();
  return cut.IsDone() ? cut.Shape() : TopoDS_Shape();
}

TopoDS_Shape make_offset_face_box(double s) {
  const TopoDS_Shape box = make_box(s);
  TopoDS_Face top;
  for (TopExp_Explorer it(box, TopAbs_FACE); it.More(); it.Next()) {
    GProp_GProps props;
    BRepGProp::SurfaceProperties(it.Current(), props);
    if (std::abs(props.CentreOfMass().Z() - s) < 1.0e-6 * s) {
      top = TopoDS::Face(it.Current());
      break;
    }
  }
  if (top.IsNull())
    return TopoDS_Shape();
  BRepOffset_MakeOffset mo;
  mo.Initialize(box, 0.0, 1.0e-4 * s, BRepOffset_Skin, Standard_False, Standard_False,
                GeomAbs_Intersection, Standard_False, Standard_False);
  mo.AllowLinearization(Standard_False);
  mo.SetOffsetOnFace(top, 0.2 * s);
  mo.MakeOffsetShape();
  return mo.IsDone() ? mo.Shape() : TopoDS_Shape();
}

// Raise every edge and vertex tolerance to `tol` mm. `UpdateEdge`/`UpdateVertex`
// take the maximum, so this only ever loosens. Vertices are raised alongside the
// edges because a vertex tighter than its edge is not valid B-Rep, and an
// invalid shape would make the row a validity story instead of a tolerance one.
void inflate_tolerance(const TopoDS_Shape &shape, double tol) {
  BRep_Builder builder;
  for (TopExp_Explorer it(shape, TopAbs_EDGE); it.More(); it.Next())
    builder.UpdateEdge(TopoDS::Edge(it.Current()), tol);
  for (TopExp_Explorer it(shape, TopAbs_VERTEX); it.More(); it.Next())
    builder.UpdateVertex(TopoDS::Vertex(it.Current()), tol);
}

// The IMPORTED-GEOMETRY case for the tolerance-relative half of the micro rule.
// `StepReadPolicy::max_precision_val` is 1.0 mm, so a body arriving from a STEP
// file may legally carry edge tolerances orders above `Precision::Confusion()` —
// and the micro limit is `2*max(tol, micro_floor)`, which such a body drags up
// with it. 1e-3 mm is a realistic imported tolerance and puts the limit at
// 2e-3 mm; the box's own edges are 0.01 mm even at the smallest census scale, so
// a clean import must still report zero.
TopoDS_Shape make_loose_tolerance_box(double s) {
  const TopoDS_Shape box = make_box(s);
  inflate_tolerance(box, 1.0e-3);
  return box;
}

// Planar-planar cut: a corner notch. Distinct from the holed plate, whose cut is
// planar-against-cylindrical.
TopoDS_Shape make_boolean_cut(double s) {
  const TopoDS_Shape box = make_box(s);
  const TopoDS_Shape notch =
      BRepPrimAPI_MakeBox(gp_Pnt(0.6 * s, 0.6 * s, 0.6 * s), s, s, s).Shape();
  BRepAlgoAPI_Cut cut(box, notch);
  cut.Build();
  return cut.IsDone() ? cut.Shape() : TopoDS_Shape();
}

// --- placement --------------------------------------------------------------

// Baked, not a `TopLoc_Location`: a location leaves the underlying arithmetic at
// the origin, which is the easy case. Baking the translation into the geometry
// is what an imported far-from-origin model actually looks like.
TopoDS_Shape place(const TopoDS_Shape &shape, double offset) {
  if (shape.IsNull() || offset == 0.0)
    return shape;
  gp_Trsf trsf;
  trsf.SetTranslation(gp_Vec(offset, 0.0, 0.0));
  BRepBuilderAPI_Transform transform(shape, trsf, Standard_True);
  return transform.IsDone() ? transform.Shape() : TopoDS_Shape();
}

// --- the OLD metric, computed locally ---------------------------------------
//
// Reimplemented here rather than left behind in production as dead code, so the
// comparison the positive probes rest on is explicit and cannot rot into
// something the shipped detector no longer resembles.
struct LegacyCounts {
  int micro_edges = 0;
  int sliver_faces = 0;
  double minimum_edge_ratio = 0.0;
  double minimum_face_ratio = 0.0;
};

LegacyCounts legacy_counts(const TopoDS_Shape &shape) {
  LegacyCounts out;
  const validation::GeometryPrecisionContext context = validation::precision_of(shape);
  const double diagonal = context.scale_diagonal;
  if (diagonal <= 0.0)
    return out;
  out.minimum_edge_ratio = 1.0;
  out.minimum_face_ratio = 1.0;
  for (TopExp_Explorer it(shape, TopAbs_EDGE); it.More(); it.Next()) {
    GProp_GProps props;
    BRepGProp::LinearProperties(it.Current(), props);
    const double ratio = props.Mass() / diagonal;
    out.minimum_edge_ratio = std::min(out.minimum_edge_ratio, ratio);
    out.micro_edges += ratio < 1.0e-9 ? 1 : 0;
  }
  for (TopExp_Explorer it(shape, TopAbs_FACE); it.More(); it.Next()) {
    GProp_GProps props;
    BRepGProp::SurfaceProperties(it.Current(), props);
    const double ratio = props.Mass() / (diagonal * diagonal);
    out.minimum_face_ratio = std::min(out.minimum_face_ratio, ratio);
    out.sliver_faces += ratio < 1.0e-12 ? 1 : 0;
  }
  return out;
}

// --- STEP round trip --------------------------------------------------------

// Cheap and offline: the fixture is GENERATED by the same writer sequence
// `io/ExportStep.cpp` uses, so nothing binary is tracked. Returns a null shape
// when the round trip could not be performed, which the caller reports.
//
// `uncertainty` > 0 additionally writes a DECLARED uncertainty into the file.
// OCCT's `write.precision.mode` is -1/0/1 = least/average/greatest tolerance of
// the shape and **2 = use `write.precision.val`**; only 2 lets a caller state an
// uncertainty the geometry does not already carry.
// `StepReadPolicy::precision_mode` is 0, i.e. the
// read honours whatever the file declares, so this is the only lane in the tree
// that can put a loose tolerance on a body the way a foreign CAD system does.
// The knobs are saved and restored: `Interface_Static` is process-global and a
// leaked value would bias every later row.
TopoDS_Shape step_roundtrip(const TopoDS_Shape &shape, const std::string &tag,
                            double uncertainty) {
  if (shape.IsNull())
    return shape;
  std::error_code ec;
  const std::filesystem::path path =
      std::filesystem::temp_directory_path(ec) / ("onecad_census_" + tag + ".step");
  if (ec)
    return TopoDS_Shape();

  const int saved_mode = Interface_Static::IVal("write.precision.mode");
  const double saved_val = Interface_Static::RVal("write.precision.val");
  if (uncertainty > 0.0) {
    Interface_Static::SetIVal("write.precision.mode", 2);
    Interface_Static::SetRVal("write.precision.val", uncertainty);
  }
  const std::string written = stepfx::write_step_fixture(shape, path.string());
  Interface_Static::SetIVal("write.precision.mode", saved_mode);
  Interface_Static::SetRVal("write.precision.val", saved_val);
  if (!written.empty())
    return TopoDS_Shape();

  const onecad::io::StepReadResult read = onecad::io::read_step(path.string());
  std::filesystem::remove(path, ec);
  if (!read.ok() || read.solids.empty())
    return TopoDS_Shape();
  return read.solids.front();
}

// The limit the micro rule actually applies, evaluated at the shape's LOOSEST
// edge tolerance: `kMicroFactor * max(tol, micro_edge_length()/kMicroFactor)`.
// `micro_edge_length()` alone is only the floor half of that maximum, so on an
// imported body it understates the bar by orders of magnitude.
double effective_micro_limit(const validation::GeometryPrecisionContext &context,
                             double edge_tolerance) {
  return validation::kMicroFactor *
         std::max(edge_tolerance, context.micro_edge_length() / validation::kMicroFactor);
}

// --- the census -------------------------------------------------------------

struct CatalogEntry {
  const char *name;
  std::function<TopoDS_Shape(double)> build;
  bool via_step = false;
  // > 0 writes a DECLARED uncertainty into the STEP file (`via_step` only).
  double step_uncertainty = 0.0;
};

void run_census() {
  const std::vector<CatalogEntry> catalog = {
      {"box", make_box},
      {"cylinder", make_cylinder},
      {"sphere", make_sphere},
      {"cone", make_cone},
      {"torus", make_torus},
      {"filletedBox", make_filleted_box},
      {"chamferedBox", make_chamfered_box},
      {"shelledBox", make_shelled_box},
      {"gear", make_gear},
      {"holedPlate", make_holed_plate},
      {"offsetFaceBox", make_offset_face_box},
      {"booleanCut", make_boolean_cut},
      {"looseTolBox", make_loose_tolerance_box},
      {"stepImportedBox", make_box, true},
      {"stepLooseBox", make_box, true, 1.0e-3},
  };
  const double scales[] = {0.01, 1.0, 100.0, 10000.0};
  const double placements[] = {0.0, 1.0e6};

  // `edgeTol` and `microLimit` are printed side by side on purpose: the micro
  // rule is `length <= 2*max(edgeTol, microFloor)`, so a row where edgeTol
  // dominates is a row where the LOCAL tolerance, not the global scale, is
  // setting the bar. That is the half of the rule the imported-geometry cases
  // exist to exercise.
  std::printf("%-16s %10s %10s %6s %7s %6s %14s %14s %11s %11s\n", "shape", "scale",
              "placeX", "micro", "sliver", "degen", "minEdgeLen", "minFaceWidth", "edgeTol",
              "microLimit");
  for (const CatalogEntry &entry : catalog) {
    for (double scale : scales) {
      for (double offset : placements) {
        TopoDS_Shape shape;
        try {
          shape = place(entry.build(scale), offset);
          if (entry.via_step) {
            char tag[64];
            std::snprintf(tag, sizeof(tag), "%s_%g_%g", entry.name, scale, offset);
            shape = step_roundtrip(shape, tag, entry.step_uncertainty);
          }
        } catch (const Standard_Failure &f) {
          shape = TopoDS_Shape();
          std::fprintf(stderr, "  (%s @ %g/%g raised: %s)\n", entry.name, scale, offset,
                       f.GetMessageString() ? f.GetMessageString() : "OCCT");
        }
        const std::string label =
            std::string(entry.name) + " @ scale " + std::to_string(scale) + " place " +
            std::to_string(offset);
        check(!shape.IsNull(), "census: " + label + " builds");
        if (shape.IsNull()) {
          std::printf("%-16s %10g %10g %6s %7s %6s %14s %14s %11s %11s\n", entry.name,
                      scale, offset, "-", "-", "-", "BUILD", "FAILED", "-", "-");
          continue;
        }
        const validation::ShapeEvidence evidence =
            validation::collect_shape_evidence(shape, validation::PublicationTier::TierB);
        const validation::GeometryPrecisionContext context = validation::precision_of(shape);
        std::printf("%-16s %10g %10g %6d %7d %6d %14.6g %14.6g %11.4g %11.4g\n",
                    entry.name, scale, offset, evidence.micro_edge_count,
                    evidence.sliver_face_count, evidence.degenerate_edge_count,
                    evidence.minimum_edge_length, evidence.minimum_face_width,
                    evidence.tolerances.edge,
                    effective_micro_limit(context, evidence.tolerances.edge));

        check(evidence.micro_topology_checked, "census: " + label + " reports micro evidence");
        check(evidence.micro_edge_count == 0,
              "census: " + label + " reports no micro edges, got " +
                  std::to_string(evidence.micro_edge_count));
        check(evidence.sliver_face_count == 0,
              "census: " + label + " reports no sliver faces, got " +
                  std::to_string(evidence.sliver_face_count));
        if (std::string(entry.name) == "stepLooseBox") {
          // CHARACTERIZATION, not a requirement. The written file really does
          // declare 1e-3 mm (`UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-03)`
          // — read back out of the fixture while developing this row), yet the
          // imported solid arrives at OCCT's Confusion: an analytic box
          // round-trips exactly, so nothing forces its B-Rep tolerance up.
          // Pinned because if OCCT ever starts propagating the declared
          // uncertainty, the micro limit for EVERY import moves four orders with
          // it, and this census must be re-read before any bound is trusted.
          check(evidence.tolerances.edge < 1.0e-6,
                "census: " + label +
                    " imports at Confusion despite a declared 1e-3 mm uncertainty, got "
                    "edgeTol " + std::to_string(evidence.tolerances.edge));
        }
        const bool has_apex =
            std::string(entry.name) == "sphere" || std::string(entry.name) == "cone";
        if (has_apex) {
          check(evidence.degenerate_edge_count > 0,
                "census: " + label + " reports its degenerate seam/apex edges, got " +
                    std::to_string(evidence.degenerate_edge_count));
        }
      }
    }
  }
  std::fflush(stdout);
}

// --- positive probes --------------------------------------------------------

// A 1.5e-7 mm edge sitting on a 10 mm box: shorter than `micro_edge_length()`
// and therefore indistinguishable from a point, yet its ratio to the compound's
// 17 mm diagonal is 8.7e-9 — an order ABOVE the old 1e-9 cut, so the previous
// detector was blind to it.
void test_micro_edge_probe() {
  const TopoDS_Shape box = make_box(10.0);
  BRepBuilderAPI_MakeEdge tiny(gp_Pnt(0.0, 0.0, 0.0), gp_Pnt(1.5e-7, 0.0, 0.0));
  check(tiny.IsDone(), "micro-edge probe: the 1.5e-7 mm edge builds");
  if (!tiny.IsDone())
    return;
  BRep_Builder builder;
  TopoDS_Compound compound;
  builder.MakeCompound(compound);
  builder.Add(compound, box);
  builder.Add(compound, tiny.Edge());

  const validation::GeometryPrecisionContext context = validation::precision_of(compound);
  const validation::ShapeEvidence evidence =
      validation::collect_shape_evidence(compound, validation::PublicationTier::TierB);
  const LegacyCounts legacy = legacy_counts(compound);
  std::printf("micro-edge probe: len=1.5e-07 limit=%.6g micro=%d legacyMicro=%d "
              "legacyRatio=%.6g\n",
              context.micro_edge_length(), evidence.micro_edge_count, legacy.micro_edges,
              legacy.minimum_edge_ratio);

  check(1.5e-7 <= context.micro_edge_length(),
        "micro-edge probe: the probe edge really is below the micro limit");
  check(evidence.micro_edge_count == 1,
        "micro-edge probe: detected exactly one micro edge, got " +
            std::to_string(evidence.micro_edge_count));
  check(evidence.degenerate_edge_count == 0,
        "micro-edge probe: nothing here is degenerate");
  check(legacy.micro_edges == 0,
        "micro-edge probe: the ratio-to-diagonal metric MISSES it, got " +
            std::to_string(legacy.micro_edges));

  // Same edge with an ordinary length is not micro — the limit is doing the
  // work, not the mere presence of a loose edge in a compound.
  BRepBuilderAPI_MakeEdge ordinary(gp_Pnt(0.0, 0.0, 0.0), gp_Pnt(1.0, 0.0, 0.0));
  TopoDS_Compound control;
  builder.MakeCompound(control);
  builder.Add(control, box);
  builder.Add(control, ordinary.Edge());
  check(validation::collect_shape_evidence(control, validation::PublicationTier::TierB)
                .micro_edge_count == 0,
        "micro-edge probe: a 1 mm edge on the same box is not micro");
}

// THE TOLERANCE-RELATIVE HALF OF THE RULE, in both directions.
//
// The limit is `2*max(tol(edge), micro_floor)`. Without this pair the `tol(edge)`
// term has no coverage at all: every other shape in this file carries OCCT's
// default 1e-7 tolerance, where the floor dominates and the term could be
// deleted with the suite still green. That matters because an imported body may
// legally carry edge tolerance up to `StepReadPolicy::max_precision_val` = 1.0 mm,
// which would put the micro limit at 2.0 mm — a bar ordinary features can reach.
//
// Positive: a 1.5e-3 mm edge whose own tolerance is 1e-3 mm. Its limit is 2e-3,
// so it is micro — the edge is shorter than its own uncertainty and is not
// distinguishable from a point.
// Negative control: the SAME 1.5e-3 mm edge at default tolerance. Its limit is
// 2e-7, so it is an ordinary small edge and must not be flagged.
void test_tolerance_local_micro_probe() {
  constexpr double kEdgeLength = 1.5e-3;
  constexpr double kLooseTolerance = 1.0e-3;

  const auto build = [](double tolerance) {
    BRep_Builder builder;
    TopoDS_Compound compound;
    builder.MakeCompound(compound);
    builder.Add(compound, make_box(10.0));
    BRepBuilderAPI_MakeEdge edge(gp_Pnt(0.0, 0.0, 0.0), gp_Pnt(kEdgeLength, 0.0, 0.0));
    TopoDS_Shape shaped = edge.Edge();
    if (tolerance > 0.0)
      inflate_tolerance(shaped, tolerance);
    builder.Add(compound, shaped);
    return compound;
  };

  const TopoDS_Shape loose = build(kLooseTolerance);
  const TopoDS_Shape tight = build(0.0);
  const validation::ShapeEvidence loose_evidence =
      validation::collect_shape_evidence(loose, validation::PublicationTier::TierB);
  const validation::ShapeEvidence tight_evidence =
      validation::collect_shape_evidence(tight, validation::PublicationTier::TierB);
  const validation::GeometryPrecisionContext context = validation::precision_of(loose);
  std::printf("tolerance-local probe: len=%.6g looseTol=%.6g looseLimit=%.6g looseMicro=%d "
              "tightLimit=%.6g tightMicro=%d\n",
              kEdgeLength, kLooseTolerance,
              effective_micro_limit(context, kLooseTolerance), loose_evidence.micro_edge_count,
              effective_micro_limit(context, 0.0), tight_evidence.micro_edge_count);

  check(kEdgeLength <= effective_micro_limit(context, kLooseTolerance),
        "tolerance-local probe: the loose edge really is inside its own tolerance limit");
  check(kEdgeLength > effective_micro_limit(context, 0.0),
        "tolerance-local probe: the same edge is OUTSIDE the floor-only limit, so the "
        "tolerance term is the only thing that can flag it");
  check(loose_evidence.micro_edge_count == 1,
        "tolerance-local probe: the loose-tolerance edge is micro, got " +
            std::to_string(loose_evidence.micro_edge_count));
  check(tight_evidence.micro_edge_count == 0,
        "tolerance-local probe: the same edge at default tolerance is not micro, got " +
            std::to_string(tight_evidence.micro_edge_count));
}

// A 1000 mm x 1e-7 mm plate. Minimum width `2*area/perimeter` is 1e-7 mm — a
// face that cannot be told from its own boundary. Its AREA, however, is 1e-4 mm2
// against a 1e6 mm2 diagonal square, i.e. a ratio of 1e-10: two orders above the
// old 1e-12 cut. This is the case the width metric exists for.
void test_sliver_face_probe() {
  constexpr double kLength = 1000.0;
  constexpr double kWidth = 1.0e-7;
  BRepBuilderAPI_MakeFace maker(gp_Pln(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)), 0.0, kLength, 0.0,
                                kWidth);
  check(maker.IsDone(), "sliver probe: the 1000 x 1e-7 mm face builds");
  if (!maker.IsDone())
    return;
  const TopoDS_Shape sliver = maker.Face();

  const validation::GeometryPrecisionContext context = validation::precision_of(sliver);
  const validation::ShapeEvidence evidence =
      validation::collect_shape_evidence(sliver, validation::PublicationTier::TierB);
  const LegacyCounts legacy = legacy_counts(sliver);
  std::printf("sliver probe: width=%.6g limit=%.6g sliver=%d legacySliver=%d "
              "legacyAreaRatio=%.6g\n",
              evidence.minimum_face_width, context.sliver_face_width(),
              evidence.sliver_face_count, legacy.sliver_faces, legacy.minimum_face_ratio);

  check(evidence.minimum_face_width <= context.sliver_face_width(),
        "sliver probe: measured width is at or below the sliver limit");
  check(evidence.sliver_face_count == 1,
        "sliver probe: detected exactly one sliver face, got " +
            std::to_string(evidence.sliver_face_count));
  check(legacy.sliver_faces == 0,
        "sliver probe: the area-to-diagonal-squared metric MISSES it, got " +
            std::to_string(legacy.sliver_faces));
  check(legacy.minimum_face_ratio >= 1.0e-12,
        "sliver probe: the legacy area ratio is above its own cut, so the miss is the "
        "METRIC and not the threshold");

  // A square plate of the same area is not a sliver: area alone decides nothing.
  BRepBuilderAPI_MakeFace square(gp_Pln(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)), 0.0, 0.01, 0.0, 0.01);
  check(validation::collect_shape_evidence(square.Face(), validation::PublicationTier::TierB)
                .sliver_face_count == 0,
        "sliver probe: a 0.01 x 0.01 mm square of the same area is not a sliver");
}

// WP1-G3 moved micro-topology collection out of the Tier B branch so a Tier A
// policy carrying a bound has evidence to judge. Tier A is the INTERACTIVE
// lane (every preview downgrades to it), so the cost of that move is reported
// rather than asserted to be free: the numbers below are what a reviewer needs
// to decide whether it stays there.
void report_tier_a_cost() {
  const TopoDS_Shape heavy = make_gear(100.0);
  if (heavy.IsNull())
    return;
  TopTools_IndexedMapOfShape faces;
  TopTools_IndexedMapOfShape edges;
  TopExp::MapShapes(heavy, TopAbs_FACE, faces);
  TopExp::MapShapes(heavy, TopAbs_EDGE, edges);
  const validation::ShapeEvidence fast =
      validation::collect_shape_evidence(heavy, validation::PublicationTier::TierA);
  const validation::ShapeEvidence deep =
      validation::collect_shape_evidence(heavy, validation::PublicationTier::TierB);
  std::printf("tierA cost (gear @100mm, %d faces / %d edges): tierA=%.3f ms tierB=%.3f ms\n",
              faces.Extent(), edges.Extent(), fast.validator_duration_ms,
              deep.validator_duration_ms);
  check(fast.micro_topology_checked,
        "tier A carries micro-topology evidence, so a Tier A bound can be judged");
  check(!fast.manifold_checked && !fast.self_interference_checked,
        "tier A still skips the BOP-backed evidence");
}

}  // namespace

int main() {
  run_census();
  test_micro_edge_probe();
  test_tolerance_local_micro_probe();
  test_sliver_face_probe();
  report_tier_a_cost();
  if (g_failures == 0)
    std::fprintf(stderr, "test_micro_topology_census: all checks passed\n");
  return g_failures > 125 ? 125 : g_failures;
}
