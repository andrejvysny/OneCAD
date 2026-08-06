// PrepareOffsetFace.h — the read-only `OffsetFace` authoring handshake (SCHEMA §7.6).
//
// The FIRST half of the `op.offsetFace` freeze. Given the user's picked faces it
// computes, on a COPY of the head:
//   * the G1 tangent-chain closure (`BRepLib::ContinuityOfFaces >= GeomAbs_G1` at
//     ≈1e-4 rad, BFS over 2-manifold edges) — the FULL operative set the record will
//     freeze, because the kernel auto-propagates an offset across tangent junctions
//     and cannot hold a tangent neighbour fixed (Phase-0 spike);
//   * the `Total` opposite-face candidate, FAIL CLOSED (anti-parallel plane at a
//     positive inward distance, whose translated footprint covers the ENTIRE selected
//     face, validated by a material-column boolean; exactly one passing candidate);
//   * `currentDims` — the measured radius / thickness that seed the absolute
//     distance types in the UI.
//
// It does NOT fence the head, prepare, accept, discard, or — critically — MINT.
// The response carries snapshot-scoped TopoKeys plus descriptor/anchor EVIDENCE only,
// never `ElementId`s (Invariant 2: Rust is the sole minting authority). Rust promotes
// the evidence through the `AcquireElementIds` path and authors the final params.
//
// UNLIKE the advisory §7.5 reads this verb IS SNAPSHOT-FENCED: its answer is about to
// be FROZEN into a document record, so an answer against a superseded head would
// freeze ordinals of a shape that no longer exists. A stale `snapshotId` is refused
// with `STALE_PREVIEW` (SCHEMA §7.6 / §8).
//
// REFUSALS ARE ANSWERS (`ok:true`, `result.refusal` non-null), never errors — the
// frontend renders them as a hint. Codes (SCHEMA §7.6): `crossBody`,
// `unsupportedSurface`, `chainMismatch`, `noUniqueOpposite`, `notPlanar`,
// `chainOnTotal`, `notCylindrical`, `mixedAxis`, `nonManifold`.
//   * `notPlanar` also covers "Total selected on something other than exactly ONE
//     planar face" — the code set is closed on the wire, and a multi-face Total is a
//     "the operative set is not a single plane" condition; the message names the count.
// A pick that cannot be resolved at all is a REF_UNRESOLVED *error* (it is a stale
// address, not a geometric refusal) — the `AcquireElementIds` convention.
//
// Determinism: identical head + identical request ⇒ byte-identical response. Every
// list is ordered by FACE ORDINAL (`TopExp::MapShapes`), never by pointer or hash.
#pragma once

#include "protocol/Envelope.h"
#include "session/Session.h"

namespace onecad::session {

protocol::Envelope handle_prepare_offset_face(Session& session, const protocol::Envelope& req);

}  // namespace onecad::session
