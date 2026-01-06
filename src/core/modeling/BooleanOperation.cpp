#include "BooleanOperation.h"

#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepAlgoAPI_Common.hxx>
#include <BRepExtrema_DistShapeShape.hxx>
#include <BRepGProp.hxx>
#include <GProp_GProps.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Face.hxx>
#include <BRepAdaptor_Surface.hxx>

namespace onecad::core::modeling {

TopoDS_Shape BooleanOperation::perform(const TopoDS_Shape& tool, 
                                       const TopoDS_Shape& target, 
                                       app::BooleanMode mode) {
    if (target.IsNull()) {
        return tool;
    }

    switch (mode) {
        case app::BooleanMode::Add: {
            BRepAlgoAPI_Fuse fuse(target, tool);
            fuse.Build();
            if (fuse.IsDone()) {
                return fuse.Shape();
            }
            break;
        }
        case app::BooleanMode::Cut: {
            BRepAlgoAPI_Cut cut(target, tool);
            cut.Build();
            if (cut.IsDone()) {
                return cut.Shape();
            }
            break;
        }
        // Intersect (Common) logic can be added here if BooleanMode supports it.
        // Currently OperationRecord.h only defines NewBody, Add, Cut.
        default:
            break;
    }
    return tool;
}

app::BooleanMode BooleanOperation::detectMode(const TopoDS_Shape& tool, 
                                              const std::vector<TopoDS_Shape>& targets,
                                              const gp_Dir& extrudeDir) {
    // Simple heuristic:
    // 1. Find if tool intersects with any target.
    // 2. If no intersection -> NewBody.
    // 3. If intersection:
    //    - Check direction vs normal of intersected faces? 
    //    - For now, let's use a simpler approach similar to Shapr3D:
    //      - If the tool was created by pulling OUT of a body -> Add.
    //      - If the tool was created by pushing INTO a body -> Cut.
    
    // Determining "Pushing In" vs "Pulling Out":
    // This requires context about the base face. 
    // But here we only have the final shape and direction.
    
    // Alternative check: Volume intersection.
    // If the tool shares significant volume with a target, it's likely a Cut (removing material) or Add.
    // However, Extrude logic usually handles this by:
    // - Positive extrusion from a face usually adds material.
    // - Negative extrusion (into the body) usually removes material.
    
    // We can check if the tool volume overlaps significantly with any target.
    
    for (const auto& target : targets) {
        // Quick bounding box check
        BRepExtrema_DistShapeShape dist(tool, target);
        if (dist.Value() > 1e-6) {
            continue; // Not touching
        }

        // Check for common volume
        BRepAlgoAPI_Common common(target, tool);
        common.Build();
        if (common.IsDone() && !common.Shape().IsNull()) {
            GProp_GProps props;
            BRepGProp::VolumeProperties(common.Shape(), props);
            if (props.Mass() > 1e-6) {
                // Significant overlap -> Likely Cut?
                // Or user might want to Add. 
                // Defaulting to Cut if overlap is detected is a common "Push" behavior.
                return app::BooleanMode::Cut; 
            }
        }
        
        // If touching but no volume overlap (e.g. face-to-face), it's likely an Add.
        return app::BooleanMode::Add;
    }

    return app::BooleanMode::NewBody;
}

} // namespace onecad::core::modeling
