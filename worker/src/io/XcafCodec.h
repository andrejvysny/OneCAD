// XcafCodec.h — the BinXCAF (`.xbf`) byte form shared by the §7.8 `InspectStep`
// conversion lane and the §7.3 `ImportStep` `sourceCodec:"xbf"` replay lane.
//
// ── Why BinXCAF and not BinTools ────────────────────────────────────────────
// `io/BrepCodec` (BinTools, `sourceCodec:"brep"`) round-trips TOPOLOGY ONLY. A
// STEP file's product names and face colors live in XCAF attributes, which a brep
// dump has nowhere to put — so a brep-replay document silently lost every imported
// color and name on reopen. BinXCAF is OCCT's binary OCAF form: it stores the
// shapes AND the `XCAFDoc_ShapeTool` / `XCAFDoc_ColorTool` attributes over them, so
// a save→reopen replay reproduces the appearance the user imported. `brep` stays a
// valid §7.3 codec for documents already authored against it (SCHEMA §7.8).
//
// ── The format pin ──────────────────────────────────────────────────────────
// One header owns it, exactly like BrepCodec: `InspectStep` reports
// `kXcafFormatVersion` as the `geometryFormat` a document should record, and
// `ImportStep` validates a record's `brepFormat` against the same constant. Pinned
// to a LITERAL (not `TDocStd_FormatVersion_CURRENT`) because CURRENT moves with the
// OCCT release and a document authored today must keep replaying tomorrow.
//
// ── Canonical content ───────────────────────────────────────────────────────
// The document written here is BUILT EXPLICITLY, never handed straight through
// from the STEP reader's own label tree: one free top-level label per ordered
// solid, in the §7.3 ordinal order, each carrying its product name and its
// per-face colors. So the xbf content is a canonical rendering of what the import
// publishes rather than whatever intermediate assembly structure the reader
// happened to build, and the read side can iterate it as a flat ordered list.
//
// ── Determinism ─────────────────────────────────────────────────────────────
// `TDocStd_Application::SaveAs` over the same document content is byte-stable (no
// timestamp / user / path is embedded), which matters because Rust
// content-addresses the blob by sha256 — a per-run-varying blob would make
// re-importing one file look like a different operation every time.
#pragma once

#include <cstdint>
#include <string>
#include <vector>

#include <TopoDS_Shape.hxx>

class Quantity_ColorRGBA;

namespace onecad::io {

// TDocStd_FormatVersion_VERSION_12 — the OCAF storage version this worker WRITES.
// Kept as an int (not the OCCT enum) because it crosses the wire as
// `geometryFormat` / `ImportStep.brepFormat`.
inline constexpr int kXcafFormatVersion = 12;

// The `sourceCodec` / `geometryCodec` string and the container extension.
inline constexpr const char* kXcafCodecName = "xbf";

// One face color, packed `r<<24 | g<<16 | b<<8 | a`, components u8 **sRGB** — the
// exact byte order the MESH1 `FACE_COLORS` section stores (mesh_format.md §4).
using PackedColor = std::uint32_t;

// Alpha 0 means "no authored color": the renderer falls back to the body material
// (mesh_format.md §4 `{0,0,0,0}`). Chosen over a sentinel index so a color vector
// is a dense per-face array with no side table.
inline constexpr PackedColor kUnsetColor = 0;

// Linear `Quantity_ColorRGBA` (OCCT's internal space) → packed u8 sRGB, using
// OCCT's own `Convert_LinearRGB_To_sRGB` — the same transform `Quantity_Color::
// ColorToHex` applies, so a color reports here as OCCT's own viewer would show it.
PackedColor pack_srgb(const Quantity_ColorRGBA& color);

// The authored appearance of ONE solid.
struct SolidAttributes {
    // STEP product name, "" when none was recoverable. Advisory evidence, never
    // identity (SCHEMA §7.3).
    std::string name;
    // Per face, in `TopExp::MapShapes(solid, TopAbs_FACE)` order of the solid it
    // describes — the SAME order `tess::tessellate_body` walks, which is what makes
    // the MESH1 `FACE_COLORS` section index-aligned with `FACE_RANGES`. EMPTY when
    // no face of the solid carries a color (the common case); otherwise sized to
    // the face count with `kUnsetColor` in the gaps.
    std::vector<PackedColor> face_colors;

    bool empty() const { return name.empty() && face_colors.empty(); }
};

// Serialize `solids`, in the given order, as one BinXCAF document. `attrs` is
// index-aligned with `solids`; a shorter/empty vector means "no attributes".
// Returns "" on success, else the failure message.
std::string write_xcaf_document(const std::vector<TopoDS_Shape>& solids,
                                const std::vector<SolidAttributes>& attrs,
                                std::vector<std::uint8_t>& bytes_out);

struct XcafReadResult {
    // Free top-level shapes IN STORED ORDER (label tag order == the write order,
    // never re-sorted — see ImportOp.h on why re-deriving the ordinals would be a
    // silent re-numbering).
    std::vector<TopoDS_Shape> solids;
    // 1:1 with `solids`.
    std::vector<SolidAttributes> attributes;
    std::string error;  // "" on success

    bool ok() const { return error.empty(); }
};

// Deserialize a BinXCAF document written by `write_xcaf_document`. Rejects — with a
// message, never an exception — bytes that are not an OCAF binary file, a document
// that carries no free shape, and a free shape that is not a solid.
//
// The `BINFILE` magic is checked BEFORE OCCT sees the bytes, for the same stdout
// hygiene reason BrepCodec checks the BinTools banner (see the .cpp).
XcafReadResult read_xcaf_solids(const std::string& path);

}  // namespace onecad::io
