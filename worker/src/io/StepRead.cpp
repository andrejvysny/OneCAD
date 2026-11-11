// StepRead.cpp — see StepRead.h.
#include "io/StepRead.h"
#include "io/SourceCapture.h"
#include "util/PolicyVersions.h"

#include <algorithm>
#include <map>
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
#include <StepData_StepModel.hxx>
#include <StepShape_AdvancedFace.hxx>
#include <TransferBRep.hxx>
#include <Transfer_TransientProcess.hxx>
#include <XSControl_WorkSession.hxx>
#include <XSControl_TransferReader.hxx>
#include <ShapeBuild_ReShape.hxx>
#include <ShapeAnalysis_Shell.hxx>
#include <ShapeExtend_Status.hxx>
#include <ShapeFix_Shape.hxx>
#include <ShapeFix_Solid.hxx>
#include <StepBasic_Product.hxx>
#include <StepBasic_ProductDefinition.hxx>
#include <StepBasic_ProductDefinitionFormation.hxx>
#include <StepRepr_ProductDefinitionShape.hxx>
#include <StepRepr_AssemblyComponentUsage.hxx>
#include <StepRepr_MappedItem.hxx>
#include <StepRepr_RepresentationMap.hxx>
#include <StepRepr_RepresentationRelationshipWithTransformation.hxx>
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
    int open_shells = 0;             // non-solid geometry promote_solids dropped
    int free_faces = 0;
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

static bool is_source_policy_v1(const StepReadPolicy& policy) {
    const StepReadPolicy expected;
    return policy.target_unit == expected.target_unit && policy.precision_mode == expected.precision_mode &&
           policy.precision_val == expected.precision_val && policy.product_mode == expected.product_mode &&
           policy.max_precision_mode == expected.max_precision_mode &&
           policy.max_precision_val == expected.max_precision_val &&
           policy.min_sew_tolerance == expected.min_sew_tolerance;
}

static std::vector<SourceFaceClaim> step_source_faces(STEPControl_Reader& reader,
                                                       const TopoDS_Shape& root) {
    const auto model = reader.StepModel();
    const auto process = reader.WS()->TransferReader()->TransientProcess();
    for (int i = 1; i <= model->NbEntities(); ++i) {
        const auto entity = model->Entity(i);
        if (entity->IsKind(STANDARD_TYPE(StepRepr_AssemblyComponentUsage)) ||
            entity->IsKind(STANDARD_TYPE(StepRepr_MappedItem)) ||
            entity->IsKind(STANDARD_TYPE(StepRepr_RepresentationMap)) ||
            entity->IsKind(STANDARD_TYPE(StepRepr_RepresentationRelationshipWithTransformation))) {
            return {};
        }
    }
    int root_label = 0;
    int matching_roots = 0;
    for (int i = 1; i <= reader.NbRootsForTransfer(); ++i) {
        const auto entity = reader.RootForTransfer(i);
        const TopoDS_Shape result = TransferBRep::ShapeResult(process, entity);
        if (!result.IsNull() && result.IsSame(root)) {
            root_label = model->IdentLabel(entity);
            ++matching_roots;
        }
    }
    if (matching_roots != 1 || root_label <= 0) return {};
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(root, TopAbs_FACE, faces);
    if (static_cast<std::size_t>(faces.Extent()) > kMaxSourceFaceClaims) return {};
    std::vector<SourceFaceClaim> claims;
    std::map<int, int> labels;
    for (int i = 1; i <= model->NbEntities(); ++i) {
        const auto entity = model->Entity(i);
        if (!entity->IsKind(STANDARD_TYPE(StepShape_AdvancedFace))) continue;
        const int label = model->IdentLabel(entity);
        if (label <= 0) continue;
        const TopoDS_Shape face = TransferBRep::ShapeResult(process, entity);
        if (face.IsNull() || face.ShapeType() != TopAbs_FACE || faces.FindIndex(face) == 0) continue;
        SourceFaceClaim claim;
        claim.face = face;
        claim.codec = "step";
        claim.codec_version = "occt-step-stream-v1";
        claim.step_root_entity = root_label;
        claim.step_face_entity = label;
        if (claims.size() == static_cast<std::size_t>(faces.Extent())) return {};
        claims.push_back(std::move(claim));
        ++labels[label];
    }
    std::erase_if(claims, [&](const SourceFaceClaim& claim) {
        return labels[claim.step_face_entity] != 1;
    });
    retain_complete_source_faces(claims, root);
    return claims;
}

// Stage 1 — sew. Faces are stitched into shells at the file's own tolerance,
// floored so a degenerate declared precision cannot make this a no-op. Run
// UNCONDITIONALLY: a root that is already a well-formed solid survives sewing
// unchanged, and skipping it conditionally would make the pipeline path depend on
// the input's prior state (the determinism contract in StepRead.h).
TopoDS_Shape sew_root(const TopoDS_Shape& root, double tol, bool& sewn_out,
                       std::vector<SourceFaceClaim>* claims) {
    BRepBuilderAPI_Sewing sewer(tol, /*option1 sewing*/ Standard_True,
                                /*option2 analysis*/ Standard_True,
                                /*option3 cutting*/ Standard_True,
                                /*option4 non-manifold*/ Standard_False);
    sewer.Add(root);
    sewer.Perform();
    const TopoDS_Shape sewed = sewer.SewedShape();
    sewn_out = sewer.NbContigousEdges() > 0;
    if (claims != nullptr && !sewed.IsNull()) {
        for (auto& claim : *claims) {
            const TopoDS_Shape before = claim.face;
            if (sewer.IsModified(claim.face)) claim.face = sewer.Modified(claim.face);
            else if (sewer.IsModifiedSubShape(claim.face)) claim.face = sewer.ModifiedSubShape(claim.face);
            if (!before.IsNull() && !claim.face.IsNull() && !before.IsSame(claim.face)) {
                claim.source_face_replaced = true;
            }
        }
        retain_complete_source_faces(*claims, sewed);
    }
    return sewed.IsNull() ? root : sewed;
}

// Stage 2 — heal. `ShapeFix_Shape` with the wire fixes enabled, bounded by the
// policy's max precision. Unconditional, same rationale as sewing.
TopoDS_Shape heal_shape(const TopoDS_Shape& shape, double tol, double max_tol, bool& healed_out,
                        std::vector<SourceFaceClaim>* claims) {
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
    if (claims != nullptr) {
        for (auto& claim : *claims) {
            const TopoDS_Shape before = claim.face;
            if (!claim.face.IsNull()) claim.face = fixer->Context()->Apply(claim.face);
            if (!before.IsNull() && !claim.face.IsNull() && !before.IsSame(claim.face)) {
                claim.source_face_replaced = true;
            }
        }
    }
    const TopoDS_Shape fixed = fixer->Shape();
    if (claims != nullptr) retain_complete_source_faces(*claims, fixed.IsNull() ? shape : fixed);
    return fixed.IsNull() ? shape : fixed;
}

// Stage 3 — promote. Existing solids pass through verbatim; every CLOSED shell
// not already owned by one becomes a solid. Open shells and faces outside any
// shell are dropped here and COUNTED, so the caller can report the loss whether
// or not the root still yields solids.
TopoDS_Shape promote_solids(const TopoDS_Shape& shape, double tol, HealedRoot& hr) {
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
            ++hr.open_shells;
            continue;
        }
        ShapeFix_Solid solid_fixer;
        solid_fixer.SetPrecision(tol);
        const TopoDS_Solid solid = solid_fixer.SolidFromShell(shell);
        if (!solid.IsNull()) builder.Add(out, solid);
    }
    for (TopExp_Explorer fe(shape, TopAbs_FACE, TopAbs_SHELL); fe.More(); fe.Next()) {
        ++hr.free_faces;
    }
    return out;
}

HealedRoot heal_root(const TopoDS_Shape& root, const StepReadPolicy& policy,
                     std::vector<SourceFaceClaim>* claims = nullptr) {
    HealedRoot hr;
    const double file_tol = max_vertex_tolerance(root);
    const double tol = std::max(policy.min_sew_tolerance, file_tol);
    const double max_tol = std::max(tol, policy.max_precision_val);

    const TopoDS_Shape sewed = sew_root(root, tol, hr.sewn, claims);
    const ShapeCounts before = count_shape(sewed);
    const TopoDS_Shape fixed = heal_shape(sewed, tol, max_tol, hr.healed, claims);
    hr.topology_changed = hr.healed && count_shape(fixed) != before;
    hr.shape = promote_solids(fixed, tol, hr);
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

static StepReadResult read_step_impl(const std::string& path, const std::span<const std::uint8_t>* bytes,
                                    const StepReadPolicy& policy, const onecad::CancelToken* cancel) {
    StepReadResult out;
    // Cancellation is a DISTINCT outcome, not a failure: `cancelled` drives the
    // caller to SCHEMA §8 CANCELLED (session intact) instead of OP_FAILED.
    const auto bail_cancelled = [&out]() {
        out.solids.clear();
        out.face_claims.clear();
        out.cancelled = true;
        out.error = "StepRead: cancelled";
        return std::move(out);
    };

    if (bytes == nullptr && path.empty()) {
        out.error = "StepRead: empty path";
        return out;
    }
    if (cancel != nullptr && cancel->cancelled()) return bail_cancelled();

    // Registers the STEP norm AND its Interface_Static knobs. Must run before the
    // guard snapshots them, otherwise every knob reads as absent and the pinning
    // silently degrades to "whatever the process already held". Idempotent.
    // Process-global OCCT state is touched from here to the end of the call
    // (OcctStaticGuard.h); declared first so the guards restore under it.
    // Bounded: a wedged sibling in-process worker must not wedge this one.
    const BoundedLock occt_globals(occt_globals_mutex(), cancel);
    if (occt_globals.cancelled()) return bail_cancelled();
    if (!occt_globals.owns()) {
        out.error = occt_globals.busy_error("StepRead", "occt_globals_mutex");
        return out;
    }
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
        SourceSpanStream source(bytes ? *bytes : std::span<const std::uint8_t>{});
        const IFSelect_ReturnStatus status = bytes ? reader.ReadStream("captured.step", source)
                                                 : reader.ReadFile(path.c_str());
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
        // Identity is optional; avoid unbounded entity-by-root work on hostile assemblies.
        constexpr std::size_t kMaxIdentityEntityRootVisits = 10000000;
        const std::size_t root_visits = static_cast<std::size_t>(std::max(shape_count, 1));
        const bool identity_budget = static_cast<std::size_t>(reader.StepModel()->NbEntities()) <=
                                     kMaxIdentityEntityRootVisits / root_visits;
        const bool identity_policy = is_source_policy_v1(policy);
        if (bytes != nullptr && !identity_policy) {
            add_diag(out, "SOURCE_FACE_IDENTITY_PARTIAL", "nonstandard STEP read policy is not certified");
        }
        if (bytes != nullptr && !identity_budget) {
            add_diag(out, "SOURCE_FACE_IDENTITY_PARTIAL", "source identity scan budget exceeded");
        }
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
        bool identity_exhausted = false;

        for (int i = 1; i <= shape_count; ++i) {
            if (cancel != nullptr && cancel->cancelled()) return bail_cancelled();
            const TopoDS_Shape shape = reader.Shape(i);
            if (shape.IsNull()) {
                add_diag(out, step_diag::kRootSkipped,
                         "transferred shape " + std::to_string(i) + " is null");
                continue;
            }
            const auto original_faces = static_cast<std::size_t>(count_shape(shape).faces);
            const bool eligible = bytes != nullptr && identity_budget && identity_policy && !identity_exhausted;
            const bool admitted = eligible && out.source_unit.size() <= SourceClaimBatch::kMaxLabelBytes &&
                original_faces <= kMaxSourceFaceClaims - out.face_claims.size() &&
                out.face_claims.admit(original_faces);
            if (eligible && !admitted && original_faces != 0) {
                identity_exhausted = true;
                out.face_claims.clear();
                add_diag(out, "SOURCE_FACE_IDENTITY_PARTIAL", "source claim admission exhausted");
            }
            auto claims = admitted ? step_source_faces(reader, shape) : std::vector<SourceFaceClaim>{};
            const auto count_before = claims.size();
            const HealedRoot hr = heal_root(shape, policy, bytes != nullptr ? &claims : nullptr);
            if (bytes != nullptr) {
                retain_complete_source_faces(claims, hr.shape);
                if (count_before != original_faces || claims.size() != count_before || claims.empty()) {
                    add_diag(out, "SOURCE_FACE_IDENTITY_PARTIAL", "some source faces lack unique transfer/healing evidence");
                }
                if (claims.size() > kMaxSourceFaceClaims - out.face_claims.size()) {
                    out.face_claims.clear();
                    add_diag(out, "SOURCE_FACE_IDENTITY_PARTIAL", "source face claim budget exceeded");
                } else {
                    for (auto& claim : claims) {
                        if (!out.face_claims.append(std::move(claim))) {
                            out.face_claims.clear();
                            identity_exhausted = true;
                            add_diag(out, "SOURCE_FACE_IDENTITY_PARTIAL", "source claim bounds exceeded");
                            break;
                        }
                    }
                }
            }
            any_sewn = any_sewn || hr.sewn;
            any_healed = any_healed || hr.healed;
            any_restructured = any_restructured || hr.topology_changed;

            int shape_solids = 0;
            for (TopExp_Explorer se(hr.shape, TopAbs_SOLID); se.More(); se.Next()) ++shape_solids;
            if (shape_solids == 0) {
                add_diag(out, step_diag::kRootSkipped,
                         "transferred shape " + std::to_string(i) + " yielded no solid" +
                             (hr.open_shells > 0 ? " (open shell after healing)" : ""));
                continue;
            }
            // Kernel audit 2026-11-11 #15: a root that DOES yield solids used to
            // drop its surface geometry without a word. Import stays solids-only;
            // the loss is named by type and count.
            if (hr.open_shells > 0 || hr.free_faces > 0) {
                add_diag(out, step_diag::kGeometryOmitted,
                         "transferred shape " + std::to_string(i) + ": omitted " +
                             std::to_string(hr.open_shells) + " open shell(s) and " +
                             std::to_string(hr.free_faces) +
                             " free face(s); only solids are imported");
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
                out.face_claims.clear();
                return out;
            }
        }
        out.solids.reserve(ranked.size());
        for (const ops::RankedSolid& solid : ranked) out.solids.push_back(solid.shape);
        if (bytes != nullptr) {
            const std::size_t before = out.face_claims.size();
            retain_complete_source_faces(out.face_claims, all);
            TopTools_IndexedMapOfShape published_faces;
            std::vector<std::size_t> owners(1, 0);
            for (const auto& solid : out.solids) {
                TopTools_IndexedMapOfShape faces;
                TopExp::MapShapes(solid, TopAbs_FACE, faces);
                for (int i = 1; i <= faces.Extent(); ++i) {
                    const int index = published_faces.Add(faces(i));
                    if (static_cast<std::size_t>(index) >= owners.size()) owners.resize(index + 1, 0);
                    ++owners[index];
                }
            }
            out.face_claims.erase_if([&](const SourceFaceClaim& claim) {
                const int index = published_faces.FindIndex(claim.face);
                return index == 0 || owners[index] != 1;
            });
            if (out.face_claims.size() != before) {
                add_diag(out, "SOURCE_FACE_IDENTITY_PARTIAL",
                         "source faces do not have unique published-body ownership");
            }
        }
        if (bytes != nullptr && !out.face_claims.empty()) {
            const std::string digest = hashing::sha256_hex(bytes->data(), bytes->size());
            for (auto& claim : out.face_claims) {
                claim.source_sha256 = digest;
                claim.reader_policy = "step-v1";
                claim.kernel_fingerprint = policy::occt_fingerprint();
                claim.source_unit = out.source_unit;
                claim.target_unit = policy.target_unit;
            }
        }
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
                TopTools_IndexedMapOfShape invalid_faces;
                TopExp::MapShapes(out.solids[k], TopAbs_FACE, invalid_faces);
                const auto before = out.face_claims.size();
                out.face_claims.erase_if([&](const SourceFaceClaim& claim) {
                    return invalid_faces.FindIndex(claim.face) != 0;
                });
                if (out.face_claims.size() != before) {
                    add_diag(out, "SOURCE_FACE_IDENTITY_PARTIAL", "invalid solid source claims omitted");
                }
            }
        }
    } catch (const Standard_Failure& f) {
        // An aborted OCCT algorithm usually SURFACES as a Standard_Failure, so the
        // token has to be consulted before the exception is rendered as a failure.
        if (cancel != nullptr && cancel->cancelled()) return bail_cancelled();
        out.solids.clear();
        out.face_claims.clear();
        out.error = std::string("StepRead raised: ") +
                    (f.GetMessageString() != nullptr ? f.GetMessageString() : "OCCT failure");
        WLOG_ERROR("StepRead: %s", out.error->c_str());
        return out;
    } catch (const std::exception& e) {
        if (cancel != nullptr && cancel->cancelled()) return bail_cancelled();
        out.solids.clear();
        out.face_claims.clear();
        out.error = std::string("StepRead raised: ") + e.what();
        WLOG_ERROR("StepRead: %s", out.error->c_str());
        return out;
    }

    out.face_claims.release_empty();
    return out;
}

StepReadResult read_step(const std::string& path, const StepReadPolicy& policy,
                         const onecad::CancelToken* cancel) {
    return read_step_impl(path, nullptr, policy, cancel);
}

StepReadResult read_step_bytes(std::span<const std::uint8_t> bytes, const StepReadPolicy& policy,
                               const onecad::CancelToken* cancel) {
    return read_step_impl("<captured source>", &bytes, policy, cancel);
}

}  // namespace onecad::io
