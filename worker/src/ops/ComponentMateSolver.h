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
// `seat_anchor` is the world point used to pick a seat along an infinite
// axis / on an infinite plane. The LIVE interactive gesture (WP-1.5) uses
// the cursor hit point; a REGEN re-seat (WP-3.1, no cursor exists) instead
// passes the component's own CURRENT frozen `placement.translate` — this
// preserves how far along the axis / where on the plane the user originally
// seated it as the target moves, rather than snapping to the frame's raw
// origin every tick. (`concentricAndCoincident` ignores the anchor
// entirely either way — spec's hole-rim snap seats at the hole's own
// origin by construction.)
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
                                                    bool flipped);

}  // namespace onecad::ops
