#include "ModelPickerAdapter.h"

#include <algorithm>
#include <cmath>
#include <limits>

namespace onecad::ui::selection {

namespace {
constexpr int kVertexPriority = 0;
constexpr int kEdgePriority = 1;
constexpr int kFacePriority = 2;
constexpr int kBodyPriority = 3;

std::string vertexIdForIndex(std::uint32_t index) {
    return "v" + std::to_string(index);
}

std::string edgeIdForIndices(std::uint32_t a, std::uint32_t b) {
    if (a > b) {
        std::swap(a, b);
    }
    return "e" + std::to_string(a) + "_" + std::to_string(b);
}

bool rayTriangleIntersect(const QVector3D& origin,
                          const QVector3D& direction,
                          const QVector3D& v0,
                          const QVector3D& v1,
                          const QVector3D& v2,
                          float* outT,
                          QVector3D* outNormal) {
    QVector3D edge1 = v1 - v0;
    QVector3D edge2 = v2 - v0;
    QVector3D pvec = QVector3D::crossProduct(direction, edge2);
    float det = QVector3D::dotProduct(edge1, pvec);
    if (std::abs(det) < 1e-8f) {
        return false;
    }
    float invDet = 1.0f / det;
    QVector3D tvec = origin - v0;
    float u = QVector3D::dotProduct(tvec, pvec) * invDet;
    if (u < 0.0f || u > 1.0f) {
        return false;
    }
    QVector3D qvec = QVector3D::crossProduct(tvec, edge1);
    float v = QVector3D::dotProduct(direction, qvec) * invDet;
    if (v < 0.0f || u + v > 1.0f) {
        return false;
    }
    float t = QVector3D::dotProduct(edge2, qvec) * invDet;
    if (t <= 0.0f) {
        return false;
    }
    if (outT) {
        *outT = t;
    }
    if (outNormal) {
        *outNormal = QVector3D::crossProduct(edge1, edge2).normalized();
    }
    return true;
}

double distancePointToSegment(const QPointF& p, const QPointF& a, const QPointF& b) {
    QPointF ab = b - a;
    double lenSq = ab.x() * ab.x() + ab.y() * ab.y();
    if (lenSq < 1e-6) {
        QPointF diff = p - a;
        return std::sqrt(diff.x() * diff.x() + diff.y() * diff.y());
    }
    double t = ((p.x() - a.x()) * ab.x() + (p.y() - a.y()) * ab.y()) / lenSq;
    t = std::clamp(t, 0.0, 1.0);
    QPointF proj(a.x() + ab.x() * t, a.y() + ab.y() * t);
    QPointF diff = p - proj;
    return std::sqrt(diff.x() * diff.x() + diff.y() * diff.y());
}

} // namespace

void ModelPickerAdapter::setMeshes(std::vector<Mesh>&& meshes) {
    meshes_.clear();
    meshes_.reserve(meshes.size());

    for (auto& mesh : meshes) {
        MeshCache cache;
        cache.bodyId = mesh.bodyId;
        cache.vertices = std::move(mesh.vertices);
        cache.triangles = std::move(mesh.triangles);

        for (const auto& tri : cache.triangles) {
            if (tri.i0 >= cache.vertices.size() ||
                tri.i1 >= cache.vertices.size() ||
                tri.i2 >= cache.vertices.size()) {
                continue;
            }
            std::array<QVector3D, 3> triVerts = {
                cache.vertices[tri.i0],
                cache.vertices[tri.i1],
                cache.vertices[tri.i2]
            };
            cache.faceMap[tri.faceId].push_back(triVerts);
        }

        if (!mesh.topologyByFace.empty()) {
            for (const auto& [faceId, topo] : mesh.topologyByFace) {
                MeshCache::FaceTopologyCache faceCache;

                for (const auto& edge : topo.edges) {
                    if (edge.points.size() < 2) {
                        continue;
                    }
                    if (cache.edgePolylines.find(edge.edgeId) == cache.edgePolylines.end()) {
                        cache.edgePolylines[edge.edgeId] = edge.points;
                    }
                    faceCache.edgeIds.push_back(edge.edgeId);
                }

                for (const auto& vertex : topo.vertices) {
                    if (cache.vertexMap.find(vertex.vertexId) == cache.vertexMap.end()) {
                        cache.vertexMap[vertex.vertexId] = vertex.position;
                    }
                    cache.pickableVertices.insert(vertex.vertexId);
                    faceCache.vertexIds.push_back(vertex.vertexId);
                }

                cache.faceTopology[faceId] = std::move(faceCache);
            }
        } else {
            std::unordered_map<std::string, std::unordered_map<std::string, int>> edgeCountsByFace;
            for (const auto& tri : cache.triangles) {
                if (tri.i0 >= cache.vertices.size() ||
                    tri.i1 >= cache.vertices.size() ||
                    tri.i2 >= cache.vertices.size()) {
                    continue;
                }
                std::array<std::pair<std::uint32_t, std::uint32_t>, 3> edges = {{
                    {tri.i0, tri.i1},
                    {tri.i1, tri.i2},
                    {tri.i2, tri.i0}
                }};
                for (const auto& edge : edges) {
                    std::string edgeId = edgeIdForIndices(edge.first, edge.second);
                    edgeCountsByFace[tri.faceId][edgeId]++;
                }
            }

            for (const auto& [faceId, edges] : edgeCountsByFace) {
                MeshCache::FaceTopologyCache faceCache;
                std::unordered_set<std::string> addedVertices;
                for (const auto& [edgeId, count] : edges) {
                    if (count != 1) {
                        continue;
                    }
                    size_t underscore = edgeId.find('_');
                    if (underscore == std::string::npos) {
                        continue;
                    }
                    std::uint32_t a = static_cast<std::uint32_t>(
                        std::stoul(edgeId.substr(1, underscore - 1)));
                    std::uint32_t b = static_cast<std::uint32_t>(
                        std::stoul(edgeId.substr(underscore + 1)));
                    if (static_cast<size_t>(a) >= cache.vertices.size() ||
                        static_cast<size_t>(b) >= cache.vertices.size()) {
                        continue;
                    }
                    std::vector<QVector3D> polyline = {cache.vertices[a], cache.vertices[b]};
                    if (cache.edgePolylines.find(edgeId) == cache.edgePolylines.end()) {
                        cache.edgePolylines[edgeId] = polyline;
                    }
                    faceCache.edgeIds.push_back(edgeId);

                    std::string vA = vertexIdForIndex(a);
                    std::string vB = vertexIdForIndex(b);
                    cache.vertexMap[vA] = cache.vertices[a];
                    cache.vertexMap[vB] = cache.vertices[b];
                    cache.pickableVertices.insert(vA);
                    cache.pickableVertices.insert(vB);
                    if (addedVertices.insert(vA).second) {
                        faceCache.vertexIds.push_back(vA);
                    }
                    if (addedVertices.insert(vB).second) {
                        faceCache.vertexIds.push_back(vB);
                    }
                }
                cache.faceTopology[faceId] = std::move(faceCache);
            }
        }

        cache.faceGroupLeaderByFaceId = std::move(mesh.faceGroupByFaceId);
        if (cache.faceGroupLeaderByFaceId.empty()) {
            for (const auto& [faceId, tris] : cache.faceMap) {
                (void)tris;
                cache.faceGroupLeaderByFaceId[faceId] = faceId;
            }
        } else {
            for (const auto& [faceId, tris] : cache.faceMap) {
                (void)tris;
                if (cache.faceGroupLeaderByFaceId.find(faceId) == cache.faceGroupLeaderByFaceId.end()) {
                    cache.faceGroupLeaderByFaceId[faceId] = faceId;
                }
            }
        }
        for (const auto& [faceId, leaderId] : cache.faceGroupLeaderByFaceId) {
            cache.faceGroupMembers[leaderId].push_back(faceId);
        }

        meshes_.push_back(std::move(cache));
    }
}

app::selection::PickResult ModelPickerAdapter::pick(const QPoint& screenPos,
                                                    double tolerancePixels,
                                                    const QMatrix4x4& viewProjection,
                                                    const QSize& viewportSize) const {
    app::selection::PickResult result;
    if (meshes_.empty()) {
        return result;
    }

    Ray ray = buildRay(screenPos, viewProjection, viewportSize);
    if (!ray.valid) {
        return result;
    }
    struct FaceHit {
        const MeshCache* mesh = nullptr;
        Triangle triangle;
        QVector3D normal;
        QVector3D point;
        float t = 0.0f;
    };

    std::vector<FaceHit> faceHits;
    faceHits.reserve(16);
    std::unordered_map<std::string, size_t> faceIndex;

    for (const auto& mesh : meshes_) {
        for (const auto& tri : mesh.triangles) {
            if (tri.i0 >= mesh.vertices.size() ||
                tri.i1 >= mesh.vertices.size() ||
                tri.i2 >= mesh.vertices.size()) {
                continue;
            }
            const QVector3D& v0 = mesh.vertices[tri.i0];
            const QVector3D& v1 = mesh.vertices[tri.i1];
            const QVector3D& v2 = mesh.vertices[tri.i2];
            float t = 0.0f;
            QVector3D normal;
            if (!rayTriangleIntersect(ray.origin, ray.direction, v0, v1, v2, &t, &normal)) {
                continue;
            }
            std::string key = mesh.bodyId + ":" + tri.faceId;
            auto it = faceIndex.find(key);
            if (it == faceIndex.end()) {
                FaceHit hit;
                hit.mesh = &mesh;
                hit.triangle = tri;
                hit.normal = normal;
                hit.point = ray.origin + ray.direction * t;
                hit.t = t;
                faceIndex[key] = faceHits.size();
                faceHits.push_back(hit);
            } else {
                FaceHit& hit = faceHits[it->second];
                if (t < hit.t) {
                    hit.triangle = tri;
                    hit.normal = normal;
                    hit.point = ray.origin + ray.direction * t;
                    hit.t = t;
                }
            }
        }
    }

    if (faceHits.empty()) {
        return result;
    }

    std::sort(faceHits.begin(), faceHits.end(), [](const FaceHit& a, const FaceHit& b) {
        return a.t < b.t;
    });

    const FaceHit& frontHit = faceHits.front();
    const MeshCache* hitMesh = frontHit.mesh;
    const Triangle& hitTriangle = frontHit.triangle;

    // Filter occluded faces from candidates
    // We only consider faces that are very close to the front-most hit (e.g. coincident faces)
    // Adjust epsilon based on scene scale if needed.
    constexpr float kDepthEpsilon = 1e-4f;
    float minT = frontHit.t;

    std::vector<FaceHit> visibleHits;
    visibleHits.reserve(faceHits.size());
    for (const auto& hit : faceHits) {
        if (hit.t <= minT + kDepthEpsilon) {
            visibleHits.push_back(hit);
        }
    }

    QPointF clickPoint(screenPos);
    double bestVertexDistance = std::numeric_limits<double>::max();
    std::string bestVertexId;
    QVector3D bestVertexPos;
    double bestEdgeDistance = std::numeric_limits<double>::max();
    std::string bestEdgeId;
    QVector3D bestEdgeMid;
    bool usedTopology = false;
    auto topoIt = hitMesh->faceTopology.find(hitTriangle.faceId);
    if (topoIt != hitMesh->faceTopology.end()) {
        const auto& topo = topoIt->second;
        if (!topo.vertexIds.empty() || !topo.edgeIds.empty()) {
            usedTopology = true;

            for (const auto& vertexId : topo.vertexIds) {
                auto it = hitMesh->vertexMap.find(vertexId);
                if (it == hitMesh->vertexMap.end()) {
                    continue;
                }
                QPointF projectedPos;
                if (!projectToScreen(viewProjection, it->second, viewportSize, &projectedPos)) {
                    continue;
                }
                double dist = std::hypot(clickPoint.x() - projectedPos.x(), clickPoint.y() - projectedPos.y());
                if (dist < bestVertexDistance) {
                    bestVertexDistance = dist;
                    bestVertexId = vertexId;
                    bestVertexPos = it->second;
                }
            }

            for (const auto& edgeId : topo.edgeIds) {
                auto polyIt = hitMesh->edgePolylines.find(edgeId);
                if (polyIt == hitMesh->edgePolylines.end() || polyIt->second.size() < 2) {
                    continue;
                }
                const auto& points = polyIt->second;
                for (size_t i = 0; i + 1 < points.size(); ++i) {
                    QPointF a;
                    QPointF b;
                    if (!projectToScreen(viewProjection, points[i], viewportSize, &a)) {
                        continue;
                    }
                    if (!projectToScreen(viewProjection, points[i + 1], viewportSize, &b)) {
                        continue;
                    }
                    double dist = distancePointToSegment(clickPoint, a, b);
                    if (dist < bestEdgeDistance) {
                        bestEdgeDistance = dist;
                        bestEdgeId = edgeId;
                        bestEdgeMid = (points[i] + points[i + 1]) * 0.5f;
                    }
                }
            }
        }
    }

    if (!usedTopology) {
        auto vertexA = hitMesh->vertices[hitTriangle.i0];
        auto vertexB = hitMesh->vertices[hitTriangle.i1];
        auto vertexC = hitMesh->vertices[hitTriangle.i2];

        QPointF screenA, screenB, screenC;
        bool projA = projectToScreen(viewProjection, vertexA, viewportSize, &screenA);
        bool projB = projectToScreen(viewProjection, vertexB, viewportSize, &screenB);
        bool projC = projectToScreen(viewProjection, vertexC, viewportSize, &screenC);

        bool restrictVertices = !hitMesh->pickableVertices.empty();
        auto canPickVertex = [&](const std::string& id) {
            return !restrictVertices || hitMesh->pickableVertices.find(id) != hitMesh->pickableVertices.end();
        };

        if (projA) {
            double dist = std::hypot(clickPoint.x() - screenA.x(), clickPoint.y() - screenA.y());
            if (dist < bestVertexDistance && canPickVertex(vertexIdForIndex(hitTriangle.i0))) {
                bestVertexDistance = dist;
                bestVertexId = vertexIdForIndex(hitTriangle.i0);
                bestVertexPos = vertexA;
            }
        }
        if (projB) {
            double dist = std::hypot(clickPoint.x() - screenB.x(), clickPoint.y() - screenB.y());
            if (dist < bestVertexDistance && canPickVertex(vertexIdForIndex(hitTriangle.i1))) {
                bestVertexDistance = dist;
                bestVertexId = vertexIdForIndex(hitTriangle.i1);
                bestVertexPos = vertexB;
            }
        }
        if (projC) {
            double dist = std::hypot(clickPoint.x() - screenC.x(), clickPoint.y() - screenC.y());
            if (dist < bestVertexDistance && canPickVertex(vertexIdForIndex(hitTriangle.i2))) {
                bestVertexDistance = dist;
                bestVertexId = vertexIdForIndex(hitTriangle.i2);
                bestVertexPos = vertexC;
            }
        }

        if (projA && projB) {
            double dist = distancePointToSegment(clickPoint, screenA, screenB);
            if (dist < bestEdgeDistance) {
                std::string edgeId = edgeIdForIndices(hitTriangle.i0, hitTriangle.i1);
                if (hitMesh->edgePolylines.find(edgeId) != hitMesh->edgePolylines.end()) {
                    bestEdgeDistance = dist;
                    bestEdgeId = edgeId;
                    bestEdgeMid = (vertexA + vertexB) * 0.5f;
                }
            }
        }
        if (projB && projC) {
            double dist = distancePointToSegment(clickPoint, screenB, screenC);
            if (dist < bestEdgeDistance) {
                std::string edgeId = edgeIdForIndices(hitTriangle.i1, hitTriangle.i2);
                if (hitMesh->edgePolylines.find(edgeId) != hitMesh->edgePolylines.end()) {
                    bestEdgeDistance = dist;
                    bestEdgeId = edgeId;
                    bestEdgeMid = (vertexB + vertexC) * 0.5f;
                }
            }
        }
        if (projC && projA) {
            double dist = distancePointToSegment(clickPoint, screenC, screenA);
            if (dist < bestEdgeDistance) {
                std::string edgeId = edgeIdForIndices(hitTriangle.i2, hitTriangle.i0);
                if (hitMesh->edgePolylines.find(edgeId) != hitMesh->edgePolylines.end()) {
                    bestEdgeDistance = dist;
                    bestEdgeId = edgeId;
                    bestEdgeMid = (vertexC + vertexA) * 0.5f;
                }
            }
        }
    }

    if (!bestVertexId.empty() && bestVertexDistance <= tolerancePixels) {
        app::selection::SelectionItem item;
        item.kind = app::selection::SelectionKind::Vertex;
        item.id = {hitMesh->bodyId, bestVertexId};
        item.priority = kVertexPriority;
        item.screenDistance = bestVertexDistance;
        item.depth = static_cast<double>(frontHit.t);
        item.worldPos = {bestVertexPos.x(), bestVertexPos.y(), bestVertexPos.z()};
        result.hits.push_back(item);
    } else if (!bestEdgeId.empty() && bestEdgeDistance <= tolerancePixels) {
        app::selection::SelectionItem item;
        item.kind = app::selection::SelectionKind::Edge;
        item.id = {hitMesh->bodyId, bestEdgeId};
        item.priority = kEdgePriority;
        item.screenDistance = bestEdgeDistance;
        item.depth = static_cast<double>(frontHit.t);
        item.worldPos = {bestEdgeMid.x(), bestEdgeMid.y(), bestEdgeMid.z()};
        result.hits.push_back(item);
    }

    std::unordered_map<std::string, float> bodyDepths;
    std::unordered_map<std::string, QVector3D> bodyPoints;
    std::unordered_map<std::string, QVector3D> bodyNormals;

    for (const auto& hit : visibleHits) {
        app::selection::SelectionItem faceItem;
        faceItem.kind = app::selection::SelectionKind::Face;
        std::string faceId = hit.triangle.faceId;
        auto groupIt = hit.mesh->faceGroupLeaderByFaceId.find(faceId);
        if (groupIt != hit.mesh->faceGroupLeaderByFaceId.end()) {
            faceId = groupIt->second;
        }
        faceItem.id = {hit.mesh->bodyId, faceId};
        faceItem.priority = kFacePriority;
        faceItem.screenDistance = 0.0;
        faceItem.depth = static_cast<double>(hit.t);
        faceItem.worldPos = {hit.point.x(), hit.point.y(), hit.point.z()};
        faceItem.normal = {hit.normal.x(), hit.normal.y(), hit.normal.z()};
        result.hits.push_back(faceItem);

        auto bodyIt = bodyDepths.find(hit.mesh->bodyId);
        if (bodyIt == bodyDepths.end() || hit.t < bodyIt->second) {
            bodyDepths[hit.mesh->bodyId] = hit.t;
            bodyPoints[hit.mesh->bodyId] = hit.point;
            bodyNormals[hit.mesh->bodyId] = hit.normal;
        }
    }

    for (const auto& [bodyId, depth] : bodyDepths) {
        app::selection::SelectionItem bodyItem;
        bodyItem.kind = app::selection::SelectionKind::Body;
        bodyItem.id = {bodyId, bodyId};
        bodyItem.priority = kBodyPriority;
        bodyItem.screenDistance = 0.0;
        bodyItem.depth = static_cast<double>(depth);
        const QVector3D& point = bodyPoints[bodyId];
        const QVector3D& normal = bodyNormals[bodyId];
        bodyItem.worldPos = {point.x(), point.y(), point.z()};
        bodyItem.normal = {normal.x(), normal.y(), normal.z()};
        result.hits.push_back(bodyItem);
    }

    return result;
}

bool ModelPickerAdapter::getFaceTriangles(const std::string& bodyId,
                                          const std::string& faceId,
                                          std::vector<std::array<QVector3D, 3>>& outTriangles) const {
    for (const auto& mesh : meshes_) {
        if (mesh.bodyId != bodyId) {
            continue;
        }
        std::string groupId = faceId;
        auto groupIt = mesh.faceGroupLeaderByFaceId.find(faceId);
        if (groupIt != mesh.faceGroupLeaderByFaceId.end()) {
            groupId = groupIt->second;
        }
        outTriangles.clear();
        auto membersIt = mesh.faceGroupMembers.find(groupId);
        if (membersIt != mesh.faceGroupMembers.end()) {
            for (const auto& memberId : membersIt->second) {
                auto it = mesh.faceMap.find(memberId);
                if (it != mesh.faceMap.end()) {
                    outTriangles.insert(outTriangles.end(), it->second.begin(), it->second.end());
                }
            }
            return !outTriangles.empty();
        }
        auto it = mesh.faceMap.find(faceId);
        if (it != mesh.faceMap.end()) {
            outTriangles = it->second;
            return true;
        }
        return false;
    }
    return false;
}

bool ModelPickerAdapter::getBodyTriangles(const std::string& bodyId,
                                          std::vector<std::array<QVector3D, 3>>& outTriangles) const {
    for (const auto& mesh : meshes_) {
        if (mesh.bodyId != bodyId) {
            continue;
        }
        outTriangles.clear();
        for (const auto& [faceId, tris] : mesh.faceMap) {
            (void)faceId;
            outTriangles.insert(outTriangles.end(), tris.begin(), tris.end());
        }
        return !outTriangles.empty();
    }
    return false;
}

bool ModelPickerAdapter::getEdgeSegment(const std::string& bodyId,
                                        const std::string& edgeId,
                                        std::array<QVector3D, 2>& outSegment) const {
    for (const auto& mesh : meshes_) {
        if (mesh.bodyId != bodyId) {
            continue;
        }
        auto it = mesh.edgePolylines.find(edgeId);
        if (it == mesh.edgePolylines.end() || it->second.size() < 2) {
            return false;
        }
        outSegment = {it->second.front(), it->second.back()};
        return true;
    }
    return false;
}

bool ModelPickerAdapter::getEdgePolyline(const std::string& bodyId,
                                         const std::string& edgeId,
                                         std::vector<QVector3D>& outPolyline) const {
    for (const auto& mesh : meshes_) {
        if (mesh.bodyId != bodyId) {
            continue;
        }
        auto it = mesh.edgePolylines.find(edgeId);
        if (it == mesh.edgePolylines.end()) {
            return false;
        }
        outPolyline = it->second;
        return true;
    }
    return false;
}

bool ModelPickerAdapter::getVertexPosition(const std::string& bodyId,
                                           const std::string& vertexId,
                                           QVector3D& outVertex) const {
    for (const auto& mesh : meshes_) {
        if (mesh.bodyId != bodyId) {
            continue;
        }
        auto it = mesh.vertexMap.find(vertexId);
        if (it == mesh.vertexMap.end()) {
            return false;
        }
        outVertex = it->second;
        return true;
    }
    return false;
}

bool ModelPickerAdapter::getFaceBoundaryEdges(const std::string& bodyId,
                                               const std::string& faceId,
                                               std::vector<std::vector<QVector3D>>& outEdges) const {
    for (const auto& mesh : meshes_) {
        if (mesh.bodyId != bodyId) {
            continue;
        }
        std::string groupId = faceId;
        auto groupIt = mesh.faceGroupLeaderByFaceId.find(faceId);
        if (groupIt != mesh.faceGroupLeaderByFaceId.end()) {
            groupId = groupIt->second;
        }
        outEdges.clear();
        std::unordered_set<std::string> seenEdges;
        auto membersIt = mesh.faceGroupMembers.find(groupId);
        if (membersIt != mesh.faceGroupMembers.end()) {
            for (const auto& memberId : membersIt->second) {
                auto topoIt = mesh.faceTopology.find(memberId);
                if (topoIt == mesh.faceTopology.end()) {
                    continue;
                }
                for (const auto& edgeId : topoIt->second.edgeIds) {
                    if (!seenEdges.insert(edgeId).second) {
                        continue;
                    }
                    auto polyIt = mesh.edgePolylines.find(edgeId);
                    if (polyIt != mesh.edgePolylines.end() && polyIt->second.size() >= 2) {
                        outEdges.push_back(polyIt->second);
                    }
                }
            }
            return !outEdges.empty();
        }
        auto topoIt = mesh.faceTopology.find(faceId);
        if (topoIt == mesh.faceTopology.end()) {
            return false;
        }
        for (const auto& edgeId : topoIt->second.edgeIds) {
            auto polyIt = mesh.edgePolylines.find(edgeId);
            if (polyIt != mesh.edgePolylines.end() && polyIt->second.size() >= 2) {
                outEdges.push_back(polyIt->second);
            }
        }
        return !outEdges.empty();
    }
    return false;
}

ModelPickerAdapter::Ray ModelPickerAdapter::buildRay(const QPoint& screenPos,
                                                     const QMatrix4x4& viewProjection,
                                                     const QSize& viewportSize) const {
    Ray ray;
    if (viewportSize.width() <= 0 || viewportSize.height() <= 0) {
        ray.origin = QVector3D();
        ray.direction = QVector3D();
        ray.valid = false;
        return ray;
    }

    bool invertible = false;
    QMatrix4x4 invViewProj = viewProjection.inverted(&invertible);
    if (!invertible) {
        ray.origin = QVector3D();
        ray.direction = QVector3D();
        ray.valid = false;
        return ray;
    }

    float ndcX = (2.0f * screenPos.x() / viewportSize.width()) - 1.0f;
    float ndcY = 1.0f - (2.0f * screenPos.y() / viewportSize.height());

    QVector4D nearPoint = invViewProj * QVector4D(ndcX, ndcY, -1.0f, 1.0f);
    QVector4D farPoint = invViewProj * QVector4D(ndcX, ndcY, 1.0f, 1.0f);

    if (std::abs(nearPoint.w()) < 1e-6f || std::abs(farPoint.w()) < 1e-6f) {
        ray.origin = QVector3D();
        ray.direction = QVector3D();
        ray.valid = false;
        return ray;
    }

    QVector3D origin = nearPoint.toVector3D() / nearPoint.w();
    QVector3D farPos = farPoint.toVector3D() / farPoint.w();
    ray.origin = origin;
    ray.direction = (farPos - origin).normalized();
    ray.valid = true;
    return ray;
}

bool ModelPickerAdapter::projectToScreen(const QMatrix4x4& viewProjection,
                                         const QVector3D& worldPos,
                                         const QSize& viewportSize,
                                         QPointF* outPos) const {
    QVector4D clip = viewProjection * QVector4D(worldPos, 1.0f);
    if (clip.w() <= 1e-6f) {
        return false;
    }
    QVector3D ndc = clip.toVector3D() / clip.w();
    float x = (ndc.x() * 0.5f + 0.5f) * viewportSize.width();
    float y = (1.0f - (ndc.y() * 0.5f + 0.5f)) * viewportSize.height();
    *outPos = QPointF(x, y);
    return true;
}

} // namespace onecad::ui::selection
