#include "ops/BooleanInternals.h"
#include "ops/OpCommon.h"
#include "kernel/validation/GeometryPrecision.h"
#include <BinTools.hxx>
#include <BRepTools.hxx>
#include <BRepGProp.hxx>
#include <GProp_GProps.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <fstream>
#include <iostream>
#include <memory>
#include <vector>

using namespace onecad;
using namespace onecad::kernel::validation;
static nlohmann::json audit(const TopoDS_Shape& shape, double ceiling) {
    PublicationPolicy p;
    p.name = "Boolean";
    p.allowed_top_level_shapes = TopLevelShapePolicy::SolidSet;
    p.max_solid_count = -1;
    p.tier = PublicationTier::TierB;
    p.require_closed_manifold = true;
    p.maximum_tolerance = ceiling;
    p.allow_empty_lifecycle = true;
    return evaluate_publication_policy(collect_shape_evidence(shape, p.tier), p).to_json();
}
static nlohmann::json lineage(const TopoDS_Shape& input, const TopoDS_Shape& result,
                             BRepBuilderAPI_MakeShape& history) {
    TopTools_IndexedMapOfShape faces, final_faces;
    TopExp::MapShapes(input, TopAbs_FACE, faces);
    TopExp::MapShapes(result, TopAbs_FACE, final_faces);
    nlohmann::json rows = nlohmann::json::array();
    for (int i=1; i<=faces.Extent(); ++i) {
        const auto& f=faces(i); int modified=0,generated=0;
        for (const auto& image : history.Modified(f)) if (final_faces.Contains(image)) ++modified;
        for (const auto& image : history.Generated(f)) if (final_faces.Contains(image)) ++generated;
        rows.push_back({{"face",i},{"deleted",history.IsDeleted(f)},
                        {"unchanged",final_faces.Contains(f)},
                        {"modifiedFinal",modified},{"generatedFinal",generated}});
    }
    return rows;
}
static nlohmann::json properties(const TopoDS_Shape& shape) {
    GProp_GProps volume,area;
    BRepGProp::VolumeProperties(shape,volume);
    BRepGProp::SurfaceProperties(shape,area);
    return {{"volumeMm3",volume.Mass()},{"areaMm2",area.Mass()}};
}
int main(int argc, char** argv) {
    if (argc != 3) return 2;
    TopoDS_Shape target,tool;
    { std::ifstream in(argv[1],std::ios::binary); BinTools::Read(target,in); }
    { std::ifstream in(argv[2],std::ios::binary); BinTools::Read(tool,in); }
    if(target.IsNull() || tool.IsNull()) return 3;
    auto prec=precision_of(target);
    double ceiling=prec.tolerance_ceiling(std::max(prec.input_tolerance,precision_of(tool).input_tolerance),2.0,1.0e-6);
    std::shared_ptr<BRepBuilderAPI_MakeShape> history;
    auto raw=ops::checked_boolean(target,tool,app::BooleanMode::Intersect,false,nlohmann::json::object(),nullptr,history);
    if(raw.shape.IsNull() || !history) { std::cerr << raw.error_message << std::endl; return 4; }
    nlohmann::json result={{"ceilingMm",ceiling},{"beforeAudit",audit(raw.shape,ceiling)},
        {"beforeProperties",properties(raw.shape)},
        {"targetBeforeLineage",lineage(target,raw.shape,*history)},
        {"toolBeforeLineage",lineage(tool,raw.shape,*history)}};
    TopoDS_Shape before=raw.shape; auto before_history=history;
    bool normalized=ops::normalize_boolean_internals(target,raw.shape,history,nullptr,{});
    result["normalized"]=normalized;
    result["sameResultIdentity"]=raw.shape.IsSame(before);
    result["sameHistoryPointer"]=history==before_history;
    result["afterAudit"]=audit(raw.shape,ceiling);
    result["afterProperties"]=properties(raw.shape);
    result["targetAfterLineage"]=lineage(target,raw.shape,*history);
    result["toolAfterLineage"]=lineage(tool,raw.shape,*history);
    if(normalized) BRepTools::Write(raw.shape,"/private/tmp/onecad-resume-evidence/framework-negative-retention-common-normalized.brep");
    std::cout<<result.dump(2)<<std::endl;
}
