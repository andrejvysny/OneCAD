#include "SceneMeshStore.h"

namespace onecad::render {

void SceneMeshStore::setBodyMesh(const std::string& bodyId, Mesh mesh) {
    mesh.bodyId = bodyId;
    meshes_[bodyId] = std::move(mesh);
}

bool SceneMeshStore::removeBody(const std::string& bodyId) {
    return meshes_.erase(bodyId) > 0;
}

void SceneMeshStore::clear() {
    meshes_.clear();
}

std::vector<SceneMeshStore::Mesh> SceneMeshStore::meshes() const {
    std::vector<Mesh> result;
    result.reserve(meshes_.size());
    for (const auto& [id, mesh] : meshes_) {
        result.push_back(mesh);
    }
    return result;
}

const SceneMeshStore::Mesh* SceneMeshStore::findMesh(const std::string& bodyId) const {
    auto it = meshes_.find(bodyId);
    if (it == meshes_.end()) {
        return nullptr;
    }
    return &it->second;
}

} // namespace onecad::render
