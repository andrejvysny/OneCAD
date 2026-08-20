// HoleTool.cpp — see HoleTool.h.
#include "ops/HoleTool.h"

#include <algorithm>
#include <cmath>
#include <vector>

#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepBndLib.hxx>
#include <BRepPrimAPI_MakeCone.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <Bnd_Box.hxx>
#include <Standard_Failure.hxx>
#include <gp_Ax2.hxx>
#include <gp_Vec.hxx>

#include "kernel/validation/GeometryPrecision.h"

namespace onecad::ops {

namespace {

// "A dimension the kernel can act on" now comes from the precision context —
// `GeometryPrecisionContext::authoring_resolution()`. The value is unchanged
// (1e-3 mm); it is no longer restated here.
// ExtrudeOp.cpp:49 kThroughAllFallback — used only when the host has no bbox.
constexpr double kThroughAllFallback = 1.0e5;

constexpr double kPi = 3.14159265358979323846;

double deg_to_rad(double deg) { return deg * kPi / 180.0; }

// A coaxial cylinder that starts `kFaceOvershoot` outside the face and runs
// `length` INTO the material (so exactly `length` of material is removed).
TopoDS_Shape seated_cylinder(const gp_Pnt& origin, const gp_Dir& axis, double radius,
                             double length) {
    const gp_Pnt start = origin.Translated(gp_Vec(axis) * -kFaceOvershoot);
    return BRepPrimAPI_MakeCylinder(gp_Ax2(start, axis), radius, length + kFaceOvershoot).Shape();
}

}  // namespace

double countersink_depth(const HoleDims& d) {
    if (d.cs_diameter <= 0.0 || d.cs_angle_deg <= 0.0 || d.cs_angle_deg >= 180.0) return 0.0;
    const double half = deg_to_rad(d.cs_angle_deg) * 0.5;
    const double t = std::tan(half);
    if (t <= 0.0) return 0.0;
    return ((d.cs_diameter - d.diameter) * 0.5) / t;
}

double through_all_length(const gp_Pnt& origin, const gp_Dir& axis, const TopoDS_Shape& target) {
    // Mirrors ExtrudeOp::through_all_distance: bound the drill by the host's own
    // extent rather than a magic length, so the boolean never works with a
    // kilometre-long tool on a millimetre part.
    if (!target.IsNull()) {
        Bnd_Box box;
        BRepBndLib::Add(target, box);
        if (!box.IsVoid()) {
            Standard_Real xmin, ymin, zmin, xmax, ymax, zmax;
            box.Get(xmin, ymin, zmin, xmax, ymax, zmax);
            double max_proj = 0.0;
            for (int corner = 0; corner < 8; ++corner) {
                const gp_Pnt p((corner & 1) ? xmax : xmin, (corner & 2) ? ymax : ymin,
                               (corner & 4) ? zmax : zmin);
                max_proj = std::max(max_proj, gp_Vec(origin, p).Dot(gp_Vec(axis)));
            }
            const double diag = gp_Pnt(xmin, ymin, zmin).Distance(gp_Pnt(xmax, ymax, zmax));
            const double min_value =
                kernel::validation::precision_of(target).authoring_resolution();
            return std::max(max_proj, min_value) + 0.01 * diag + 1.0;
        }
    }
    return kThroughAllFallback;
}

TopoDS_Shape build_hole_tool(const gp_Pnt& origin, const gp_Dir& axis, const HoleDims& dims,
                             std::string& err) {
    const double drill_r = dims.diameter * 0.5;
    // Dimension validation runs before any shape is measured, and
    // `authoring_resolution()` is scale-independent in v1 (GeometryPrecision.h),
    // so the floor-only context answers exactly what a measured one would.
    const double min_value =
        kernel::validation::GeometryPrecisionContext{}.authoring_resolution();
    if (drill_r < min_value || dims.depth < min_value) {
        err = "Hole tool has a degenerate drill (diameter/depth below 1e-3)";
        return {};
    }

    try {
        std::vector<TopoDS_Shape> pieces;
        pieces.push_back(seated_cylinder(origin, axis, drill_r, dims.depth));

        if (dims.cb_diameter > 0.0 && dims.cb_depth > 0.0) {
            pieces.push_back(seated_cylinder(origin, axis, dims.cb_diameter * 0.5, dims.cb_depth));
        }

        const double cs_depth = countersink_depth(dims);
        if (cs_depth > 0.0) {
            // FRUSTUM extension: the cone starts `kFaceOvershoot` outside the face
            // with a correspondingly larger top radius, so its profile AT the face
            // plane is still exactly `cs_diameter` (see HoleTool.h).
            const double half = deg_to_rad(dims.cs_angle_deg) * 0.5;
            const double top_r = dims.cs_diameter * 0.5 + kFaceOvershoot * std::tan(half);
            const gp_Pnt start = origin.Translated(gp_Vec(axis) * -kFaceOvershoot);
            pieces.push_back(BRepPrimAPI_MakeCone(gp_Ax2(start, axis), top_r, drill_r,
                                                  cs_depth + kFaceOvershoot)
                                 .Shape());
        }

        TopoDS_Shape tool = pieces.front();
        for (std::size_t i = 1; i < pieces.size(); ++i) {
            BRepAlgoAPI_Fuse fuse(tool, pieces[i]);
            fuse.Build();
            if (!fuse.IsDone()) {
                err = "Hole tool solid could not be fused";
                return {};
            }
            tool = fuse.Shape();
        }
        if (tool.IsNull()) err = "Hole tool solid is null";
        return tool;
    } catch (const Standard_Failure& f) {
        err = std::string("Hole tool solid could not be built: ") +
              (f.GetMessageString() ? f.GetMessageString() : "OCCT");
        return {};
    } catch (...) {
        err = "Hole tool solid could not be built";
        return {};
    }
}

}  // namespace onecad::ops
