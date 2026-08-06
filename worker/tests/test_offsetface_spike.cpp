// test_offsetface_spike.cpp — OFFSET-FACE Phase 0: OCCT selective-offset go/no-go.
// Pure OCCT (no ops infra): characterizes BRepOffset_MakeOffset with per-face
// SetOffsetOnFace (Skin, Intersection=false, Join=GeomAbs_Intersection) before
// any OffsetFace schema/op work is frozen. Covers: box/frustum/cylinder happy
// paths, the hole σ-convention, the split-cylinder tangent go/no-go, the
// Generated-history pattern the element-map adapter depends on, and the known
// traps (negative radius inside-out, zero-volume collapse, foreign face).
// No framework: exit code == failure count.
#include <cmath>
#include <cstdio>
#include <string>

#include <BRepAdaptor_Surface.hxx>
#include <BRepAlgo_Image.hxx>
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepGProp.hxx>
#include <BRepOffset_MakeOffset.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakeWedge.hxx>
#include <GProp_GProps.hxx>
#include <Precision.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Ax2.hxx>
#include <gp_Dir.hxx>
#include <gp_Pnt.hxx>

namespace {

int g_failures = 0;
void check(bool cond, const std::string& msg) {
    if (!cond) { std::fprintf(stderr, "FAIL: %s\n", msg.c_str()); ++g_failures; }
}
void check_near(double got, double want, double tol, const std::string& msg) {
    if (std::abs(got - want) > tol) {
        std::fprintf(stderr, "FAIL: %s (got %.4f want %.4f)\n", msg.c_str(), got, want);
        ++g_failures;
    }
}

double volume_of(const TopoDS_Shape& s) {
    GProp_GProps props;
    BRepGProp::VolumeProperties(s, props);
    return props.Mass();
}

int count_of(const TopoDS_Shape& s, TopAbs_ShapeEnum kind) {
    TopTools_IndexedMapOfShape map;
    TopExp::MapShapes(s, kind, map);
    return map.Extent();
}

// Face whose barycenter is nearest (cx,cy,cz) — mirrors test_wp6_extrude helper
// but self-contained (surface-props barycenter, no element-map dependency).
TopoDS_Face face_by_center(const TopoDS_Shape& shape, double cx, double cy, double cz) {
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(shape, TopAbs_FACE, faces);
    TopoDS_Face best;
    double best_d2 = -1.0;
    for (int i = 1; i <= faces.Extent(); ++i) {
        GProp_GProps props;
        BRepGProp::SurfaceProperties(faces(i), props);
        const gp_Pnt c = props.CentreOfMass();
        const double dx = c.X() - cx, dy = c.Y() - cy, dz = c.Z() - cz;
        const double d2 = dx * dx + dy * dy + dz * dz;
        if (best_d2 < 0.0 || d2 < best_d2) { best_d2 = d2; best = TopoDS::Face(faces(i)); }
    }
    return best;
}

// All cylindrical faces of `shape` (used for the lateral faces of cylinders/holes).
TopTools_ListOfShape cylindrical_faces(const TopoDS_Shape& shape) {
    TopTools_ListOfShape out;
    for (TopExp_Explorer ex(shape, TopAbs_FACE); ex.More(); ex.Next()) {
        BRepAdaptor_Surface surf(TopoDS::Face(ex.Current()));
        if (surf.GetType() == GeomAbs_Cylinder) out.Append(ex.Current());
    }
    return out;
}

constexpr double kTol = 1.0e-4;  // spike-only; the op will derive its own named constant

// The exact call sequence the plan freezes (Skin, boolean Intersection=false,
// Join=Intersection, Thickening=false, RemoveIntEdges=false, no linearization).
struct OffsetRun {
    BRepOffset_MakeOffset mo;
    bool done = false;
    TopoDS_Shape result;
    explicit OffsetRun(const TopoDS_Shape& body) {
        mo.Initialize(body, 0.0, kTol, BRepOffset_Skin,
                      /*Intersection*/ Standard_False, /*SelfInter*/ Standard_False,
                      GeomAbs_Intersection, /*Thickening*/ Standard_False,
                      /*RemoveIntEdges*/ Standard_False);
        mo.AllowLinearization(Standard_False);
    }
    void offset_face(const TopoDS_Face& f, double d) { mo.SetOffsetOnFace(f, d); }
    bool run() {
        mo.MakeOffsetShape();
        done = mo.IsDone();
        if (done) result = mo.Shape();
        return done;
    }
    bool valid() const {
        if (!done) return false;
        BRepCheck_Analyzer an(result);
        return an.IsValid() == Standard_True;
    }
};

// ---------------------------------------------------------------------------
// 1. Box top +2: sharp extension, volume 10*10*12, face/edge counts preserved.
void test_box_top_plus2() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
    OffsetRun run(box);
    run.offset_face(face_by_center(box, 5, 5, 10), 2.0);
    check(run.run(), "box+2: IsDone");
    check(run.valid(), "box+2: BRepCheck valid");
    check_near(volume_of(run.result), 1200.0, 1e-6, "box+2: volume 1200");
    check(count_of(run.result, TopAbs_FACE) == 6, "box+2: still 6 faces");
    check(count_of(run.result, TopAbs_SOLID) == 1, "box+2: single solid");
}

// 2. Box top -2 (shrink): volume 800 — signed semantics.
void test_box_top_minus2() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
    OffsetRun run(box);
    run.offset_face(face_by_center(box, 5, 5, 10), -2.0);
    check(run.run(), "box-2: IsDone");
    check(run.valid(), "box-2: BRepCheck valid");
    check_near(volume_of(run.result), 800.0, 1e-6, "box-2: volume 800");
}

// 3. Frustum top +2 must PRESERVE the taper. 7-arg MakeWedge(dx,dy,dz,
//    xmin,zmin,xmax,zmax) tapers along Y in BOTH x and z: base 10×10 at y=0,
//    top 6×6 at y=10 ⇒ cross-section (10−0.4y)², V(Y)=[1000−(10−0.4Y)³]/1.2.
//    Base 653.33; taper-preserving extension to y=12 gives 716.16; a
//    prism-style push/pull would give 725.33 (base + 36·2). This is the
//    Shapr3D semantic the plan rejects BRepFeat_MakePrism over.
void test_frustum_taper_preserved() {
    const TopoDS_Shape frustum =
        BRepPrimAPI_MakeWedge(10.0, 10.0, 10.0, 2.0, 2.0, 8.0, 8.0).Shape();
    check_near(volume_of(frustum), 653.3333, 1e-3, "frustum: base volume");
    // The tapered top is the face at max Y.
    TopoDS_Face top;
    double best_y = -1e9;
    for (TopExp_Explorer ex(frustum, TopAbs_FACE); ex.More(); ex.Next()) {
        GProp_GProps props;
        BRepGProp::SurfaceProperties(ex.Current(), props);
        if (props.CentreOfMass().Y() > best_y) {
            best_y = props.CentreOfMass().Y();
            top = TopoDS::Face(ex.Current());
        }
    }
    OffsetRun run(frustum);
    run.offset_face(top, 2.0);
    check(run.run(), "frustum+2: IsDone");
    check(run.valid(), "frustum+2: BRepCheck valid");
    check_near(volume_of(run.result), 716.16, 0.5, "frustum+2: taper preserved (716.16, not 725.33)");
    check(count_of(run.result, TopAbs_FACE) == count_of(frustum, TopAbs_FACE),
          "frustum+2: face count preserved");
}

// 4. Full cylinder lateral +2 → coaxial radius change 10→12.
void test_cylinder_radius_grow() {
    const TopoDS_Shape cyl = BRepPrimAPI_MakeCylinder(10.0, 20.0).Shape();
    TopTools_ListOfShape lats = cylindrical_faces(cyl);
    check(lats.Extent() == 1, "cyl: one lateral face");
    OffsetRun run(cyl);
    run.offset_face(TopoDS::Face(lats.First()), 2.0);
    check(run.run(), "cyl+2: IsDone");
    check(run.valid(), "cyl+2: BRepCheck valid");
    check_near(volume_of(run.result), M_PI * 144.0 * 20.0, 1e-3, "cyl+2: volume πr'²h, r'=12");
    TopTools_ListOfShape out_lats = cylindrical_faces(run.result);
    check(out_lats.Extent() == 1, "cyl+2: one lateral face in result");
    if (out_lats.Extent() == 1) {
        BRepAdaptor_Surface surf(TopoDS::Face(out_lats.First()));
        check_near(surf.Cylinder().Radius(), 12.0, 1e-9, "cyl+2: exact radius 12");
        check(surf.Cylinder().Axis().Direction().IsParallel(gp_Dir(0, 0, 1), 1e-9),
              "cyl+2: axis unchanged");
    }
}

// 5. Through-hole σ-convention: box 20×20×10 with r=5 hole; growing the hole to
//    r=6 requires d = σ(target−current) with σ=−1 ⇒ d=−1. Empirically pins the
//    sign math in the plan (naive +1 would SHRINK the hole).
void test_hole_sigma_convention() {
    const TopoDS_Shape slab = BRepPrimAPI_MakeBox(gp_Pnt(-10, -10, 0), 20.0, 20.0, 10.0).Shape();
    const TopoDS_Shape drill = BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(0, 0, -1), gp_Dir(0, 0, 1)), 5.0, 12.0).Shape();
    const TopoDS_Shape holed = BRepAlgoAPI_Cut(slab, drill).Shape();
    const double base_vol = 20.0 * 20.0 * 10.0 - M_PI * 25.0 * 10.0;
    check_near(volume_of(holed), base_vol, 1e-3, "hole: base volume");

    TopTools_ListOfShape lats = cylindrical_faces(holed);
    check(lats.Extent() == 1, "hole: one cylindrical face");

    // d = -1 (σ=-1, target r 6): hole grows.
    OffsetRun grow(holed);
    grow.offset_face(TopoDS::Face(lats.First()), -1.0);
    check(grow.run(), "hole d=-1: IsDone");
    check(grow.valid(), "hole d=-1: valid");
    check_near(volume_of(grow.result), 20.0 * 20.0 * 10.0 - M_PI * 36.0 * 10.0, 1e-3,
               "hole d=-1: hole grew to r=6");

    // d = +1: material grows inward, hole SHRINKS to r=4 (the naive-sign trap).
    OffsetRun shrink(holed);
    TopTools_ListOfShape lats2 = cylindrical_faces(holed);
    shrink.offset_face(TopoDS::Face(lats2.First()), 1.0);
    check(shrink.run(), "hole d=+1: IsDone");
    check_near(volume_of(shrink.result), 20.0 * 20.0 * 10.0 - M_PI * 16.0 * 10.0, 1e-3,
               "hole d=+1: hole shrank to r=4 (σ trap confirmed)");
}

// 6. GO/NO-GO — split cylindrical wall (two tangent half-lateral faces).
//    (a) both offset +2 → must produce the same result as the full-cylinder case;
//    (b) only ONE offset → geometrically impossible to reclose tangentially
//        (coaxial r10/r12 never intersect); MUST be loudly detectable, never a
//        silently-published wrong body.
void test_split_cylinder_go_no_go() {
    const gp_Ax2 up(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1), gp_Dir(1, 0, 0));
    const gp_Ax2 down(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1), gp_Dir(-1, 0, 0));
    const TopoDS_Shape half_a = BRepPrimAPI_MakeCylinder(up, 10.0, 20.0, M_PI).Shape();
    const TopoDS_Shape half_b = BRepPrimAPI_MakeCylinder(down, 10.0, 20.0, M_PI).Shape();
    const TopoDS_Shape fused = BRepAlgoAPI_Fuse(half_a, half_b).Shape();
    check_near(volume_of(fused), M_PI * 100.0 * 20.0, 1e-3, "splitcyl: fused full cylinder");
    TopTools_ListOfShape lats = cylindrical_faces(fused);
    std::fprintf(stderr, "INFO: splitcyl fused: %d cylindrical faces, %d total faces\n",
                 lats.Extent(), count_of(fused, TopAbs_FACE));
    if (lats.Extent() != 2) {
        // Fuse unified the lateral faces — the split-wall scenario needs a
        // different construction; report loudly so the spike verdict names it.
        check(false, "splitcyl: expected 2 tangent lateral faces after fuse");
        return;
    }

    // (a) Chained: both faces +2.
    OffsetRun both(fused);
    for (TopTools_ListOfShape::Iterator it(lats); it.More(); it.Next())
        both.offset_face(TopoDS::Face(it.Value()), 2.0);
    const bool both_done = both.run();
    check(both_done, "splitcyl both+2: IsDone");
    if (both_done) {
        check(both.valid(), "splitcyl both+2: valid");
        check_near(volume_of(both.result), M_PI * 144.0 * 20.0, 1e-3,
                   "splitcyl both+2: r=12 volume");
    }

    // (b) One only: CHARACTERIZED KERNEL BEHAVIOR (first spike run) — OCCT
    //     auto-propagates the offset across the tangent junction and returns
    //     the full chained r=12 result (identical to (a)). "One tangent patch
    //     moves, the other fixed" is not expressible on this kernel path.
    //     Consequence for the op: with chainTangentFaces=false, a selection
    //     with tangent-adjacent unselected faces MUST be refused at authoring
    //     (the kernel would silently widen the operative set otherwise).
    TopTools_ListOfShape lats_b = cylindrical_faces(fused);
    OffsetRun one(fused);
    one.offset_face(TopoDS::Face(lats_b.First()), 2.0);
    const bool one_done = one.run();
    std::fprintf(stderr, "INFO: splitcyl one+2: IsDone=%d error=%d\n", one_done ? 1 : 0,
                 static_cast<int>(one.mo.Error()));
    check(one_done, "splitcyl one+2: IsDone (kernel auto-propagates)");
    if (one_done) {
        check(one.valid(), "splitcyl one+2: valid");
        check_near(volume_of(one.result), M_PI * 144.0 * 20.0, 1e-3,
                   "splitcyl one+2: kernel propagated tangent chain (full r=12)");
    }
}

// 7. History probe (box top +2). CHARACTERIZED (first spike run): the public
//    Generated()/Modified() lists are EMPTY for faces and IsDeleted() is false
//    — the adapter CANNOT be built on them. The real face lineage lives in the
//    BRepAlgo_Image returned by OffsetFacesFromShapes() (initial face → offset
//    face links). Pin that here: every original face must be reachable through
//    the image, and its image must contain a face present in the result.
void test_history_image_pattern() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
    OffsetRun run(box);
    run.offset_face(face_by_center(box, 5, 5, 10), 2.0);
    check(run.run(), "history: IsDone");

    TopTools_IndexedMapOfShape result_faces;
    TopExp::MapShapes(run.result, TopAbs_FACE, result_faces);

    const BRepAlgo_Image& img = run.mo.OffsetFacesFromShapes();
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(box, TopAbs_FACE, faces);
    int gen_or_mod = 0, imaged_in_result = 0;
    for (int i = 1; i <= faces.Extent(); ++i) {
        if (!run.mo.Generated(faces(i)).IsEmpty() || !run.mo.Modified(faces(i)).IsEmpty())
            ++gen_or_mod;
        if (img.HasImage(faces(i))) {
            const TopTools_ListOfShape& li = img.Image(faces(i));
            for (TopTools_ListOfShape::Iterator it(li); it.More(); it.Next()) {
                if (it.Value().ShapeType() == TopAbs_FACE &&
                    result_faces.Contains(it.Value())) {
                    ++imaged_in_result;
                    break;
                }
            }
        }
    }
    std::fprintf(stderr,
                 "INFO: history faces=%d generated_or_modified=%d image_in_result=%d\n",
                 faces.Extent(), gen_or_mod, imaged_in_result);
    // Adapter viability: the image must cover every original face with a
    // successor that actually exists in the result shape.
    check(imaged_in_result == faces.Extent(),
          "history: every face reachable via OffsetFacesFromShapes image");
}

// 8. Traps the op's postconditions must catch.
void test_traps() {
    // (a) Negative-radius inside-out: r=10, d=-11 — plan preflights R+σd>tol,
    //     because OCCT may return a "valid" inverted r=1 cylinder.
    const TopoDS_Shape cyl = BRepPrimAPI_MakeCylinder(10.0, 20.0).Shape();
    TopTools_ListOfShape lats = cylindrical_faces(cyl);
    OffsetRun inv(cyl);
    inv.offset_face(TopoDS::Face(lats.First()), -11.0);
    const bool inv_done = inv.run();
    std::fprintf(stderr, "INFO: trap r-11: IsDone=%d\n", inv_done ? 1 : 0);
    if (inv_done && inv.valid()) {
        TopTools_ListOfShape out = cylindrical_faces(inv.result);
        double r_out = -1.0;
        if (out.Extent() >= 1) r_out = BRepAdaptor_Surface(TopoDS::Face(out.First())).Cylinder().Radius();
        std::fprintf(stderr, "INFO: trap r-11: result radius=%.3f volume=%.3f\n", r_out,
                     volume_of(inv.result));
        // Semantic postcondition catches it: expected radius is NEGATIVE ⇒ the
        // op must have preflight-refused; whatever OCCT returned differs from
        // the (impossible) request.
        check(r_out > 0.0, "trap r-11: OCCT returned some positive-radius shape (preflight is ours)");
    }

    // (b) Zero-volume collapse: box top -10 lands on the bottom face. IsDone +
    //     BRepCheck may both pass; the volume postcondition must catch it.
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
    OffsetRun flat(box);
    flat.offset_face(face_by_center(box, 5, 5, 10), -10.0);
    const bool flat_done = flat.run();
    std::fprintf(stderr, "INFO: trap collapse: IsDone=%d\n", flat_done ? 1 : 0);
    if (flat_done) {
        const double v = volume_of(flat.result);
        std::fprintf(stderr, "INFO: trap collapse: valid=%d volume=%.6f\n",
                     flat.valid() ? 1 : 0, v);
        check(v < 1.0, "trap collapse: volume ~0 (volume postcondition is the catch)");
    }

    // (c) Foreign face: silently ignored by OCCT — ownership preflight is ours.
    const TopoDS_Shape other = BRepPrimAPI_MakeBox(gp_Pnt(100, 100, 100), 5.0, 5.0, 5.0).Shape();
    OffsetRun foreign(box);
    foreign.offset_face(face_by_center(other, 102.5, 102.5, 105), 2.0);
    const bool foreign_done = foreign.run();
    if (foreign_done) {
        check_near(volume_of(foreign.result), 1000.0, 1e-6,
                   "trap foreign: shape unchanged (ownership preflight is ours)");
    } else {
        std::fprintf(stderr, "INFO: trap foreign: IsDone=false (also acceptable)\n");
    }
}

}  // namespace

int main() {
    test_box_top_plus2();
    test_box_top_minus2();
    test_frustum_taper_preserved();
    test_cylinder_radius_grow();
    test_hole_sigma_convention();
    test_split_cylinder_go_no_go();
    test_history_image_pattern();
    test_traps();
    if (g_failures == 0) std::fprintf(stderr, "test_offsetface_spike: all checks passed\n");
    return g_failures;
}
