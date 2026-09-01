// BrepCodec.h — the BinTools brep byte form shared by the §7.8 `InspectStep`
// conversion lane and the §7.3 `ImportStep` `sourceCodec:"brep"` replay lane.
//
// One header owns the FORMAT PIN so the producer and the consumer cannot drift:
// `InspectStep` reports `kBrepFormatVersion` as the `brepFormat` a document should
// record, and `ImportStep` validates a record's `brepFormat` against the same
// constant. The version is pinned to a LITERAL rather than tracking
// `BinTools_FormatVersion_CURRENT`, because CURRENT moves with the OCCT release
// and a document authored today must keep replaying byte-identically tomorrow.
//
// The compound's CHILD ORDER carries the §7.3 ordinal order. Nothing downstream
// re-sorts it (see ImportOp.h) — the order is baked in here, once, at conversion.
#pragma once

#include <cstdint>
#include <string>
#include <vector>

#include <TopoDS_Shape.hxx>

namespace onecad::io {

// BinTools_FormatVersion_VERSION_4 — the version this worker WRITES. Kept as an
// int (not the OCCT enum) because it crosses the wire as `brepFormat`.
inline constexpr int kBrepFormatVersion = 4;

// Serialize `solids`, in the given order, as ONE `TopoDS_Compound` in BinTools
// `kBrepFormatVersion` form. Triangulation and normals are excluded: the healed
// STEP solids carry none, and excluding them keeps the bytes a function of the
// exact geometry only. Returns "" on success, else the failure message.
std::string write_brep_compound(const std::vector<TopoDS_Shape>& solids,
                                std::vector<std::uint8_t>& bytes_out);

// Serialize ONE shape verbatim — NO compound wrapper — in the same BinTools
// `kBrepFormatVersion` form, with the same triangulation/normal exclusions.
// The §7.3 `source.kind:"profile"` blob is a single planar FACE; wrapping it
// would make every reader rely on the unwrap leniency instead of the contract.
std::string write_brep_shape(const TopoDS_Shape& shape, std::vector<std::uint8_t>& bytes_out);

struct BrepReadResult {
    // Direct children of the stored compound, IN STORED ORDER (never re-sorted).
    std::vector<TopoDS_Shape> solids;
    std::string error;  // "" on success

    bool ok() const { return error.empty(); }
};

// Deserialize a BinTools file written by `write_brep_compound`. Rejects — with a
// message, never an exception — bytes that are not BinTools, a null shape, a
// top-level shape that is neither a solid nor a compound, a compound carrying a
// non-solid child, and an empty result.
//
// The version banner is checked BEFORE OCCT sees the bytes: `BinTools_ShapeSet`
// reports a version mismatch by printing to `std::cout` directly, which is the
// worker's frame channel (see the rationale in the .cpp).
BrepReadResult read_brep_solids(const std::string& path);

struct BrepShapeResult {
    TopoDS_Shape shape;  // null on failure
    std::string error;   // "" on success

    bool ok() const { return error.empty(); }
};

// Deserialize a BinTools file as ONE shape, with NO shape-kind policy of its
// own — that question belongs to the caller, which owns the refusal that names
// it (§7.3 `profile` wants a planar face and answers `PROFILE_BLOB_NOT_ONE_FACE`).
//
// A compound carrying EXACTLY ONE child is unwrapped to that child — the same
// leniency `read_brep_solids` grants a producer that skipped the wrapper, in the
// other direction. A compound with any other child count is returned AS the
// compound, so the caller refuses it by name instead of receiving a silently
// chosen first child.
//
// Header-checked before OCCT sees the bytes, for the `read_brep_solids` reason.
BrepShapeResult read_brep_shape(const std::string& path);

}  // namespace onecad::io
