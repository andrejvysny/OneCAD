// XcafRead.cpp — see XcafRead.h.
#include "io/XcafRead.h"

#include <cmath>
#include <mutex>
#include <set>

#include <BRepGProp.hxx>
#include <GProp_GProps.hxx>
#include <IFSelect_ReturnStatus.hxx>
#include <Message_ProgressRange.hxx>
#include <Quantity_ColorRGBA.hxx>
#include <STEPCAFControl_Reader.hxx>
#include <STEPControl_Controller.hxx>
#include <Standard_Failure.hxx>
#include <TCollection_AsciiString.hxx>
#include <TDF_Label.hxx>
#include <TDF_LabelSequence.hxx>
#include <TDataStd_Name.hxx>
#include <TDocStd_Application.hxx>
#include <TDocStd_Document.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <XCAFDoc_ColorTool.hxx>
#include <XCAFDoc_ColorType.hxx>
#include <XCAFDoc_DocumentTool.hxx>
#include <XCAFDoc_ShapeTool.hxx>

#include "io/OcafApp.h"
#include "io/OcctStaticGuard.h"
#include "ops/CancelProgress.h"
#include "util/Log.h"

namespace onecad::io {

namespace {

// The SAME 1e-6 quantization `ops::ordered_solids`, the ElementMap descriptors and
// the test digests use. Rendered as a signed integer, never a formatted double.
long long quantize(double v) {
    if (!std::isfinite(v)) return 0;
    return static_cast<long long>(std::llround(v * 1e6));
}

// Records `key → value`, dropping the entry entirely when two DIFFERENT values
// collide on one key. See XcafRead.h: a coincident face must not inherit a
// neighbour's appearance.
template <typename V>
void insert_unambiguous(std::map<GeomKey, V>& into, std::set<GeomKey>& poisoned,
                        const GeomKey& key, const V& value) {
    if (poisoned.count(key) != 0) return;
    auto it = into.find(key);
    if (it == into.end()) {
        into.emplace(key, value);
        return;
    }
    if (!(it->second == value)) {
        into.erase(it);
        poisoned.insert(key);
    }
}

std::string label_name(const TDF_Label& label) {
    Handle(TDataStd_Name) attr;
    if (!label.FindAttribute(TDataStd_Name::GetID(), attr)) return {};
    return TCollection_AsciiString(attr->Get()).ToCString();
}

// Deepest assembly nesting walked. A STEP assembly this deep is pathological; the
// bound exists so a cyclic/oversized label tree cannot make an ADVISORY pass
// expensive.
constexpr int kMaxAssemblyDepth = 16;

// `ColorSurf` is what STEP's `surface_style_usage` maps to; `ColorGen` is the
// generic fallback a producer may have used instead.
bool shape_color(const Handle(XCAFDoc_ColorTool) & colors, const TopoDS_Shape& shape,
                 PackedColor& out) {
    Quantity_ColorRGBA color;
    if (colors->GetColor(shape, XCAFDoc_ColorSurf, color) ||
        colors->GetColor(shape, XCAFDoc_ColorGen, color)) {
        out = pack_srgb(color);
        return true;
    }
    return false;
}

bool own_label_color(const TDF_Label& label, PackedColor& out) {
    Quantity_ColorRGBA color;
    if (XCAFDoc_ColorTool::GetColor(label, XCAFDoc_ColorSurf, color) ||
        XCAFDoc_ColorTool::GetColor(label, XCAFDoc_ColorGen, color)) {
        out = pack_srgb(color);
        return true;
    }
    return false;
}

// PASS 1 — names, plus every EXPLICIT per-face color. Recurses into assembly
// components so a nested part's faces are reached with their instance location
// applied (which is what makes the geometric key line up with `read_step`'s
// flattened solids).
void harvest_explicit(const Handle(XCAFDoc_ShapeTool) & shapes,
                      const Handle(XCAFDoc_ColorTool) & colors, const TDF_Label& label,
                      int depth, StepAttributes& out, std::set<GeomKey>& poisoned_names,
                      std::set<GeomKey>& poisoned_colors) {
    if (depth > kMaxAssemblyDepth) return;
    const TopoDS_Shape shape = XCAFDoc_ShapeTool::GetShape(label);
    if (shape.IsNull()) return;

    const std::string name = label_name(label);
    if (!name.empty()) {
        // A label may wrap a compound of several solids (a STEP root that carried
        // more than one body); every one of them takes the label's name.
        for (TopExp_Explorer se(shape, TopAbs_SOLID); se.More(); se.Next()) {
            insert_unambiguous(out.solid_names, poisoned_names, solid_key(se.Current()), name);
        }
    }

    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(shape, TopAbs_FACE, faces);
    for (int i = 1; i <= faces.Extent(); ++i) {
        PackedColor color = kUnsetColor;
        if (!shape_color(colors, faces(i), color)) continue;
        insert_unambiguous(out.face_colors, poisoned_colors, face_key(faces(i)), color);
    }

    TDF_LabelSequence components;
    if (XCAFDoc_ShapeTool::GetComponents(label, components)) {
        for (int i = 1; i <= components.Length(); ++i) {
            harvest_explicit(shapes, colors, components.Value(i), depth + 1, out, poisoned_names,
                             poisoned_colors);
        }
    }
}

// PASS 2 — colors set on a SHAPE'S OWN LABEL rather than on any face (the usual
// "the whole part is blue" case). XCAF does NOT resolve those downward:
// `GetColor(face, …)` returns false for a face whose only color lives on the
// enclosing solid's label. Runs after pass 1 and only fills faces that took no
// explicit color, so a per-face color always wins over its part's.
void harvest_inherited(const Handle(XCAFDoc_ShapeTool) & shapes,
                       const Handle(XCAFDoc_ColorTool) & colors, const TDF_Label& label,
                       int depth, StepAttributes& out, std::set<GeomKey>& poisoned_colors) {
    if (depth > kMaxAssemblyDepth) return;
    const TopoDS_Shape shape = XCAFDoc_ShapeTool::GetShape(label);
    if (shape.IsNull()) return;

    PackedColor whole = kUnsetColor;
    if (own_label_color(label, whole)) {
        TopTools_IndexedMapOfShape faces;
        TopExp::MapShapes(shape, TopAbs_FACE, faces);
        for (int i = 1; i <= faces.Extent(); ++i) {
            const GeomKey key = face_key(faces(i));
            if (out.face_colors.count(key) != 0) continue;  // explicit wins
            insert_unambiguous(out.face_colors, poisoned_colors, key, whole);
        }
    }

    TDF_LabelSequence components;
    if (XCAFDoc_ShapeTool::GetComponents(label, components)) {
        for (int i = 1; i <= components.Length(); ++i) {
            harvest_inherited(shapes, colors, components.Value(i), depth + 1, out,
                              poisoned_colors);
        }
    }
}

}  // namespace

GeomKey face_key(const TopoDS_Shape& face) {
    GeomKey k;
    if (face.IsNull()) return k;
    GProp_GProps props;
    BRepGProp::SurfaceProperties(face, props);
    const gp_Pnt c = props.CentreOfMass();
    k.measure = quantize(props.Mass());
    k.cx = quantize(c.X());
    k.cy = quantize(c.Y());
    k.cz = quantize(c.Z());
    return k;
}

GeomKey solid_key(const TopoDS_Shape& solid) {
    GeomKey k;
    if (solid.IsNull()) return k;
    GProp_GProps props;
    BRepGProp::VolumeProperties(solid, props);
    const gp_Pnt c = props.CentreOfMass();
    k.measure = quantize(props.Mass());
    k.cx = quantize(c.X());
    k.cy = quantize(c.Y());
    k.cz = quantize(c.Z());
    return k;
}

StepAttributes read_step_attributes(const std::string& path, const StepReadPolicy& policy,
                                    const onecad::CancelToken* cancel) {
    StepAttributes out;
    if (path.empty()) {
        out.error = "XcafRead: empty path";
        return out;
    }
    if (cancel != nullptr && cancel->cancelled()) {
        out.error = "XcafRead: cancelled";
        return out;
    }

    // Same prologue as `read_step`: register the norm + its knobs BEFORE the guard
    // snapshots them, and keep OCCT chatter off fd 1.
    STEPControl_Controller::Init();
    OcctMessengerGuard quiet;
    InterfaceStaticGuard knobs;

    const std::lock_guard<std::mutex> lock(ocaf_mutex());
    Handle(TDocStd_Document) doc;
    try {
        knobs.set_cstr("xstep.cascade.unit", policy.target_unit);
        knobs.set_int("read.precision.mode", policy.precision_mode);
        knobs.set_real("read.precision.val", policy.precision_val);
        knobs.set_int("read.step.product.mode", policy.product_mode);
        knobs.set_int("read.maxprecision.mode", policy.max_precision_mode);
        knobs.set_real("read.maxprecision.val", policy.max_precision_val);

        STEPCAFControl_Reader reader;
        reader.SetColorMode(Standard_True);
        reader.SetNameMode(Standard_True);
        reader.SetLayerMode(Standard_False);
        reader.SetPropsMode(Standard_False);
        if (reader.ReadFile(path.c_str()) != IFSelect_RetDone) {
            out.error = "XcafRead: ReadFile failed for " + path;
            return out;
        }

        ocaf_application()->NewDocument("BinXCAF", doc);
        if (doc.IsNull()) {
            out.error = "XcafRead: could not create an XCAF document";
            return out;
        }

        Message_ProgressRange range;
        Handle(ops::CancelProgress) progress;
        if (cancel != nullptr) {
            progress = new ops::CancelProgress(*cancel);
            range = progress->Start();
        }
        if (reader.Transfer(doc, range) != Standard_True) {
            out.error = "XcafRead: XCAF transfer produced nothing for " + path;
            ocaf_application()->Close(doc);
            return out;
        }
        if (cancel != nullptr && cancel->cancelled()) {
            out.error = "XcafRead: cancelled";
            ocaf_application()->Close(doc);
            return out;
        }

        Handle(XCAFDoc_ShapeTool) shapes = XCAFDoc_DocumentTool::ShapeTool(doc->Main());
        Handle(XCAFDoc_ColorTool) colors = XCAFDoc_DocumentTool::ColorTool(doc->Main());
        TDF_LabelSequence free_labels;
        shapes->GetFreeShapes(free_labels);

        std::set<GeomKey> poisoned_names;
        std::set<GeomKey> poisoned_colors;
        for (int i = 1; i <= free_labels.Length(); ++i) {
            harvest_explicit(shapes, colors, free_labels.Value(i), 0, out, poisoned_names,
                             poisoned_colors);
        }
        for (int i = 1; i <= free_labels.Length(); ++i) {
            harvest_inherited(shapes, colors, free_labels.Value(i), 0, out, poisoned_colors);
        }
        ocaf_application()->Close(doc);
    } catch (const Standard_Failure& f) {
        if (!doc.IsNull()) {
            try {
                ocaf_application()->Close(doc);
            } catch (const Standard_Failure&) {
            }
        }
        out.face_colors.clear();
        out.solid_names.clear();
        out.error = std::string("XcafRead raised: ") +
                    (f.GetMessageString() != nullptr ? f.GetMessageString() : "OCCT failure");
        WLOG_ERROR("XcafRead: %s", out.error.c_str());
    } catch (const std::exception& e) {
        out.face_colors.clear();
        out.solid_names.clear();
        out.error = std::string("XcafRead raised: ") + e.what();
        WLOG_ERROR("XcafRead: %s", out.error.c_str());
    }
    return out;
}

BoundAttributes bind_attributes(const std::vector<TopoDS_Shape>& solids,
                                const StepAttributes& attrs) {
    BoundAttributes out;
    out.per_solid.resize(solids.size());
    for (std::size_t k = 0; k < solids.size(); ++k) {
        SolidAttributes& sa = out.per_solid[k];
        if (solids[k].IsNull()) continue;

        const auto name_it = attrs.solid_names.find(solid_key(solids[k]));
        if (name_it != attrs.solid_names.end()) {
            sa.name = name_it->second;
            ++out.names_matched;
        }

        TopTools_IndexedMapOfShape faces;
        TopExp::MapShapes(solids[k], TopAbs_FACE, faces);
        out.faces_total += static_cast<std::size_t>(faces.Extent());
        if (attrs.face_colors.empty()) continue;

        std::vector<PackedColor> colors(static_cast<std::size_t>(faces.Extent()), kUnsetColor);
        bool any = false;
        for (int i = 1; i <= faces.Extent(); ++i) {
            const auto it = attrs.face_colors.find(face_key(faces(i)));
            if (it == attrs.face_colors.end()) continue;
            colors[static_cast<std::size_t>(i - 1)] = it->second;
            any = true;
            ++out.faces_matched;
        }
        if (any) sa.face_colors = std::move(colors);
    }
    return out;
}

}  // namespace onecad::io
