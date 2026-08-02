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

}  // namespace onecad::io
