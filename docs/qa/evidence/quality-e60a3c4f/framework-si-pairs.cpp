#include <BRep_Builder.hxx>

#include <BRepTools.hxx>
#include <BRepAlgoAPI_Check.hxx>
#include <BOPAlgo_CheckResult.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <iostream>
#include <iomanip>
int main(int argc,char**argv){
 if(argc!=2)return 2;TopoDS_Shape s;BRep_Builder b;if(!BRepTools::Read(s,argv[1],b))return 3;
 TopTools_IndexedMapOfShape all,faces,edges,vertices;TopExp::MapShapes(s,all);TopExp::MapShapes(s,TopAbs_FACE,faces);TopExp::MapShapes(s,TopAbs_EDGE,edges);TopExp::MapShapes(s,TopAbs_VERTEX,vertices);
 auto print=[&](const TopoDS_Shape& v){if(v.IsNull()){std::cout<<" null";return;}std::cout<<" type="<<int(v.ShapeType())<<" all="<<all.FindIndex(v)<<" face="<<faces.FindIndex(v)<<" edge="<<edges.FindIndex(v)<<" vertex="<<vertices.FindIndex(v)<<" orientation="<<int(v.Orientation());};
 BRepAlgoAPI_Check c;c.SetData(s,false,true);c.SetRunParallel(false);c.Perform();if(c.HasErrors())return 4;
 int n=0;for(const BOPAlgo_CheckResult& r:c.Result()){if(r.GetCheckStatus()!=BOPAlgo_SelfIntersect)continue;std::cout<<"\nresult "<<++n<<" ancestor1";print(r.GetShape1());std::cout<<" ancestor2";print(r.GetShape2());std::cout<<"\nfaulty1";for(const auto& v:r.GetFaultyShapes1())print(v);std::cout<<"\nfaulty2";for(const auto& v:r.GetFaultyShapes2())print(v);std::cout<<"\n";}std::cout<<"count "<<n<<"\n";
}
