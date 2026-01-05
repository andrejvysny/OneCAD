#include "TessellationCache.h"

#include <BRepMesh_IncrementalMesh.hxx>
#include <BRep_Tool.hxx>
#include <Poly_Triangulation.hxx>
#include <BRepAdaptor_Curve.hxx>
#include <BRepTools_WireExplorer.hxx>
#include <GCPnts_AbscissaPoint.hxx>
#include <GCPnts_UniformAbscissa.hxx>
#include <Standard_Failure.hxx>
#include <TopExp_Explorer.hxx>
#include <TopExp.hxx>
#include <TopLoc_Location.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Wire.hxx>
#include <TopoDS_Vertex.hxx>
#include <TopTools_ShapeMapHasher.hxx>

#include <limits>
#include <unordered_map>
#include <unordered_set>

namespace onecad::render {

SceneMeshStore::Mesh TessellationCache::buildMesh(const std::string& bodyId,
                                                  const TopoDS_Shape& shape,
                                                  kernel::elementmap::ElementMap& elementMap) const {
    SceneMeshStore::Mesh mesh;
    mesh.bodyId = bodyId;
    mesh.modelMatrix.setToIdentity();

    if (shape.IsNull()) {
        return mesh;
    }

    BRepMesh_IncrementalMesh mesher(shape, settings_.linearDeflection,
                                    settings_.parallel, settings_.angularDeflection, true);
    mesher.Perform();
    if (!mesher.IsDone()) {
        return mesh;
    }

    for (TopExp_Explorer faceExp(shape, TopAbs_FACE); faceExp.More(); faceExp.Next()) {
        TopoDS_Face face = TopoDS::Face(faceExp.Current());
        TopLoc_Location location;
        Handle(Poly_Triangulation) triangulation = BRep_Tool::Triangulation(face, location);
        if (triangulation.IsNull()) {
            continue;
        }

        std::string faceId;
        auto ids = elementMap.findIdsByShape(face);
        if (!ids.empty()) {
            faceId = ids.front().value;
        }

        if (faceId.empty()) {
            faceId = bodyId + "/face/unknown_" + std::to_string(mesh.triangles.size());
        }

        SceneMeshStore::FaceTopology topology = buildFaceTopology(bodyId, face, elementMap);
        topology.faceId = faceId;
        mesh.topologyByFace[faceId] = std::move(topology);

        const gp_Trsf& trsf = location.Transformation();
        std::uint32_t nodeOffset = static_cast<std::uint32_t>(mesh.vertices.size());
        int nodeCount = triangulation->NbNodes();
        mesh.vertices.reserve(mesh.vertices.size() + nodeCount);
        for (int i = 1; i <= nodeCount; ++i) {
            gp_Pnt p = triangulation->Node(i).Transformed(trsf);
            mesh.vertices.emplace_back(static_cast<float>(p.X()),
                                       static_cast<float>(p.Y()),
                                       static_cast<float>(p.Z()));
        }

        int triCount = triangulation->NbTriangles();
        mesh.triangles.reserve(mesh.triangles.size() + triCount);
        for (int i = 1; i <= triCount; ++i) {
            int n1 = 0;
            int n2 = 0;
            int n3 = 0;
            triangulation->Triangle(i).Get(n1, n2, n3);
            SceneMeshStore::Triangle tri;
            tri.i0 = nodeOffset + static_cast<std::uint32_t>(n1 - 1);
            tri.i1 = nodeOffset + static_cast<std::uint32_t>(n2 - 1);
            tri.i2 = nodeOffset + static_cast<std::uint32_t>(n3 - 1);
            tri.faceId = faceId;
            mesh.triangles.push_back(tri);
        }
    }

    return mesh;
}

SceneMeshStore::FaceTopology TessellationCache::buildFaceTopology(
    const std::string& bodyId,
    const TopoDS_Face& face,
    kernel::elementmap::ElementMap& elementMap) const {
    SceneMeshStore::FaceTopology topology;

    std::unordered_set<std::string> seenEdges;
    std::unordered_set<std::string> seenVertices;
    std::unordered_map<TopoDS_Shape, std::string, TopTools_ShapeMapHasher, TopTools_ShapeMapHasher>
        generatedEdgeIds;
    std::unordered_map<TopoDS_Shape, std::string, TopTools_ShapeMapHasher, TopTools_ShapeMapHasher>
        generatedVertexIds;
    int unknownEdgeCount = 0;
    int unknownVertexCount = 0;

    for (TopExp_Explorer wireExp(face, TopAbs_WIRE); wireExp.More(); wireExp.Next()) {
        TopoDS_Wire wire = TopoDS::Wire(wireExp.Current());
        for (BRepTools_WireExplorer edgeExp(wire, face); edgeExp.More(); edgeExp.Next()) {
            TopoDS_Edge edge = edgeExp.Current();

            std::string edgeId;
            auto edgeIds = elementMap.findIdsByShape(edge);
            if (!edgeIds.empty()) {
                edgeId = edgeIds.front().value;
            }
            if (edgeId.empty()) {
                auto it = generatedEdgeIds.find(edge);
                if (it != generatedEdgeIds.end()) {
                    edgeId = it->second;
                } else {
                    edgeId = bodyId + "/edge/unknown_" + std::to_string(unknownEdgeCount++);
                    generatedEdgeIds.emplace(edge, edgeId);
                }
            }

            if (seenEdges.find(edgeId) == seenEdges.end()) {
                SceneMeshStore::EdgePolyline polyline;
                polyline.edgeId = edgeId;

                BRepAdaptor_Curve curve(edge);
                double first = curve.FirstParameter();
                double last = curve.LastParameter();
                double length = 0.0;
                try {
                    length = GCPnts_AbscissaPoint::Length(curve, first, last);
                } catch (const Standard_Failure&) {
                    length = 0.0;
                }
                double step = std::max(settings_.linearDeflection * 2.0, 0.1);
                int segments = std::max(2, static_cast<int>(std::ceil(length / step)));

                GCPnts_UniformAbscissa abscissa(curve, segments);
                if (abscissa.IsDone() && abscissa.NbPoints() > 1) {
                    for (int i = 1; i <= abscissa.NbPoints(); ++i) {
                        double param = abscissa.Parameter(i);
                        gp_Pnt point = curve.Value(param);
                        polyline.points.emplace_back(static_cast<float>(point.X()),
                                                     static_cast<float>(point.Y()),
                                                     static_cast<float>(point.Z()));
                    }
                } else {
                    gp_Pnt p1 = curve.Value(first);
                    gp_Pnt p2 = curve.Value(last);
                    polyline.points.emplace_back(static_cast<float>(p1.X()),
                                                 static_cast<float>(p1.Y()),
                                                 static_cast<float>(p1.Z()));
                    polyline.points.emplace_back(static_cast<float>(p2.X()),
                                                 static_cast<float>(p2.Y()),
                                                 static_cast<float>(p2.Z()));
                }

                if (polyline.points.size() >= 2) {
                    topology.edges.push_back(std::move(polyline));
                    seenEdges.insert(edgeId);
                }
            }

            TopoDS_Vertex v1;
            TopoDS_Vertex v2;
            TopExp::Vertices(edge, v1, v2);
            TopoDS_Vertex vertices[2] = {v1, v2};
            for (const auto& vertex : vertices) {
                if (vertex.IsNull()) {
                    continue;
                }
                std::string vertexId;
                auto vertexIds = elementMap.findIdsByShape(vertex);
                if (!vertexIds.empty()) {
                    vertexId = vertexIds.front().value;
                }
                if (vertexId.empty()) {
                    auto it = generatedVertexIds.find(vertex);
                    if (it != generatedVertexIds.end()) {
                        vertexId = it->second;
                    } else {
                        vertexId = bodyId + "/vertex/unknown_" + std::to_string(unknownVertexCount++);
                        generatedVertexIds.emplace(vertex, vertexId);
                    }
                }
                if (seenVertices.find(vertexId) != seenVertices.end()) {
                    continue;
                }
                gp_Pnt point = BRep_Tool::Pnt(vertex);
                if (!vertex.Location().IsIdentity()) {
                    point.Transform(vertex.Location().Transformation());
                }
                SceneMeshStore::VertexSample sample;
                sample.vertexId = vertexId;
                sample.position = QVector3D(static_cast<float>(point.X()),
                                            static_cast<float>(point.Y()),
                                            static_cast<float>(point.Z()));
                topology.vertices.push_back(sample);
                seenVertices.insert(vertexId);
            }
        }
    }

    return topology;
}

} // namespace onecad::render
