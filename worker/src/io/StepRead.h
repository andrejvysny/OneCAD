// StepRead.h — pure, deterministic STEP → ordered solids (STEP-IMPORT WP-A W0).
//
// `read_step()` is a FREE FUNCTION over a filesystem path. It owns no session,
// registers no verb, mints no BodyId and touches no dispatcher — W0 deliberately
// stops at the kernel boundary so the codec decision (step-replay vs
// brep-replay) can be made against measured evidence before any wire surface is
// committed. Verb wiring is W1.
//
// ── Determinism contract ────────────────────────────────────────────────────
// Two reads of the same bytes, in the same process or in two processes, MUST
// produce the same solids in the same order. Three things buy that:
//   * the read policy is a FIXED knob set applied unconditionally (never
//     "whatever Interface_Static happened to hold"), so a prior export or a
//     prior import cannot bias a later read;
//   * the pipeline stages are unconditional and fixed-order — sew, then heal,
//     then promote — so a shape never takes a different route because an earlier
//     stage happened to succeed;
//   * the output order is `ops::ordered_solids` (quantized volume / centroid /
//     face-count), never unordered `TopExp` iteration, which reflects OCCT's
//     internal map layout and therefore the allocator.
//
// ── Failure model ───────────────────────────────────────────────────────────
// A `Standard_Failure` anywhere in the pipeline is caught and rendered as
// `error`; the function never terminates and never leaks an OCCT exception to
// its caller. `error == nullopt` means the read completed — it does NOT mean the
// file contained solids; a valid STEP file with no solid yields an empty
// `solids` plus a `STEP_NO_SOLIDS` diagnostic.
#pragma once

#include <optional>
#include <string>
#include <vector>

#include <TopoDS_Shape.hxx>

#include "util/Cancel.h"

namespace onecad::io {

// Diagnostic codes. Advisory: none of these is an error, they describe what the
// pipeline had to do to the incoming geometry (evidence for the codec decision —
// a file that needs healing cannot be replayed from its STEP bytes for free).
namespace step_diag {
inline constexpr const char* kSewn = "STEP_SEWN";                    // faces were stitched
inline constexpr const char* kHealed = "STEP_HEALED";                // ShapeFix_Shape changed it
inline constexpr const char* kUnitConverted = "STEP_UNIT_CONVERTED";  // file unit != mm
inline constexpr const char* kNoSolids = "STEP_NO_SOLIDS";           // nothing solid in the file
inline constexpr const char* kRootSkipped = "STEP_ROOT_SKIPPED";     // a root yielded no solid
inline constexpr const char* kInvalidShape = "STEP_INVALID_SHAPE";   // BRepCheck rejected a solid
}  // namespace step_diag

inline constexpr const char* kAmbiguousImportOrder = "AMBIGUOUS_IMPORT_ORDER";

// v1 read policy — the FIXED knob set. The struct exists so a later work package
// can vary one value at a call site without rewriting the pipeline; W0 uses the
// defaults everywhere and nothing reads a knob it has not pinned here.
struct StepReadPolicy {
    // `xstep.cascade.unit`: the unit OCCT converts the file INTO. The whole stack
    // is millimetres (corpus + SCHEMA numerics), so this is pinned, not inherited.
    std::string target_unit = "MM";
    // `read.precision.mode`: 0 = use the uncertainty declared by the file itself
    // (1 = force `precision_val`). 0 keeps the read a function of the file.
    int precision_mode = 0;
    // `read.precision.val`: only consulted when `precision_mode == 1`; pinned so
    // a leaked value from another reader cannot change a mode-1 read.
    double precision_val = 1.0e-4;
    // `read.step.product.mode`: 1 = translate the product structure (assembly
    // names / roots stay distinguishable). 0 would flatten roots and cost the
    // per-root product names.
    int product_mode = 1;
    // `read.maxprecision.mode` / `.val`: OCCT defaults, pinned. Mode 0
    // ("Preferred") caps healing tolerance at `max_precision_val`.
    int max_precision_mode = 0;
    double max_precision_val = 1.0;
    // Floor for the sewing tolerance derived from the file's own precision. A
    // file that declares an absurdly small uncertainty must not make sewing a
    // no-op; a file that declares none must not make it 0.
    double min_sew_tolerance = 1.0e-6;
};

struct StepReadDiagnostic {
    std::string code;     // one of step_diag::*
    std::string message;  // human-readable detail; NOT machine-parsed
};

struct StepReadResult {
    // Solids in `ops::ordered_solids` order. Empty is a legal success.
    std::vector<TopoDS_Shape> solids;
    // STEP product name per TRANSFER ROOT, in root order — best effort: an entry
    // is "" when the root carries no recoverable product, and the whole vector is
    // empty when the reader exposes no roots. Roots are NOT 1:1 with `solids`
    // (a root may yield several solids, or none); W0 does not correlate them.
    std::vector<std::string> product_names;
    std::vector<StepReadDiagnostic> diagnostics;
    // The file's FIRST declared length unit, uppercased ("MM", "INCH", …), or
    // "MM" when the file declares none (OCCT's own assumption). The three
    // millimetre spellings (MM / MILLIMETRE / MILLIMETER) collapse to "MM" so
    // "declared mm" and "undeclared" report identically. Advisory metadata for
    // the §7.8 `InspectStep` probe — the geometry is ALWAYS already converted to
    // `policy.target_unit`, so this never scales anything. W1.
    std::string source_unit = "MM";
    // nullopt on success. Set for: unreadable/garbage file, a failed transfer, or
    // any Standard_Failure escaping a pipeline stage.
    std::optional<std::string> error;
    // True when the caller's cancel token fired during the read; `error` is set
    // too, so `ok()` stays false, but the caller must map this to CANCELLED
    // rather than OP_FAILED (SCHEMA §8 — different session semantics). W1.
    bool cancelled = false;

    bool ok() const { return !error.has_value(); }
};

// Read `path` under `policy`. Pure: no session state, no globals left mutated
// (every `Interface_Static` knob touched is restored — see OcctStaticGuard.h).
//
// `cancel` (optional, W1) is consulted between pipeline stages AND inside
// `TransferRoots` via `ops::CancelProgress`, so a large assembly aborts promptly
// instead of running to completion. A fired token yields `cancelled == true`.
StepReadResult read_step(const std::string& path, const StepReadPolicy& policy = StepReadPolicy{},
                         const onecad::CancelToken* cancel = nullptr);

}  // namespace onecad::io
