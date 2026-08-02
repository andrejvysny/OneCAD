// XcafRead.h — the XCAF ATTRIBUTE pass over a STEP file (WP-A W4.5).
//
// `io::read_step` (W0) is the geometry authority and is deliberately left alone:
// its knob set, its pipeline order and its `ops::ordered_solids` output are pinned
// by the determinism contract in StepRead.h and by the cross-process digest tests,
// and `STEPControl_Reader` is what those pins were measured against. This module
// runs a SECOND, attribute-only pass with `STEPCAFControl_Reader` and hands back
// the product names + face colors the plain reader has nowhere to put.
//
// ── Why two passes, and how the two are correlated ──────────────────────────
// The obvious alternative — one `STEPCAFControl_Reader` feeding both geometry and
// attributes — would move the geometry lane onto a different reader class than the
// one every determinism fixture was captured against. Not worth it for advisory
// appearance data.
//
// Two passes means TShape identity cannot correlate them (each reader allocates its
// own topology), so the correlation is GEOMETRIC: a face is keyed by its quantized
// (area, centroid) and a solid by its quantized (volume, centroid) — the same 1e-6
// quantization `ops::ordered_solids` and the ElementMap descriptors already use.
// Consequences, all deliberate:
//
//   * A file whose healing is tolerance-level (the overwhelming majority) matches
//     every face: sewing/`ShapeFix_Shape` leave areas and centroids untouched even
//     when they rebuild the underlying `TFace`. Identity matching would LOSE those.
//   * Where healing genuinely RESTRUCTURED the topology (a face split, merged or
//     dropped), the replacement has no key and simply carries NO color — reported
//     as `kUnsetColor`, rendered with the body material. Colors are appearance, not
//     identity; a missing one is a cosmetic loss, never a wrong bind.
//   * An AMBIGUOUS key — two faces with the same quantized area and centroid but
//     different colors — is DROPPED rather than guessed. Coincident geometry must
//     not silently take a neighbour's appearance.
//
// `bind_attributes` reports `faces_total` / `faces_matched` so a caller (and the
// tests) can see the match rate rather than infer it.
//
// ── Failure model ───────────────────────────────────────────────────────────
// This pass is ADVISORY. A failure yields an empty `StepAttributes` with `error`
// set; the caller keeps its geometry and publishes unnamed, uncolored solids. It
// never turns a readable STEP file into a failed probe.
#pragma once

#include <cstdint>
#include <map>
#include <string>
#include <vector>

#include <TopoDS_Shape.hxx>

#include "io/StepRead.h"
#include "io/XcafCodec.h"
#include "util/Cancel.h"

namespace onecad::io {

// Quantized geometric identity of one shape: (measure, cx, cy, cz) where `measure`
// is surface area for a face and volume for a solid. `long long` at 1e-6, never a
// raw double — a float-keyed map is a comparison of rounding modes.
struct GeomKey {
    long long measure = 0;
    long long cx = 0;
    long long cy = 0;
    long long cz = 0;

    bool operator<(const GeomKey& o) const {
        if (measure != o.measure) return measure < o.measure;
        if (cx != o.cx) return cx < o.cx;
        if (cy != o.cy) return cy < o.cy;
        return cz < o.cz;
    }
};

// `GeomKey` of a FACE (surface area + centroid) / of a SOLID (volume + centroid).
GeomKey face_key(const TopoDS_Shape& face);
GeomKey solid_key(const TopoDS_Shape& solid);

// The attribute-only read result, keyed geometrically so it can be correlated with
// a `read_step` result the caller already holds.
struct StepAttributes {
    // Face key → packed sRGB color. Ambiguous keys are absent (see the header docs).
    std::map<GeomKey, PackedColor> face_colors;
    // Solid key → product name. Ambiguous keys are absent.
    std::map<GeomKey, std::string> solid_names;
    // "" on success. Non-empty means the pass produced nothing usable; the maps are
    // then empty and the caller degrades to unnamed/uncolored.
    std::string error;
};

// Run the attribute pass over `path` under the SAME `Interface_Static` policy
// `read_step` pins, so both passes see the same unit conversion and precision.
// `cancel` (optional) aborts the transfer through `ops::CancelProgress`.
StepAttributes read_step_attributes(const std::string& path,
                                    const StepReadPolicy& policy = StepReadPolicy{},
                                    const onecad::CancelToken* cancel = nullptr);

// The outcome of correlating `attrs` onto an ordered solid vector.
struct BoundAttributes {
    // 1:1 with the solids handed in, in the same order.
    std::vector<SolidAttributes> per_solid;
    // Correlation evidence (tests + the stderr log). `faces_total` counts every
    // face of every solid; `faces_matched` counts those that took a color.
    std::size_t faces_total = 0;
    std::size_t faces_matched = 0;
    std::size_t names_matched = 0;
};

// Correlate `attrs` onto `solids` (which MUST be in the §7.3 ordinal order).
BoundAttributes bind_attributes(const std::vector<TopoDS_Shape>& solids,
                                const StepAttributes& attrs);

}  // namespace onecad::io
