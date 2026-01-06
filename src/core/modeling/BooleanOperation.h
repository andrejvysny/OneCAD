#ifndef ONECAD_CORE_MODELING_BOOLEANOPERATION_H
#define ONECAD_CORE_MODELING_BOOLEANOPERATION_H

#include "../../app/document/OperationRecord.h"
#include <TopoDS_Shape.hxx>
#include <vector>
#include <gp_Dir.hxx>

namespace onecad::core::modeling {

class BooleanOperation {
public:
    /**
     * @brief Performs a boolean operation between a tool shape and target shapes.
     * @param tool The shape being applied (e.g., the extrusion).
     * @param targets The list of shapes to modify (usually just one).
     * @param mode The boolean operation mode.
     * @return The resulting shape. If multiple targets, they might be fused first or processed sequentially.
     *         For v1, we assume single target for Cut/Join/Intersect.
     */
    static TopoDS_Shape perform(const TopoDS_Shape& tool, 
                                const TopoDS_Shape& target, 
                                app::BooleanMode mode);

    /**
     * @brief Detects the most likely boolean mode based on geometric relationship.
     * @param tool The shape being applied (e.g., the extrusion).
     * @param targets The potential target shapes in the document.
     * @param extrudeDir The direction of extrusion (helper for intuition).
     * @return The detected mode (NewBody, Add, Cut).
     */
    static app::BooleanMode detectMode(const TopoDS_Shape& tool, 
                                       const std::vector<TopoDS_Shape>& targets,
                                       const gp_Dir& extrudeDir);
};

} // namespace onecad::core::modeling

#endif // ONECAD_CORE_MODELING_BOOLEANOPERATION_H
