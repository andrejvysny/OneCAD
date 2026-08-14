// StepRead.cpp — see StepRead.h.
#include "io/StepRead.h"

#include <algorithm>
#include <string>
#include <vector>

#include <BRepBuilderAPI_Sewing.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRep_Builder.hxx>
#include <BRep_Tool.hxx>
#include <IFSelect_ReturnStatus.hxx>
#include <Interface_Static.hxx>
#include <Message_ProgressRange.hxx>
#include <STEPControl_Controller.hxx>
#include <STEPControl_Reader.hxx>
#include <ShapeAnalysis_Shell.hxx>
#include <ShapeExtend_Status.hxx>
#include <ShapeFix_Shape.hxx>
#include <ShapeFix_Solid.hxx>
#include <StepBasic_Product.hxx>
#include <StepBasic_ProductDefinition.hxx>
#include <StepBasic_ProductDefinitionFormation.hxx>
#include <StepRepr_ProductDefinitionShape.hxx>
#include <StepShape_ShapeDefinitionRepresentation.hxx>
#include <Standard_Failure.hxx>
#include <TCollection_AsciiString.hxx>
#include <TColStd_SequenceOfAsciiString.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Compound.hxx>
#include <TopoDS_Shell.hxx>
#include <TopoDS_Solid.hxx>
#include <TopoDS_Vertex.hxx>

#include "io/OcctStaticGuard.h"
#include "ops/CancelProgress.h"  // ops::CancelProgress — cancel token ↔ UserBreak()
#include "ops/OpCommon.h"  // ops::ordered_solids — the deterministic solid order
#include "util/Log.h"

namespace onecad::io {

namespace {

// The knob names the v1 policy owns. Named once so the guard, the setter and the
// tests cannot drift apart.
constexpr const char* kKnobUnit = "xstep.cascade.unit";
constexpr const char* kKnobPrecisionMode = "read.precision.mode";
constexpr const char* kKnobPrecisionVal = "read.precision.val";
constexpr const char* kKnobProductMode = "read.step.product.mode";
constexpr const char* kKnobMaxPrecisionMode = "read.maxprecision.mode";
constexpr const char* kKnobMaxPrecisionVal = "read.maxprecision.val";

void add_diag(StepReadResult& out, const char* code, std::string message) {
    out.diagnostics.push_back(StepReadDiagnostic{code, std::move(message)});
}

// Largest vertex tolerance in `shape`, i.e. the file's declared uncertainty as
// the reader materialized it. This — not `read.precision.val`, which is only
// consulted in forced mode — is what a sewing pass has to bridge.
double max_vertex_tolerance(const TopoDS_Shape& shape) {
    double tol = 0.0;
    if (shape.IsNull()) return tol;
    TopTools_IndexedMapOfShape verts;
    TopExp::MapShapes(shape, TopAbs_VERTEX, verts);
    for (int i = 1; i <= verts.Extent(); ++i) {
        tol = std::max(tol, static_cast<double>(BRep_Tool::Tolerance(TopoDS::Vertex(verts(i)))));
    }
    return tol;
}

// A closed shell is one with no free (single-adjacent) edge. The `Closed` flag on
// TopoDS_Shell is only as trustworthy as whoever last set it, so the free-edge
// analysis is the authority and the flag is the cheap fast path.
bool shell_is_closed(const TopoDS_Shell& shell) {
    if (BRep_Tool::IsClosed(shell)) return true;
    ShapeAnalysis_Shell analyzer;
    analyzer.LoadShells(shell);
    analyzer.CheckOrientedShells(shell, Standard_True);
    return analyzer.HasFreeEdges() == Standard_False;
}

// Best-effort STEP product name behind one transfer root. Roots are usually
// `SHAPE_DEFINITION_REPRESENTATION`; `PRODUCT_DEFINITION` is accepted directly so
// a producer that roots differently still resolves. "" when nothing is
// recoverable — W0 treats names as advisory metadata, never as identity.
std::string product_name_of_root(const Handle(Standard_Transient)& entity) {
    if (entity.IsNull()) return {};
    Handle(StepBasic_ProductDefinition) pd =
        Handle(StepBasic_ProductDefinition)::DownCast(entity);
    if (pd.IsNull()) {
        Handle(StepShape_ShapeDefinitionRepresentation) sdr =
            Handle(StepShape_ShapeDefinitionRepresentation)::DownCast(entity);
        if (sdr.IsNull()) return {};
        Handle(StepRepr_ProductDefinitionShape) pds =
            Handle(StepRepr_ProductDefinitionShape)::DownCast(sdr->Definition().PropertyDefinition());
        if (pds.IsNull()) return {};
        pd = pds->Definition().ProductDefinition();
    }
    if (pd.IsNull()) return {};
    Handle(StepBasic_ProductDefinitionFormation) formation = pd->Formation();
    if (formation.IsNull()) return {};
    Handle(StepBasic_Product) product = formation->OfProduct();
    if (product.IsNull() || product->Name().IsNull()) return {};
    return std::string(product->Name()->ToCString());
}

// Outcome of the fixed heal pipeline for ONE transfer root.
struct HealedRoot {
    TopoDS_Shape shape;              // compound of promoted solids (possibly empty)
    bool sewn = false;               // sewing actually merged edges
    bool healed = false;             // ShapeFix_Shape reported ShapeExtend_DONE
    bool topology_changed = false;   // …and it changed face/edge/vertex counts
    bool had_open_shell = false;
};

// Face/edge/vertex counts — the cheap "did healing actually restructure this"
// probe. `ShapeFix_Shape` reports `ShapeExtend_DONE` for a tolerance-only
// touch-up too (observed: it fires on a pristine OCCT-written box), so DONE alone
// is not evidence that the incoming geometry was defective.
struct ShapeCounts {
    int faces = 0;
    int edges = 0;
    int vertices = 0;

    bool operator!=(const ShapeCounts& o) const {
        return faces != o.faces || edges != o.edges || vertices != o.vertices;
    }
};

ShapeCounts count_shape(const TopoDS_Shape& shape) {
    ShapeCounts c;
    if (shape.IsNull()) return c;
    TopTools_IndexedMapOfShape m;
    TopExp::MapShapes(shape, TopAbs_FACE, m);
    c.faces = m.Extent();
    m.Clear();
    TopExp::MapShapes(shape, TopAbs_EDGE, m);
    c.edges = m.Extent();
    m.Clear();
    TopExp::MapShapes(shape, TopAbs_VERTEX, m);
    c.vertices = m.Extent();
    return c;
}

// Stage 1 — sew. Faces are stitched into shells at the file's own tolerance,
// floored so a degenerate declared precision cannot make this a no-op. Run
// UNCONDITIONALLY: a root that is already a well-formed solid survives sewing
// unchanged, and skipping it conditionally would make the pipeline path depend on
// the input's prior state (the determinism contract in StepRead.h).
TopoDS_Shape sew_root(const TopoDS_Shape& root, double tol, bool& sewn_out) {
    BRepBuilderAPI_Sewing sewer(tol, /*option1 sewing*/ Standard_True,
                                /*option2 analysis*/ Standard_True,
                                /*option3 cutting*/ Standard_True,
                                /*option4 non-manifold*/ Standard_False);
    sewer.Add(root);
    sewer.Perform();
    const TopoDS_Shape sewed = sewer.SewedShape();
    sewn_out = sewer.NbContigousEdges() > 0;
    return sewed.IsNull() ? root : sewed;
}

// Stage 2 — heal. `ShapeFix_Shape` with the wire fixes enabled, bounded by the
// policy's max precision. Unconditional, same rationale as sewing.
TopoDS_Shape heal_shape(const TopoDS_Shape& shape, double tol, double max_tol, bool& healed_out) {
    Handle(ShapeFix_Shape) fixer = new ShapeFix_Shape(shape);
    fixer->SetPrecision(tol);
    fixer->SetMinTolerance(tol);
    fixer->SetMaxTolerance(max_tol);
    // Pinned explicitly rather than inherited: OCCT's defaults are all-on today,
    // but "the defaults" is not a determinism contract across OCCT versions.
    // `Free*` here means "sub-shapes not owned by a solid" — the solid tool
    // cascades into shell → face → wire for everything it does own, so enabling
    // all four covers both the owned and the free geometry a STEP root can carry.
    fixer->FixSolidMode() = 1;
    fixer->FixFreeShellMode() = 1;
    fixer->FixFreeFaceMode() = 1;
    fixer->FixFreeWireMode() = 1;  // wire fixes ON (brief): STEP wires are the usual offender
    fixer->Perform();
    healed_out = fixer->Status(ShapeExtend_DONE) == Standard_True;
    const TopoDS_Shape fixed = fixer->Shape();
    return fixed.IsNull() ? shape : fixed;
}

// Stage 3 — promote. Existing solids pass through verbatim; every CLOSED shell
// not already owned by one becomes a solid. Open shells are dropped here and
// reported by the caller as a skipped root.
TopoDS_Shape promote_solids(const TopoDS_Shape& shape, double tol, bool& open_shell_out) {
    BRep_Builder builder;
    TopoDS_Compound out;
    builder.MakeCompound(out);
    if (shape.IsNull()) return out;

    TopTools_IndexedMapOfShape owned_shells;
    for (TopExp_Explorer se(shape, TopAbs_SOLID); se.More(); se.Next()) {
        builder.Add(out, se.Current());
        TopExp::MapShapes(se.Current(), TopAbs_SHELL, owned_shells);
    }
    for (TopExp_Explorer sh(shape, TopAbs_SHELL); sh.More(); sh.Next()) {
        if (owned_shells.Contains(sh.Current())) continue;
        const TopoDS_Shell shell = TopoDS::Shell(sh.Current());
        if (!shell_is_closed(shell)) {
            open_shell_out = true;
            continue;
        }
        ShapeFix_Solid solid_fixer;
        solid_fixer.SetPrecision(tol);
        const TopoDS_Solid solid = solid_fixer.SolidFromShell(shell);
        if (!solid.IsNull()) builder.Add(out, solid);
    }
    return out;
}

HealedRoot heal_root(const TopoDS_Shape& root, const StepReadPolicy& policy) {
    HealedRoot hr;
    const double file_tol = max_vertex_tolerance(root);
    const double tol = std::max(policy.min_sew_tolerance, file_tol);
    const double max_tol = std::max(tol, policy.max_precision_val);

    const TopoDS_Shape sewed = sew_root(root, tol, hr.sewn);
    const ShapeCounts before = count_shape(sewed);
    const TopoDS_Shape fixed = heal_shape(sewed, tol, max_tol, hr.healed);
    hr.topology_changed = hr.healed && count_shape(fixed) != before;
    hr.shape = promote_solids(fixed, tol, hr.had_open_shell);
    return hr;
}

// Every declared length unit of the file, uppercased, in declaration order.
// Empty when the file declares none (OCCT then assumes millimetres).
std::vector<std::string> file_length_units(STEPControl_Reader& reader) {
    TColStd_SequenceOfAsciiString lengths;
    TColStd_SequenceOfAsciiString angles;
    TColStd_SequenceOfAsciiString solid_angles;
    reader.FileUnits(lengths, angles, solid_angles);
    std::vector<std::string> out;
    out.reserve(static_cast<std::size_t>(lengths.Length()));
    for (int i = 1; i <= lengths.Length(); ++i) {
        TCollection_AsciiString name = lengths.Value(i);
        name.UpperCase();
        out.emplace_back(name.ToCString());
    }
    return out;
}

bool is_mm(const std::string& name) {
    return name == "MM" || name == "MILLIMETRE" || name == "MILLIMETER";
}

// A STEP length unit that is not millimetres means OCCT scaled the geometry on
// the way in. Worth a diagnostic: the scale factor is a rounding source the codec
// decision has to price in.
bool units_are_mm(const std::vector<std::string>& units) {
    if (units.empty()) return true;  // undeclared ⇒ OCCT assumes mm; not a conversion
    for (const std::string& name : units) {
        if (!is_mm(name)) return false;
    }
    return true;
}

// `source_unit` reporting form. The three millimetre spellings collapse to "MM"
// so a file that declares MILLIMETRE and a file that declares nothing (where
// OCCT assumes mm) report the SAME unit — a caller comparing against the "MM"
// fallback must not see two different answers for the same physical unit. Any
// other unit is reported verbatim, uppercased.
std::string canonical_unit(const std::string& name) { return is_mm(name) ? "MM" : name; }

}  // namespace

StepReadResult read_step(const std::string& path, const StepReadPolicy& policy,
                         const onecad::CancelToken* cancel) {
    StepReadResult out;
    // Cancellation is a DISTINCT outcome, not a failure: `cancelled` drives the
    // caller to SCHEMA §8 CANCELLED (session intact) instead of OP_FAILED.
    const auto bail_cancelled = [&out]() {
        out.solids.clear();
        out.cancelled = true;
        out.error = "StepRead: cancelled";
        return out;
    };

    if (path.empty()) {
        out.error = "StepRead: empty path";
        return out;
    }
    if (cancel != nullptr && cancel->cancelled()) return bail_cancelled();

    // Registers the STEP norm AND its Interface_Static knobs. Must run before the
    // guard snapshots them, otherwise every knob reads as absent and the pinning
    // silently degrades to "whatever the process already held". Idempotent.
    STEPControl_Controller::Init();

    // Order matters: the messenger guard is outermost so OCCT chatter raised by
    // the knob writes themselves cannot reach fd 1 either.
    OcctMessengerGuard quiet;
    InterfaceStaticGuard knobs;

    try {
        knobs.set_cstr(kKnobUnit, policy.target_unit);
        knobs.set_int(kKnobPrecisionMode, policy.precision_mode);
        knobs.set_real(kKnobPrecisionVal, policy.precision_val);
        knobs.set_int(kKnobProductMode, policy.product_mode);
        knobs.set_int(kKnobMaxPrecisionMode, policy.max_precision_mode);
        knobs.set_real(kKnobMaxPrecisionVal, policy.max_precision_val);

        STEPControl_Reader reader;
        const IFSelect_ReturnStatus status = reader.ReadFile(path.c_str());
        if (status != IFSelect_RetDone) {
            out.error = "StepRead: ReadFile failed (status " +
                        std::to_string(static_cast<int>(status)) + ") for " + path;
            return out;
        }

        const std::vector<std::string> units = file_length_units(reader);
        if (!units.empty()) out.source_unit = canonical_unit(units.front());
        if (!units_are_mm(units)) {
            add_diag(out, step_diag::kUnitConverted,
                     "file length unit is not millimetres; converted to " + policy.target_unit);
        }

        const int root_count = static_cast<int>(reader.NbRootsForTransfer());
        for (int r = 1; r <= root_count; ++r) {
            out.product_names.push_back(product_name_of_root(reader.RootForTransfer(r)));
        }

        // The transfer is the long pole; drive it through a progress range backed
        // by the cancel token so `UserBreak()` aborts it between algorithm steps.
        Message_ProgressRange transfer_range;
        Handle(ops::CancelProgress) progress;
        if (cancel != nullptr) {
            progress = new ops::CancelProgress(*cancel);
            transfer_range = progress->Start();
        }
        const int transferred = static_cast<int>(reader.TransferRoots(transfer_range));
        if (cancel != nullptr && cancel->cancelled()) return bail_cancelled();
        // NbShapes() indexes the TRANSFERRED SHAPES, which are not 1:1 with the
        // roots counted above — a root can fail to translate, and OCCT accumulates
        // results across calls. Iterate the shapes; report against both counts so a
        // partial transfer is visible in the diagnostic rather than inferred.
        const int shape_count = static_cast<int>(reader.NbShapes());
        if (transferred < root_count) {
            add_diag(out, step_diag::kRootSkipped,
                     "only " + std::to_string(transferred) + " of " + std::to_string(root_count) +
                         " root(s) translated");
        }

        BRep_Builder builder;
        TopoDS_Compound all;
        builder.MakeCompound(all);
        bool any_sewn = false;
        bool any_healed = false;
        bool any_restructured = false;

        for (int i = 1; i <= shape_count; ++i) {
            if (cancel != nullptr && cancel->cancelled()) return bail_cancelled();
            const TopoDS_Shape shape = reader.Shape(i);
            if (shape.IsNull()) {
                add_diag(out, step_diag::kRootSkipped,
                         "transferred shape " + std::to_string(i) + " is null");
                continue;
            }
            const HealedRoot hr = heal_root(shape, policy);
            any_sewn = any_sewn || hr.sewn;
            any_healed = any_healed || hr.healed;
            any_restructured = any_restructured || hr.topology_changed;

            int shape_solids = 0;
            for (TopExp_Explorer se(hr.shape, TopAbs_SOLID); se.More(); se.Next()) ++shape_solids;
            if (shape_solids == 0) {
                add_diag(out, step_diag::kRootSkipped,
                         "transferred shape " + std::to_string(i) + " yielded no solid" +
                             (hr.had_open_shell ? " (open shell after healing)" : ""));
                continue;
            }
            builder.Add(all, hr.shape);
        }

        if (any_sewn) add_diag(out, step_diag::kSewn, "faces were stitched into shells");
        if (any_healed) {
            add_diag(out, step_diag::kHealed,
                     any_restructured
                         ? "ShapeFix_Shape restructured the geometry (face/edge/vertex counts "
                           "changed)"
                         : "ShapeFix_Shape reported a fix; topology counts unchanged (tolerance-"
                           "level touch-up)");
        }

        const std::vector<ops::RankedSolid> ranked = ops::ranked_solids(all);
        for (std::size_t k = 1; k < ranked.size(); ++k) {
            if (!(ranked[k - 1].key < ranked[k].key) && !(ranked[k].key < ranked[k - 1].key)) {
                out.error = std::string(kAmbiguousImportOrder) +
                            ": two solids share the exact canonical ordering key";
                out.solids.clear();
                return out;
            }
        }
        out.solids.reserve(ranked.size());
        for (const ops::RankedSolid& solid : ranked) out.solids.push_back(solid.shape);
        if (out.solids.empty()) {
            add_diag(out, step_diag::kNoSolids,
                     "no solid recovered from " + std::to_string(shape_count) +
                         " transferred shape(s)");
            return out;
        }

        for (std::size_t k = 0; k < out.solids.size(); ++k) {
            BRepCheck_Analyzer analyzer(out.solids[k]);
            if (analyzer.IsValid() != Standard_True) {
                add_diag(out, step_diag::kInvalidShape,
                         "solid " + std::to_string(k) + " fails BRepCheck after healing");
            }
        }
    } catch (const Standard_Failure& f) {
        // An aborted OCCT algorithm usually SURFACES as a Standard_Failure, so the
        // token has to be consulted before the exception is rendered as a failure.
        if (cancel != nullptr && cancel->cancelled()) return bail_cancelled();
        out.solids.clear();
        out.error = std::string("StepRead raised: ") +
                    (f.GetMessageString() != nullptr ? f.GetMessageString() : "OCCT failure");
        WLOG_ERROR("StepRead: %s", out.error->c_str());
        return out;
    } catch (const std::exception& e) {
        if (cancel != nullptr && cancel->cancelled()) return bail_cancelled();
        out.solids.clear();
        out.error = std::string("StepRead raised: ") + e.what();
        WLOG_ERROR("StepRead: %s", out.error->c_str());
        return out;
    }

    return out;
}

}  // namespace onecad::io
