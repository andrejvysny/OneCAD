#include "ops/OpCommon.h"
#include "ops/BooleanOp.h"
#include "kernel/validation/GeometryPrecision.h"
#include <BinTools.hxx>
#include <BRepTools.hxx>
#include <fstream>
#include <iostream>
#include <string>
#include <vector>
using namespace onecad;
using namespace onecad::kernel::validation;

TopoDS_Shape read_binary(const char* path) {
    TopoDS_Shape shape; std::ifstream input(path,std::ios::binary); BinTools::Read(shape,input); return shape;
}

bool execute(const std::string& label, const std::string& operation,
             const TopoDS_Shape& target, const TopoDS_Shape& tool, TopoDS_Shape& out) {
    session::BodyStore bodies;
    bodies.create("target","seed",target); bodies.create("tool","seed",tool);
    elementmap::ElementMapPartition partition;
    std::vector<std::pair<std::string,nlohmann::json>> sketches; std::string last;
    ops::OpContext ctx{bodies,&sketches,partition,&last,false,nlohmann::json::object(),nullptr};
    nlohmann::json command={{"opType","Boolean"},{"params",{{"operation",operation},
        {"targetBodyId","target"},{"toolBodyId","tool"}}}};
    auto outcome=ops::execute_boolean(ctx,command,"diagnostic-"+label);
    nlohmann::json report={{"stage",label},{"operation",operation},
        {"status",static_cast<int>(outcome.status)},{"code",outcome.error_code},
        {"message",outcome.error_message},{"diagnostics",outcome.diagnostics},
        {"eventCount",outcome.body_events.size()},{"targetPresent",bodies.contains("target")},
        {"toolPresent",bodies.contains("tool")}};
    if (outcome.status != ops::OpOutcome::Status::Ok || !bodies.contains("target")) {
        std::cout<<report.dump()<<std::endl; return false;
    }
    out=bodies.get("target")->geom;
    const auto prec=precision_of(target);
    PublicationPolicy policy; policy.name="Boolean"; policy.allowed_top_level_shapes=TopLevelShapePolicy::SolidSet;
    policy.max_solid_count=-1; policy.tier=PublicationTier::TierB; policy.require_closed_manifold=true;
    policy.maximum_tolerance=prec.tolerance_ceiling(std::max(prec.input_tolerance,precision_of(tool).input_tolerance),2.,1e-6);
    policy.allow_empty_lifecycle=true;
    auto decision=evaluate_publication_policy(collect_shape_evidence(out,policy.tier),policy);
    report["ceilingMm"]=policy.maximum_tolerance; report["audit"]=decision.to_json();
    std::string path="/private/tmp/onecad-resume-evidence/framework-algebra-"+label+".brep";
    BRepTools::Write(out,path.c_str()); report["path"]=path;
    std::cout<<report.dump()<<std::endl;
    return decision.publishable();
}
int main(int argc,char**argv) {
    if(argc!=5) return 2;
    auto B=read_binary(argv[1]), S=read_binary(argv[2]), P=read_binary(argv[3]), C=read_binary(argv[4]);
    if(B.IsNull()||S.IsNull()||P.IsNull()||C.IsNull()) return 3;
    TopoDS_Shape D,U,X,Y,final;
    if(!execute("D", "Cut", B,S,D)) return 10;
    if(!execute("U", "Union", D,P,U)) return 11;
    if(!execute("X", "Intersect", P,C,X)) return 12;
    if(!execute("Y", "Intersect", X,S,Y)) return 13;
    if(!execute("final", "Cut", U,Y,final)) return 14;
    return 0;
}
