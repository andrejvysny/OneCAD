// ComponentGenerators.h — the built-in, versioned parametric component
// generators (Component Library spec §6, WP-A1).
//
// One entry point, `build_component`, dispatched on the `generatorId` a
// `PlaceComponent`/`DetachComponent` record carries in `source.generatorId`.
// Before WP-A1 there was no dispatch at all: EVERY generatorId built an ISO
// 4762 socket cap screw, which is exactly the silent-substitution failure
// spec §0 invariant 4 exists to prevent. The catalog is back to that one
// family, but the DISPATCH stays — an unknown id fails loudly, naming the ids
// that do exist, instead of quietly handing back a screw.
//
// Geometry conventions every family must follow — these ARE the mate contract,
// because the placement solver seats a component by its local origin and
// local +Z (see `ComponentMateSolver.h`; `mate.selfAttachment` is not yet read
// worker-side):
//
//   * The origin sits on the part's SEATING PLANE, +Z pointing out of the
//     surface the part seats against.
//   * A screw's head occupies z ∈ [0, k] and its shank z ∈ [-length, 0], so
//     dropping it on a hole rim drives the shank into the hole.
//   * A part that sits ON a surface rather than through it (a nut, a washer)
//     occupies z ∈ [0, thickness].
//
// Dimensions come from `FastenerTables.h` and nothing here hardcodes one.
#ifndef ONECAD_OPS_COMPONENTGENERATORS_H
#define ONECAD_OPS_COMPONENTGENERATORS_H

#include <map>
#include <optional>
#include <string>

#include <TopoDS_Shape.hxx>

#include "util/Cancel.h"

namespace onecad::ops {

/// Thread rendering level (spec §6.4). `Cosmetic` is the mainstream-CAD
/// default (a plain cylinder at the nominal Ø — assembly-safe, fast);
/// `Simplified` cuts discrete annular grooves; `Modeled` cuts a true helical
/// V-thread, opt-in for 3D-print output. Families with no external thread
/// (nuts, washers) ignore it.
enum class ThreadDetail { Cosmetic, Simplified, Modeled };

bool parse_thread_detail(const std::string& s, ThreadDetail& out);

/// One generator invocation. `length_mm` is the free `length` param and is
/// ignored by families whose geometry has no length (nuts, washers) — a
/// package for those simply never declares the param. `length_given` says
/// whether the caller actually SENT it, so a family whose own default is not
/// the wire default (a motor's body length) can apply its own instead of
/// silently inheriting a screw's 20 mm.
///
/// `text_params` carries every STRING param under `source.params` verbatim,
/// including `thread` (which `thread` above is read out of, so the two can
/// never disagree). No current generator reads anything else out of it, and it
/// is kept populated on purpose: a family not keyed by a thread designation
/// reads its own key from here, so adding one needs no change to the op layer.
struct GeneratorRequest {
    std::string op_label;
    std::string thread;
    double length_mm;
    bool length_given;
    ThreadDetail detail;
    std::map<std::string, std::string> text_params;
    /// Cooperative cancel, polled per thread RING while the simplified cutter
    /// accumulates its tool compound (kernel-hardening WP-I). Null ⇒ no cancel
    /// lane, which is what every in-process test uses.
    const onecad::CancelToken* cancel = nullptr;
};

/// A generator free param outside the bounds SCHEMA §7.3 sets for it
/// (kernel-hardening WP-I). The worker refuses BY NAME with the offending value
/// before any geometry is built; the op layer turns this into an `OP_FAILED`
/// whose `detail.diagnostics[0]` carries `reasonCode`
/// `GENERATOR_PARAM_OUT_OF_RANGE` and `evidence {param, value, min, max}`.
struct GeneratorBoundViolation {
    std::string param;   ///< "length" | "turns"
    double value = 0.0;
    double min = 0.0;
    double max = 0.0;
    std::string message;  ///< human-facing, names the parameter and the value
};

/// The WP-I bounds preflight for `generator_id`'s free params. `nullopt` ⇒ in
/// range (or a family that reads none — today only `iso4762` reads `length`, so
/// every other id answers `nullopt` and its own dispatch failure stands).
///
/// Two bounds, both from SCHEMA §7.3: `length ∈ (0, 1000]` mm, and — only for
/// the `simplified` / `modeled` details, the two that actually cut thread rings
/// — `floor(length / pitch) ≤ 500` turns, `pitch` being the family TABLE's value
/// for the size rather than a wire param. An unknown thread designation is NOT
/// a bound violation: the builder's own by-name refusal (which lists the known
/// sizes) is the better message, so this returns `nullopt` and lets it run.
std::optional<GeneratorBoundViolation> generator_bounds_violation(const std::string& generator_id,
                                                                  const GeneratorRequest& req);

/// Builds `generator_id`'s solid for `req`. Never throws: every failure —
/// unknown generator, unknown thread designation, OCCT refusal — comes back
/// `false` with a message the caller surfaces as a recoverable `OP_FAILED`.
bool build_component(const std::string& generator_id, const GeneratorRequest& req,
                     TopoDS_Shape& solid_out, std::string& err);

/// `"iso4762"` — the registered ids, comma-separated, for the unknown-id
/// message and for tests that assert the seed catalog's shape.
std::string known_generator_ids();

}  // namespace onecad::ops

#endif  // ONECAD_OPS_COMPONENTGENERATORS_H
