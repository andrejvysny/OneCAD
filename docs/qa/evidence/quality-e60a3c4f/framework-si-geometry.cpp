#include <BRep_Builder.hxx>
#include <BRepTools.hxx>
#include <BRep_Tool.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRepAdaptor_Curve.hxx>
#include <BRepExtrema_DistShapeShape.hxx>
#include <BRepClass_FaceClassifier.hxx>
#include <GeomAPI_ProjectPointOnSurf.hxx>
#include <Geom2d_Curve.hxx>
#include <TopExp.hxx>
#include <TopoDS.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <iostream>
#include <iomanip>
int main(int argc,char**argv){if(argc!=2)return 2;TopoDS_Shape s;BRep_Builder b;if(!BRepTools::Read(s,argv[1],b))return 3;TopTools_IndexedMapOfShape fs,es,vs;TopExp::MapShapes(s,TopAbs_FACE,fs);TopExp::MapShapes(s,TopAbs_EDGE,es);TopExp::MapShapes(s,TopAbs_VERTEX,vs);auto f=TopoDS::Face(fs(11));auto e=TopoDS::Edge(es(32));auto v=TopoDS::Vertex(vs(23));BRepAdaptor_Surface surf(f);BRepAdaptor_Curve curve(e);std::cout<<std::setprecision(17)<<"face_surface "<<int(surf.GetType())<<" edge_curve "<<int(curve.GetType())<<" edge_range "<<curve.FirstParameter()<<" "<<curve.LastParameter()<<"\n";auto pv=BRep_Tool::Pnt(v);std::cout<<"vertex "<<pv.X()<<" "<<pv.Y()<<" "<<pv.Z()<<" vertex_tol "<<BRep_Tool::Tolerance(v)<<" face_tol "<<BRep_Tool::Tolerance(f)<<" edge_tol "<<BRep_Tool::Tolerance(e)<<"\n";TopTools_IndexedMapOfShape fe;TopExp::MapShapes(f,TopAbs_EDGE,fe);std::cout<<"edge_is_face_boundary "<<fe.Contains(e)<<"\n";double a,bp;auto pc=BRep_Tool::CurveOnSurface(e,f,a,bp);std::cout<<"edge_pcurve_exists "<<!pc.IsNull()<<" range "<<a<<" "<<bp<<"\n";auto cy=surf.Cylinder();auto cp=cy.Location();auto dir=cy.Axis().Direction();std::cout<<"cylinder "<<cp.X()<<" "<<cp.Y()<<" "<<cp.Z()<<" axis "<<dir.X()<<" "<<dir.Y()<<" "<<dir.Z()<<" radius "<<cy.Radius()<<"\n";for(int i=1;i<=fs.Extent();++i){TopTools_IndexedMapOfShape em;TopExp::MapShapes(fs(i),TopAbs_EDGE,em);if(em.Contains(e))std::cout<<"edge_adjacent_face "<<i<<" type "<<int(BRepAdaptor_Surface(TopoDS::Face(fs(i))).GetType())<<"\n";}if(!pc.IsNull())for(double t:{a,(a+bp)/2,bp}){auto uv=pc->Value(t);auto point=surf.Value(uv.X(),uv.Y());auto world=curve.Value(t);std::cout<<"pcurve "<<t<<" uv "<<uv.X()<<" "<<uv.Y()<<" residual "<<point.Distance(world)<<"\n";}for(auto sh:{TopoDS_Shape(v),TopoDS_Shape(e)}){BRepExtrema_DistShapeShape dist(sh,f);dist.Perform();std::cout<<"distance "<<dist.Value()<<" witnesses "<<dist.NbSolution()<<"\n";for(int i=1;i<=dist.NbSolution()&&i<=6;i++){auto p=dist.PointOnShape1(i),q=dist.PointOnShape2(i);std::cout<<"witness "<<p.X()<<" "<<p.Y()<<" "<<p.Z()<<" / "<<q.X()<<" "<<q.Y()<<" "<<q.Z()<<" support "<<int(dist.SupportTypeShape1(i))<<" "<<int(dist.SupportTypeShape2(i))<<"\n";}}
 auto surface=BRep_Tool::Surface(f);for(double t:{curve.FirstParameter(),(curve.FirstParameter()+curve.LastParameter())/2,curve.LastParameter()}){auto p=curve.Value(t);GeomAPI_ProjectPointOnSurf project(p,surface);std::cout<<"curve_point "<<t<<" "<<p.X()<<" "<<p.Y()<<" "<<p.Z();if(project.NbPoints()){double u,w;project.LowerDistanceParameters(u,w);BRepClass_FaceClassifier classifier(f,gp_Pnt2d(u,w),0.0);std::cout<<" uv "<<u<<" "<<w<<" dist "<<project.LowerDistance()<<" class "<<int(classifier.State());}std::cout<<"\n";}
}
