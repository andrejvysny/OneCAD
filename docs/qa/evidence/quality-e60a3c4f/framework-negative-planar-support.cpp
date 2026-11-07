#include "ops/BooleanInternals.h"
#include "ops/OpCommon.h"
#include "kernel/validation/GeometryPrecision.h"
#include <BinTools.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRepBuilderAPI_Copy.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepTools.hxx>
#include <BRepTools_ReShape.hxx>
#include <BRep_Tool.hxx>
#include <BRepGProp.hxx>
#include <GProp_GProps.hxx>
#include <Geom_BSplineSurface.hxx>
#include <Geom_Curve.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Iterator.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <gp_Pln.hxx>
#include <algorithm>
#include <fstream>
#include <iostream>
#include <tuple>
#include <vector>
using namespace onecad;
using namespace onecad::kernel::validation;
using Use=std::tuple<int,int,int>;

struct Boundary { TopTools_IndexedMapOfShape edges, wires; std::vector<Use> uses; };
Boundary boundary(const TopoDS_Shape& face, const Boundary* prior=nullptr) {
    Boundary result;
    if(prior) {result.edges=prior->edges; result.wires=prior->wires;}
    TopoDS_Shape forward=face; forward.Orientation(TopAbs_FORWARD);
    for(TopoDS_Iterator w(forward,false,true); w.More(); w.Next()) {
        if(w.Value().ShapeType()!=TopAbs_WIRE) throw std::runtime_error("non-wire face child");
        int wi=prior?result.wires.FindIndex(w.Value()):result.wires.Add(w.Value());
        if(!wi) throw std::runtime_error("new wire");
        result.uses.emplace_back(0,wi,static_cast<int>(w.Value().Orientation()));
        for(TopoDS_Iterator e(w.Value(),false,true);e.More();e.Next()) {
            if(e.Value().ShapeType()!=TopAbs_EDGE) throw std::runtime_error("non-edge wire child");
            int ei=prior?result.edges.FindIndex(e.Value()):result.edges.Add(e.Value());
            if(!ei) throw std::runtime_error("new edge");
            result.uses.emplace_back(wi,ei,static_cast<int>(e.Value().Orientation()));
        }
    }
    std::sort(result.uses.begin(),result.uses.end());return result;
}
struct EdgeGeometry { occ::handle<Geom_Curve> curve; TopLoc_Location location; double first,last,tolerance; };
std::vector<EdgeGeometry> edges(const Boundary& b) {
    std::vector<EdgeGeometry> out;
    for(int i=1;i<=b.edges.Extent();++i) {
        auto edge=TopoDS::Edge(b.edges(i)); EdgeGeometry g;
        g.curve=BRep_Tool::Curve(edge,g.location,g.first,g.last);g.tolerance=BRep_Tool::Tolerance(edge);out.push_back(g);
    }
    return out;
}
bool same_edges(const std::vector<EdgeGeometry>& a,const std::vector<EdgeGeometry>& b) {
    if(a.size()!=b.size()) return false;
    for(size_t i=0;i<a.size();++i) if(a[i].curve!=b[i].curve || !a[i].location.IsEqual(b[i].location) ||
        a[i].first!=b[i].first || a[i].last!=b[i].last || a[i].tolerance!=b[i].tolerance) return false;
    return true;
}
nlohmann::json audit(const TopoDS_Shape& s,double cap) {
    PublicationPolicy p;p.name="Boolean";p.allowed_top_level_shapes=TopLevelShapePolicy::SolidSet;
    p.max_solid_count=-1;p.tier=PublicationTier::TierB;p.require_closed_manifold=true;p.maximum_tolerance=cap;p.allow_empty_lifecycle=true;
    return evaluate_publication_policy(collect_shape_evidence(s,p.tier),p).to_json();
}
int main(int argc,char**argv) {
    if(argc!=3)return 2; TopoDS_Shape target,tool;
    {std::ifstream in(argv[1],std::ios::binary);BinTools::Read(target,in);}
    {std::ifstream in(argv[2],std::ios::binary);BinTools::Read(tool,in);}
    if(target.IsNull()||tool.IsNull())return 3;
    BRepBuilderAPI_Copy copy(target,true,false); if(!copy.IsDone())return 4;
    TopoDS_Shape work=copy.Shape();TopTools_IndexedMapOfShape faces;TopExp::MapShapes(work,TopAbs_FACE,faces);
    if(faces.Extent()<9)return 5;
    auto oldface=TopoDS::Face(faces(9));auto sf=BRepAdaptor_Surface(oldface);
    if(sf.GetType()!=GeomAbs_BSplineSurface)return 6;
    auto bs=sf.BSpline();
    if(bs->UDegree()!=1||bs->VDegree()!=1||bs->NbUPoles()!=2||bs->NbVPoles()!=2||bs->IsURational()||bs->IsVRational())return 7;
    auto a=bs->Pole(1,1),b=bs->Pole(1,2),c=bs->Pole(2,1),d=bs->Pole(2,2);
    if(a.X()!=c.X()||a.Z()!=c.Z()||b.X()!=d.X()||b.Z()!=d.Z()||a.X()==b.X())return 8;
    auto before=boundary(oldface);auto geometry_before=edges(before);
    gp_Dir normal(b.Z()-a.Z(),0.,a.X()-b.X());
    gp_Pnt midpoint; gp_Vec du,dv;
    sf.D1((sf.FirstUParameter()+sf.LastUParameter())/2.,(sf.FirstVParameter()+sf.LastVParameter())/2.,midpoint,du,dv);
    if (gp_Vec(normal).Dot(du.Crossed(dv)) < 0.) normal.Reverse();
    gp_Pln plane(gp_Pnt(a.X(),0.,a.Z()),normal);
    TopoDS_Face forward=oldface;forward.Orientation(TopAbs_FORWARD);
    auto outer=BRepTools::OuterWire(forward); if(outer.IsNull())return 9;
    BRepBuilderAPI_MakeFace maker(plane,outer,true);
    for(TopExp_Explorer it(forward,TopAbs_WIRE);it.More();it.Next())if(!it.Current().IsSame(outer))maker.Add(TopoDS::Wire(it.Current()));
    if(!maker.IsDone())return 10;auto replacement=maker.Face();replacement.Orientation(oldface.Orientation());
    auto after=boundary(replacement,&before);auto geometry_after=edges(after);
    bool boundary_same=before.uses==after.uses;bool curves_same=same_edges(geometry_before,geometry_after);
    nlohmann::json report={{"poleProof","degree 1x1 nonrational; opposite poles have bit-identical x,z; span along y"},
        {"oldFaceOrientation",static_cast<int>(oldface.Orientation())},{"newFaceOrientation",static_cast<int>(replacement.Orientation())},
        {"boundaryUses",before.uses.size()},{"boundaryUseMultisetSame",boundary_same},{"threeDimensionalCurvesRangesLocationsTolerancesSame",curves_same},
        {"inputAudit",audit(target,0.001)},{"copyBeforeAudit",audit(work,0.001)}};
    if(!boundary_same||!curves_same){std::cout<<report.dump(2)<<std::endl;return 11;}
    occ::handle<BRepTools_ReShape> reshape=new BRepTools_ReShape;reshape->Replace(oldface,replacement);
    TopoDS_Shape converted=reshape->Apply(work);auto convertedAudit=audit(converted,0.001);report["convertedAudit"]=convertedAudit;
    if(convertedAudit["disposition"]!="publishable"){std::cout<<report.dump(2)<<std::endl;return 12;}
    BRepTools::Write(converted,"/private/tmp/onecad-resume-evidence/framework-negative-planar-operand.brep");
    auto precision=precision_of(converted);double cap=precision.tolerance_ceiling(std::max(precision.input_tolerance,precision_of(tool).input_tolerance),2.,1e-6);
    std::shared_ptr<BRepBuilderAPI_MakeShape> history;
    auto raw=ops::checked_boolean(converted,tool,app::BooleanMode::Cut,false,nlohmann::json::object(),nullptr,history);
    if(raw.shape.IsNull()||!history)return 13;
    report["cutCeilingMm"]=cap;report["rawAudit"]=audit(raw.shape,cap);
    bool normalized=ops::normalize_boolean_internals(converted,raw.shape,history,nullptr,{tool});
    report["normalized"]=normalized;report["normalizedAudit"]=audit(raw.shape,cap);
    BRepTools::Write(raw.shape,"/private/tmp/onecad-resume-evidence/framework-negative-planar-cut.brep");
    std::cout<<report.dump(2)<<std::endl;return 0;
}
