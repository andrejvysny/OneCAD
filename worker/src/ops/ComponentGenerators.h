// ComponentGenerators.h — the built-in, versioned parametric component
// generators (Component Library spec §6, WP-A1).
//
// One entry point, `build_component`, dispatched on the `generatorId` a
// `PlaceComponent`/`DetachComponent` record carries in `source.generatorId`.
// Before WP-A1 there was no dispatch at all: EVERY generatorId built an ISO
// 4762 socket cap screw, which is exactly the silent-substitution failure
// spec §0 invariant 4 exists to prevent once a second family is seeded. An
// unknown id now fails loudly, naming the ids that do exist.
//
// Geometry conventions shared by every family — these ARE the mate contract,
// because the placement solver seats a component by its local origin and
// local +Z (see `ComponentMateSolver.h`; `mate.selfAttachment` is not yet read
// worker-side):
//
//   * The origin sits on the part's SEATING PLANE, +Z pointing out of the
//     surface the part seats against.
//   * A screw's head occupies z ∈ [0, k] and its shank z ∈ [-length, 0], so
//     dropping it on a hole rim drives the shank into the hole.
//   * A nut/washer occupies z ∈ [0, thickness] — it sits ON the surface.
//   * A bearing occupies z ∈ [0, width] — a ring sits ON the surface, same as
//     a washer.
//   * A motor's faceplate is the seating plane: the body occupies
//     z ∈ [0, body length] (it sits ON the plate, like a nut) while the pilot
//     boss and shaft run z < 0 (they drive INTO the plate's pilot hole, like
//     a screw's shank).
//
// Dimensions come from `FastenerTables.h` / `MachineElementTables.h` and
// nothing here hardcodes one.
#ifndef ONECAD_OPS_COMPONENTGENERATORS_H
#define ONECAD_OPS_COMPONENTGENERATORS_H

#include <map>
#include <string>

#include <TopoDS_Shape.hxx>

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
/// never disagree). Families not keyed by a thread designation — a bearing's
/// `code` — read their own key from here, which is why adding one needs no
/// change to the op layer.
struct GeneratorRequest {
    std::string op_label;
    std::string thread;
    double length_mm;
    bool length_given;
    ThreadDetail detail;
    std::map<std::string, std::string> text_params;
};

/// Builds `generator_id`'s solid for `req`. Never throws: every failure —
/// unknown generator, unknown thread designation, OCCT refusal — comes back
/// `false` with a message the caller surfaces as a recoverable `OP_FAILED`.
bool build_component(const std::string& generator_id, const GeneratorRequest& req,
                     TopoDS_Shape& solid_out, std::string& err);

/// `"iso15, iso4014, …"` — the registered ids, for the unknown-id message
/// and for tests that assert the seed catalog's shape.
std::string known_generator_ids();

}  // namespace onecad::ops

#endif  // ONECAD_OPS_COMPONENTGENERATORS_H
