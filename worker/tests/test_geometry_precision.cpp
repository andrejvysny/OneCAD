// test_geometry_precision.cpp — the IDENTITY HARNESS for GeometryPrecisionContext.
//
// This file is the entire safety argument for the precision migration. Every
// literal that is going to be replaced by a context accessor is asserted here,
// INLINED, against the accessor — before the literal is deleted. If a call site
// then moves and a suite reds, the cause is the move, not the number.
//
// The inlining is deliberate. Comparing the accessor against the constant it is
// supposed to replace, by referring to that constant's own header, would be
// circular: deleting the constant would delete the assertion with it. So the
// pre-migration expressions are written out here as literal arithmetic, copied
// from the sites named in each check.

#include <algorithm>
#include <cstdio>
#include <string>

#include <BRepBuilderAPI_MakeVertex.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <Precision.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Pnt.hxx>
#include <gp_Trsf.hxx>
#include <gp_Vec.hxx>

#include "kernel/validation/GeometryPrecision.h"

namespace validation = onecad::kernel::validation;

namespace {

int g_failures = 0;

void check(bool condition, const std::string &message) {
    if (condition) return;
    std::fprintf(stderr, "FAIL: %s\n", message.c_str());
    ++g_failures;
}

void check_exact(double actual, double expected, const std::string &message) {
    // Bit-equality, not "close". A rename that shifts a threshold by one ULP is
    // still a behaviour change, and this harness exists to refuse exactly that.
    if (actual == expected) return;
    std::fprintf(stderr, "FAIL: %s (expected %.17g, got %.17g)\n", message.c_str(), expected,
                 actual);
    ++g_failures;
}

TopoDS_Shape box(double size) {
    return BRepPrimAPI_MakeBox(size, size, size).Shape();
}

TopoDS_Shape translated(const TopoDS_Shape &shape, double x, double y, double z) {
    gp_Trsf move;
    move.SetTranslation(gp_Vec(x, y, z));
    return BRepBuilderAPI_Transform(shape, move, true).Shape();
}

// ── The environment assumptions the identities rest on ──────────────────────
void occt_confusion_is_what_the_thresholds_assume() {
    // Several identities below reduce to `Precision::Confusion()`. If a future
    // OCCT changes it, those identities silently start meaning something else,
    // so it is pinned here rather than assumed.
    check_exact(Precision::Confusion(), 1.0e-7, "OCCT Precision::Confusion() is 1e-7 mm");
}

// ── authoring_resolution: the `kMinValue` family ────────────────────────────
void authoring_resolution_reproduces_the_min_value_family() {
    // Inlined from ExtrudeOp.cpp `kMinValue` / `kToFaceMin`, FilletChamferOp.cpp
    // `kMinValue`, HoleOp.cpp `kMinValue`, ShellOp.cpp `kMinValue`, and
    // OffsetFaceOp.h `kMinimumFeatureChange` — all of which are this number.
    constexpr double kInlinedMinValue = 1e-3;

    // Constant across the whole supported range, on purpose (see the header):
    // making it scale would move a refusal boundary in ~14 operations at once.
    for (const double size : {0.01, 1.0, 100.0, 10000.0}) {
        const auto precision = validation::precision_of(box(size));
        check_exact(precision.authoring_resolution(), kInlinedMinValue,
                    "authoring_resolution is the kMinValue literal at scale " +
                        std::to_string(size));
    }
}

// ── The FLOOR-ONLY context, which several migrated guards now use ────────────
//
// Parameter validation runs before any shape exists (Hole's `read_dims`,
// Fillet/Chamfer's `read_values`, Extrude's distance guards, Gear's spec guards,
// OffsetFace's anti-parallel tests), so those sites read a default-constructed
// context rather than a measured one. That is only sound if the default context
// answers the same floors the deleted literals did.
void floor_only_context_answers_the_inlined_floors() {
    const validation::GeometryPrecisionContext floor_only;

    // Inlined from the deleted `kMinValue` family (ExtrudeOp `kMinValue`/`kToFaceMin`,
    // FilletChamferOp, HoleOp, HoleTool, ShellOp, GearTool).
    check_exact(floor_only.authoring_resolution(), 1e-3,
                "the floor-only context answers the kMinValue literal");
    // Inlined from the deleted OffsetFaceOp.h `kSemanticAngularTol`. The angular
    // budget carries no conditioning term, so floor-only is EXACT, not merely
    // conservative — that is what lets the anti-parallel guards use it.
    check_exact(floor_only.semantic_angular(), 1.0e-7,
                "the floor-only context answers the kSemanticAngularTol literal");
    // Inlined from the deleted OffsetFaceOp.h `kSemanticLengthTol`.
    check_exact(floor_only.semantic_length(), 1.0e-6,
                "the floor-only context answers the kSemanticLengthTol literal");

    // Gear's degenerate-chord guard is a thousandth of the authoring resolution and
    // is computed at the site; pin the arithmetic so the site cannot drift.
    check_exact(floor_only.authoring_resolution() * 1e-3, 1e-6,
                "the gear degenerate-chord guard is 1e-6 mm");
}

// ── minimum_volume: OffsetFace's `kMinVolume` ───────────────────────────────
void minimum_volume_reproduces_the_offset_face_literal() {
    // Inlined from OffsetFaceOp.h `kMinVolume`. Its comment calls it "a 1 µm
    // cube"; deriving it from the authoring resolution makes that arithmetic
    // rather than an assertion.
    constexpr double kInlinedMinVolume = 1.0e-9;
    const auto precision = validation::precision_of(box(100.0));
    check_exact(precision.minimum_volume(), kInlinedMinVolume,
                "minimum_volume is the kMinVolume literal");
}

// ── tolerance_ceiling: the two shapes that exist today ──────────────────────
void tolerance_ceiling_reproduces_extrude_and_fillet() {
    const TopoDS_Shape shape = box(100.0);
    const auto precision = validation::precision_of(shape);

    // Inlined from ExtrudeOp.cpp:877 and FilletBuilder.cpp:138-139, which are
    // the same expression over the same three-way max of face/edge/vertex
    // tolerances that `precision_of` computes.
    const double inlined = std::max(1.0e-3, precision.input_tolerance * 2.0 + 1.0e-6);
    check_exact(precision.tolerance_ceiling(precision.input_tolerance, 2.0, 1.0e-6), inlined,
                "tolerance_ceiling(input, 2, 1e-6) is the Extrude/Fillet ceiling");
}

void tolerance_ceiling_reproduces_offset_face() {
    const TopoDS_Shape shape = box(100.0);
    const auto precision = validation::precision_of(shape);

    // Inlined from OffsetFaceOp.cpp:1087. TWO differences from the above, and
    // both are real rather than sloppiness — which is why the accessor takes the
    // base explicitly and refuses to default it:
    //   1. the base is the CONSTRUCTION tolerance, not the input shape's;
    //   2. epsilon is 0, not 1e-6.
    // Unifying either would loosen a live ceiling, so it is a separate gate.
    const double construction_tolerance =
        std::max({Precision::Confusion(), precision.input_tolerance, 1.0e-4});
    const double inlined = std::max(1.0e-3, construction_tolerance * 2.0);
    check_exact(precision.tolerance_ceiling(construction_tolerance, 2.0, 0.0), inlined,
                "tolerance_ceiling(construction, 2, 0) is the OffsetFace ceiling");
}

// ── The conditioning term cannot move an existing threshold ─────────────────
//
// This is the property that makes adding conditioning to a "pure rename" safe,
// and it is the single most load-bearing check in the file. If it ever fails,
// every kernelbench digest is at risk and the migration must stop.
void conditioning_never_reaches_an_existing_threshold() {
    struct Placement {
        const char *name;
        double x, y, z;
    };
    // The middle entry is the t0 metamorph's own translation
    // (bench/robustness/presets/fillet-foundation-t0.json), so this asserts the
    // property directly at the coordinates the pinned digests are recorded at.
    // The last is the largest supported distance from the origin, 1 km.
    const Placement placements[] = {
        {"origin", 0.0, 0.0, 0.0},
        {"t0 metamorph translation", 1000.0, -2000.0, 3000.0},
        {"1 km from origin", 1.0e6, 1.0e6, 1.0e6},
    };

    for (const auto &placement : placements) {
        const TopoDS_Shape shape =
            translated(box(100.0), placement.x, placement.y, placement.z);
        const auto precision = validation::precision_of(shape);

        // Inlined from OffsetFaceOp.h `kSemanticLengthTol` / `kSemanticAngularTol`.
        check_exact(precision.semantic_length(), 1.0e-6,
                    std::string("semantic_length is unmoved at ") + placement.name);
        check_exact(precision.semantic_angular(), 1.0e-7,
                    std::string("semantic_angular is unmoved at ") + placement.name);

        // The claim is precisely that CONDITIONING never dominates — not that
        // the shape's own tolerance stays at Confusion, which a transform at 1 km
        // is entitled to inflate.
        check_exact(precision.linear_resolution(),
                    std::max(Precision::Confusion(), precision.input_tolerance),
                    std::string("conditioning does not drive linear_resolution at ") +
                        placement.name);

        check(precision.coordinate_magnitude * validation::kConditioning <
                  Precision::Confusion(),
              std::string("conditioning stays under Confusion at ") + placement.name);
    }
}

// ── Measurement ─────────────────────────────────────────────────────────────
void scale_and_conditioning_are_measured_separately() {
    // A small part far from the origin: model scale and conditioning are
    // different questions, and conflating them is what a bbox-ratio threshold
    // does wrong.
    const TopoDS_Shape shape = translated(box(0.01), 1000.0, 0.0, 0.0);
    const auto precision = validation::precision_of(shape);
    check(precision.scale_diagonal < 0.02,
          "scale_diagonal follows the part, not its distance from the origin");
    check(precision.coordinate_magnitude > 999.0,
          "coordinate_magnitude follows the distance from the origin");
}

void feature_magnitude_is_recorded_as_a_magnitude() {
    const auto precision = validation::precision_of(box(100.0), -5.0);
    check_exact(precision.feature_magnitude, 5.0,
                "feature_magnitude is the absolute requested change");
}

// ── Degenerate inputs answer with floors, never a guess ─────────────────────
void unmeasurable_shapes_fall_back_to_floors() {
    const auto null_precision = validation::precision_of(TopoDS_Shape());
    check_exact(null_precision.scale_diagonal, 0.0, "a null shape has no scale");
    check_exact(null_precision.coordinate_magnitude, 0.0, "a null shape has no conditioning");
    check_exact(null_precision.authoring_resolution(), 1.0e-3,
                "a null shape still answers the authoring resolution");
    check_exact(null_precision.linear_resolution(), Precision::Confusion(),
                "a null shape falls back to Confusion");

    // A vertex has a real bounding POINT but no extent. OCCT still hands back a
    // box inflated by the vertex tolerance, so the diagonal is tiny rather than
    // literally zero — what matters is that it never becomes a usable scale.
    const TopoDS_Shape vertex = BRepBuilderAPI_MakeVertex(gp_Pnt(1.0, 2.0, 3.0)).Shape();
    const auto vertex_precision = validation::precision_of(vertex);
    check(vertex_precision.scale_diagonal < vertex_precision.authoring_resolution(),
          "a zero-extent shape has no usable scale");
    check(vertex_precision.coordinate_magnitude > 3.0,
          "a zero-extent shape still reports its conditioning");
    check_exact(vertex_precision.minimum_volume(), 1.0e-9,
                "a zero-extent shape still answers the minimum volume");
}

void micro_thresholds_are_absolute_not_bbox_ratios() {
    // The defect this replaces: a ratio to the bounding-box diagonal judges a
    // small feature by the size of the part it sits on. Two parts four orders
    // apart in scale must agree at the origin, because tolerance does.
    const auto tiny = validation::precision_of(box(0.01));
    const auto huge = validation::precision_of(box(10000.0));
    check_exact(tiny.micro_edge_length(), huge.micro_edge_length(),
                "micro_edge_length does not depend on part size");
    check_exact(tiny.sliver_face_width(), huge.sliver_face_width(),
                "sliver_face_width does not depend on part size");
    check(tiny.micro_edge_length() > 0.0, "micro_edge_length is positive");

    // And it IS reachable: a 1 µm edge on a 10 m part is caught, where the
    // superseded ratio test (length/diagonal < 1e-9) would have needed the edge
    // to be under 1.7e-5 mm.
    check(0.001 > huge.micro_edge_length(),
          "a 1 um edge is above the micro threshold on a 10 m part");
}

void json_carries_the_derived_budgets() {
    const auto precision = validation::precision_of(box(100.0), 2.0);
    const nlohmann::json evidence = precision.to_json();
    check(evidence["version"] == validation::kGeometryPrecisionVersion,
          "evidence carries the precision version");
    check(evidence.contains("authoringResolution") && evidence.contains("linearResolution") &&
              evidence.contains("minimumVolume"),
          "evidence carries the derived budgets, not just the measured inputs");
}

} // namespace

int main() {
    occt_confusion_is_what_the_thresholds_assume();
    authoring_resolution_reproduces_the_min_value_family();
    floor_only_context_answers_the_inlined_floors();
    minimum_volume_reproduces_the_offset_face_literal();
    tolerance_ceiling_reproduces_extrude_and_fillet();
    tolerance_ceiling_reproduces_offset_face();
    conditioning_never_reaches_an_existing_threshold();
    scale_and_conditioning_are_measured_separately();
    feature_magnitude_is_recorded_as_a_magnitude();
    unmeasurable_shapes_fall_back_to_floors();
    micro_thresholds_are_absolute_not_bbox_ratios();
    json_carries_the_derived_budgets();
    if (g_failures == 0) std::fprintf(stderr, "geometry_precision: OK\n");
    // An exit status that is a multiple of 256 reports PASS to the shell, so the
    // raw failure count must never be returned directly.
    return g_failures == 0 ? 0 : 1;
}
