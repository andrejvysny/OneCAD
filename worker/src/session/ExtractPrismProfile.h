// ExtractPrismProfile.h — the §7.8 `ExtractPrismProfile` verb: is this body a
// prism, and if so what is its canonical end-cap profile?
//
// The worker half of the Component Library's "vendor a STEP extrusion" ingest
// (§7.3 `source.kind:"profile"`). Vendor stock arrives as ONE fixed length — an
// aluminium extrusion STEP is a 500 mm stick — while the component has to be
// placeable at any length, so the ingest tool has to recover the CROSS-SECTION
// from the stick and hand it back as the exact byte form a `profile` source
// reads: one planar face on `z = 0`, normal `+Z`, area centroid at the origin.
//
// It does not fence-and-clone, prepare, accept, discard, publish or MINT: no
// bodyEvents, no elementMapDelta, no scratch, no snapshot, no bin tail.
// `GetWorkerHead` is byte-identical before and after. "Read-only" is with
// respect to SESSION state — it writes a FILE, exactly as the other §7.8 verbs
// do, which is what lets the digest ride the JSON envelope where a golden
// fixture can assert it.
//
// SNAPSHOT-FENCED like `PrepareOffsetFace` and for the same reason: the answer
// is frozen into a package, so an answer against a superseded head would freeze
// the ordinals of a shape that no longer exists. Stale ⇒ `STALE_PREVIEW`.
//
// REFUSING IS AN ANSWER (`ok:true`, `refusal.code:"notAPrism"`), carrying the
// measured `volumeRatio` — a cross-drilled or tapered stick refuses with a
// number, which is what lets an ingest UI explain itself. An unknown `bodyId` is
// a `REF_UNRESOLVED` *error*, matching `QueryBodyTopology`: a `BodyId` is
// persistent, so a miss is a real resolve failure and not a stale reference.
//
// Determinism: same head + same request ⇒ byte-identical response AND written
// bytes across fresh processes. Every walk is `TopExp` ordinal order; the
// in-plane canonical frame is tie-broken by a rule that reads only the SHAPE
// (see `bake_canonical_profile`), never a raw eigenvector sign or the source's
// world pose — either of those would make the blob a function of the input
// file's orientation and silently break content-address dedup for two ingests
// of the same physical profile.
#pragma once

#include <cstdint>
#include <string>
#include <vector>

#include <TopoDS_Shape.hxx>
#include <gp_Dir.hxx>

#include "protocol/Envelope.h"
#include "session/Session.h"

namespace onecad::session {

// The measured answer. `is_prism` false ⇒ `refusal` carries the human message
// and `volume_ratio` the number behind it; every other field is meaningless.
struct PrismAnalysis {
    bool is_prism = false;
    gp_Dir axis{0.0, 0.0, 1.0};
    int end_cap_ordinal = 0;  // 1-based `TopExp` face ordinal of the axis-MINIMUM cap
    double length_mm = 0.0;   // PLANE-TO-PLANE, never a bounding-box extent
    double area_mm2 = 0.0;
    double volume_ratio = 0.0;
    double area_delta = 0.0;  // |areaMin − areaMax| / max, relative
    int outer_edge_count = 0;
    int inner_wire_count = 0;
    std::string refusal;  // "" iff is_prism
};

// Decide prism-ness along `axis_hint` (null ⇒ the longest axis-aligned
// bounding-box dimension, ties X→Y→Z). THREE tests must pass: no
// axis-perpendicular planar face lies strictly between the two end caps,
// `|volume − endCapArea·length| ≤ 1e-6·volume`, and the two caps' areas agree to
// 1e-6 relative. The structural test is the load-bearing one — two sticks of
// EQUAL section area fused end to end (20x20 onto 40x10) pass both measurements
// and are not a prism; only the step face at the junction says so.
// A hint must already be non-degenerate (the handler validates).
PrismAnalysis analyze_prism(const TopoDS_Shape& body, const gp_Dir* axis_hint);

// Rigidly transform the analysed end cap into the canonical profile frame and
// serialize it in the `brep` replay codec. Returns "" on success (`bytes_out`
// filled), else the failure message. Only meaningful on a positive analysis.
std::string bake_canonical_profile(const TopoDS_Shape& body, const PrismAnalysis& analysis,
                                   std::vector<std::uint8_t>& bytes_out);

protocol::Envelope handle_extract_prism_profile(Session& session, const protocol::Envelope& req);

}  // namespace onecad::session
