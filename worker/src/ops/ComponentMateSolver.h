// ComponentMateSolver.h — pure candidate-transform math for a resolved mate
// (Component Library P3 WP-3.1, spec §5.5). Ported from
// `src/modules/library/placementSolver.ts` (WP-1.5's interactive placement
// gesture) — same algorithm, gp_-free (plain arrays), so the two stay
// numerically comparable (this port's tests cite `placementSolver.test.ts`'s
// own cases as the parity oracle).
//
// COMPONENT-LOCAL CONVENTION (matches the TS file's own header, and the v1
// generator's local frame — `ComponentOp.cpp`'s SHCS build): origin at the
// seating plane, head above in local +Z, shank below in local -Z. So
// `shankAxis` attachments align local `(0,0,1)` to the target's axis
// direction; `headSeat` (plane) attachments align local `(0,0,1)` to the
// target face's OUTWARD normal. A future non-fastener generator with a
// different local convention needs its own solver branch — this is honestly
// scoped to what the seed catalog's fastener families need, not
// generator-agnostic.
#pragma once

#include <array>
#include <optional>
#include <string>

#include "nlohmann/json.hpp"

namespace onecad::ops {

// `frame` is a `session::classify_shape`-shaped JSON object: `{origin,
// normal}` for a plane, `{origin,axis,radius}` for a cylinder/circle.
// `snap_kind` is `"concentric"|"coincident"|"concentricAndCoincident"`
// (`ComponentMate.kind`'s wire form, spec §5.3's table).
//
// `seat_anchor` is the world SEAT point used to pick a seat along an infinite
// axis / on an infinite plane. The LIVE interactive gesture (WP-1.5) uses
// the cursor hit point; a REGEN re-seat (WP-3.1, no cursor exists) instead
// passes the component's own CURRENT frozen seat — this preserves how far
// along the axis / where on the plane the user originally seated it as the
// target moves, rather than snapping to the frame's raw origin every tick.
// With a `self_frame` that current seat is where the ATTACHMENT sits
// (`placement` applied to `self_frame.origin`), NOT `placement.translate`;
// see `ComponentOp.cpp::resolve_mate_reseat`, which owns that correction and
// the drift it prevents. (`concentricAndCoincident` ignores the anchor
// entirely either way — spec's hole-rim snap seats at the hole's own
// origin by construction.)
//
// `self_frame` is the component's OWN local seating basis, `mate.selfFrame`
// verbatim (SCHEMA §7.3; Component Library WP-F1.1): `{origin:[x,y,z],
// z:[x,y,z], x:[x,y,z]}`, already orthonormal (Rust's manifest parse
// normalizes it before it is frozen into the record). Anything that is not
// such an object — including the ABSENT case, which is every pre-WP-F1.1
// document — is the identity frame, and takes an early return through the
// exact pre-WP-F1.1 code path so those placements stay byte-identical.
//
// The composition is `M = S ∘ F⁻¹`, where `S` is the seat transform solved
// from (`frame`, `snap_kind`, `flipped`) below and `F` maps component-local
// identity onto the attachment frame (rotation columns `[x, z×x, z]`,
// translation `origin`). That is exactly what makes the attachment point —
// not the model origin — land on the target seat with its local `z` along the
// target axis/normal.
//
// Returns a `FrozenPlacement`-shaped JSON object (`{translate:[x,y,z],
// rotate:{center:[0,0,0],axis:[x,y,z],angleDeg}}`, directly consumable by
// `read_placement`), or `std::nullopt` if `frame` lacks what `snap_kind`
// needs (no normal/axis, or a degenerate near-zero one) — the caller treats
// that exactly like an ambiguous ladder outcome: `NeedsRepair`, never a
// guess.
std::optional<nlohmann::json> solve_mate_placement(const std::string& snap_kind,
                                                    const nlohmann::json& frame,
                                                    const std::array<double, 3>& seat_anchor,
                                                    bool flipped,
                                                    const nlohmann::json& self_frame = nlohmann::json());

}  // namespace onecad::ops
