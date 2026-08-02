// step_fixture_util.h — shared helpers for the STEP-IMPORT W0 tests.
//
// Header-only, test-only. Two jobs:
//
//  1. FIXTURE GENERATION. `corpus/step/NOTE.md` records that no STEP reference
//     file could be captured from the `OneCAD-CPP` oracle: its exporter is
//     reachable only through the Qt GUI, so there is no code-free producer. The
//     fix is not to commit opaque binaries but to GENERATE the fixtures at test
//     time from the same `STEPControl_Writer` sequence `src/io/ExportStep.cpp`
//     uses (schema knob → Transfer(AsIs) → Write). The producer is therefore in
//     the tree, the bytes are reproducible, and nothing binary is tracked.
//
//     The writer calls are mirrored here rather than routed through
//     `handle_export_step()` because that entry point takes bodies from a live
//     `Session`, which would mean standing up a full ExecutePlan/AcceptPrepared
//     transaction just to hand OCCT a cylinder. Keep this mirror in step with
//     ExportStep.cpp if its writer sequence ever changes.
//
//  2. THE CANONICAL DIGEST. One deterministic, byte-comparable rendering of a
//     read result, used for the in-process double-read check, the cross-process
//     check, and the BinTools roundtrip check. Everything in it is quantized or
//     integral — no raw doubles are printed, because a printf of a double is
//     exactly the place a "deterministic" comparison silently becomes a
//     comparison of rounding modes.
#pragma once

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <sstream>
#include <string>
#include <tuple>
#include <vector>

#include <BRepBuilderAPI_MakeShape.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRep_Builder.hxx>
#include <BRepGProp.hxx>
#include <GProp_GProps.hxx>
#include <IFSelect_ReturnStatus.hxx>
#include <Interface_Static.hxx>
#include <STEPControl_StepModelType.hxx>
#include <STEPControl_Writer.hxx>
#include <Standard_Failure.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS_Compound.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Ax2.hxx>
#include <gp_Dir.hxx>
#include <gp_Pnt.hxx>

#include "elementmap/ElementMapPartition.h"
#include "io/StepRead.h"
#include "ops/OpCommon.h"
#include "session/BodyStore.h"
#include "session/Signatures.h"

namespace stepfx {

// --- Fixture geometry -------------------------------------------------------
// Analytic volumes, asserted by the tests. Kept as constants so the geometry and
// the oracle cannot drift apart.
inline constexpr double kBoxDx = 20.0;
inline constexpr double kBoxDy = 30.0;
inline constexpr double kBoxDz = 40.0;
inline constexpr double kBoxVolume = kBoxDx * kBoxDy * kBoxDz;  // 24000

// The multi-solid fixture: two DISJOINT boxes plus a cylinder. Distinct volumes
// and distinct centroids on purpose — `ops::ordered_solids` sorts on
// (volume, centroid, faceCount), so equal-volume twins would leave the order
// resting on the centroid tie-break alone and weaken the determinism signal.
inline constexpr double kBoxAVolume = kBoxVolume;               // 20x30x40 at origin
inline constexpr double kBoxBDx = 10.0;
inline constexpr double kBoxBDy = 10.0;
inline constexpr double kBoxBDz = 10.0;
inline constexpr double kBoxBVolume = kBoxBDx * kBoxBDy * kBoxBDz;  // 1000
inline constexpr double kCylRadius = 5.0;
inline constexpr double kCylHeight = 12.0;

inline double cylinder_volume() { return M_PI * kCylRadius * kCylRadius * kCylHeight; }

inline TopoDS_Shape make_single_box() {
    return BRepPrimAPI_MakeBox(gp_Pnt(0, 0, 0), kBoxDx, kBoxDy, kBoxDz).Shape();
}

inline TopoDS_Shape make_multi_solid() {
    BRep_Builder builder;
    TopoDS_Compound compound;
    builder.MakeCompound(compound);
    builder.Add(compound, BRepPrimAPI_MakeBox(gp_Pnt(0, 0, 0), kBoxDx, kBoxDy, kBoxDz).Shape());
    builder.Add(compound,
                BRepPrimAPI_MakeBox(gp_Pnt(100, 0, 0), kBoxBDx, kBoxBDy, kBoxBDz).Shape());
    builder.Add(compound, BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(0, 200, 0), gp_Dir(0, 0, 1)),
                                                   kCylRadius, kCylHeight)
                              .Shape());
    return compound;
}

// Mirrors src/io/ExportStep.cpp's writer sequence. Returns "" on success, else a
// message. `schema` matches the SCHEMA §7.8 default the verb forwards.
inline std::string write_step_fixture(const TopoDS_Shape& shape, const std::string& path,
                                      const char* schema = "AP214IS") {
    try {
        STEPControl_Writer writer;
        Interface_Static::SetCVal("write.step.schema", schema);
        const IFSelect_ReturnStatus ts = writer.Transfer(shape, STEPControl_AsIs);
        if (ts != IFSelect_RetDone) return "STEP fixture: transfer failed";
        const IFSelect_ReturnStatus ws = writer.Write(path.c_str());
        if (ws != IFSelect_RetDone) return "STEP fixture: write failed";
    } catch (const Standard_Failure& f) {
        return std::string("STEP fixture raised: ") +
               (f.GetMessageString() != nullptr ? f.GetMessageString() : "OCCT");
    }
    return "";
}

// --- Canonical digest -------------------------------------------------------

// 1e-6 quantization — the SAME constant `ops::ordered_solids` and the ElementMap
// descriptors use. Rendered as a signed integer, never as a formatted double.
inline long long quantize(double v) { return static_cast<long long>(std::llround(v * 1e6)); }

// Digest of ONE ordered solid vector. Single line, fields separated by '|', so a
// mismatch shows up as a diff on one line rather than a wall of text:
//
//   solids=<n>|s0:v=<qvol>,c=<qx>,<qy>,<qz>,f=<n>,e=<n>,vx=<n>,keys=f:1;f:2;...|...|geom=<hex16>
//
// `keys` are topokey-style identifiers built from `TopExp::MapShapes` FACE
// indices of the solid — the same 1-based index space
// `ElementMapPartition::topokey_for_shape` promotes to a `TopoKey`, which is what
// makes them worth comparing: a `TopoKey` is snapshot-scoped EVIDENCE the
// resolution ladder consumes, so a map order that shifts between processes would
// silently move the evidence under every promoted id.
//
// They are emitted in CANONICAL GEOMETRIC order (faces sorted by quantized
// area + centroid), NOT in map order. Emitting them in map order would print
// `f:1;f:2;…;f:N` for every shape and compare nothing. Sorted geometrically, the
// list IS the map's permutation, so an allocator- or ASLR-sensitive pipeline
// shows up here as a reordered list even when every scalar metric still matches.
// This is the sharpest cross-process probe in the digest.
inline std::string digest_solids(const std::vector<TopoDS_Shape>& solids) {
    std::ostringstream os;
    os << "solids=" << solids.size();

    onecad::session::BodyStore bodies;
    for (std::size_t k = 0; k < solids.size(); ++k) {
        const TopoDS_Shape& s = solids[k];
        GProp_GProps props;
        BRepGProp::VolumeProperties(s, props);
        const gp_Pnt c = props.CentreOfMass();

        TopTools_IndexedMapOfShape faces;
        TopTools_IndexedMapOfShape edges;
        TopTools_IndexedMapOfShape verts;
        TopExp::MapShapes(s, TopAbs_FACE, faces);
        TopExp::MapShapes(s, TopAbs_EDGE, edges);
        TopExp::MapShapes(s, TopAbs_VERTEX, verts);

        os << "|s" << k << ":v=" << quantize(props.Mass()) << ",c=" << quantize(c.X()) << ','
           << quantize(c.Y()) << ',' << quantize(c.Z()) << ",f=" << faces.Extent()
           << ",e=" << edges.Extent() << ",vx=" << verts.Extent() << ",keys=";

        // Face map indices, listed in canonical geometric order — see above.
        using FaceKey = std::tuple<long long, long long, long long, long long, int>;
        std::vector<FaceKey> ordered;
        ordered.reserve(static_cast<std::size_t>(faces.Extent()));
        for (int i = 1; i <= faces.Extent(); ++i) {
            GProp_GProps fprops;
            BRepGProp::SurfaceProperties(faces(i), fprops);
            const gp_Pnt fc = fprops.CentreOfMass();
            ordered.emplace_back(quantize(fprops.Mass()), quantize(fc.X()), quantize(fc.Y()),
                                 quantize(fc.Z()), i);
        }
        std::sort(ordered.begin(), ordered.end());
        for (std::size_t i = 0; i < ordered.size(); ++i) {
            if (i > 0) os << ';';
            os << onecad::elementmap::ElementMapPartition::topokey_for_shape(
                s, faces(std::get<4>(ordered[i])), onecad::kernel::elementmap::ElementKind::Face);
        }

        // Zero-padded so BodyStore's ascending-id iteration matches solid order
        // for any count (b10 must not sort before b2).
        char id[16];
        std::snprintf(id, sizeof(id), "b%04zu", k);
        bodies.create(id, "stepread", s);
    }
    // The production geometry signature over the same solids — folds counts,
    // quantized bbox and quantized volume. Included so the digest fails if the
    // signature the rest of the stack fences on ever diverges from these metrics.
    os << "|geom=" << onecad::session::geometry_signature(bodies);
    return os.str();
}

// Digest of a whole read result: the solid digest plus the advisory channels, so
// a determinism regression in diagnostics ordering is caught too.
inline std::string digest_result(const onecad::io::StepReadResult& r) {
    std::ostringstream os;
    os << digest_solids(r.solids) << "|err=" << (r.error.has_value() ? *r.error : "none")
       << "|diag=";
    for (std::size_t i = 0; i < r.diagnostics.size(); ++i) {
        if (i > 0) os << ';';
        os << r.diagnostics[i].code;
    }
    os << "|names=";
    for (std::size_t i = 0; i < r.product_names.size(); ++i) {
        if (i > 0) os << ';';
        os << (r.product_names[i].empty() ? "-" : r.product_names[i]);
    }
    return os.str();
}

}  // namespace stepfx
