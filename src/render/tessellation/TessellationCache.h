#ifndef ONECAD_RENDER_TESSELLATION_TESSELLATIONCACHE_H
#define ONECAD_RENDER_TESSELLATION_TESSELLATIONCACHE_H

#include "../scene/SceneMeshStore.h"
#include "../../kernel/elementmap/ElementMap.h"

#include <TopoDS_Shape.hxx>
#include <TopoDS_Face.hxx>

#include <string>

namespace onecad::render {

class TessellationCache {
public:
    struct Settings {
        double linearDeflection = 0.1;
        double angularDeflection = 0.5;
        bool parallel = true;
    };

    TessellationCache() = default;

    void setSettings(const Settings& settings) { settings_ = settings; }
    const Settings& settings() const { return settings_; }

    SceneMeshStore::Mesh buildMesh(const std::string& bodyId,
                                   const TopoDS_Shape& shape,
                                   kernel::elementmap::ElementMap& elementMap) const;

private:
    SceneMeshStore::FaceTopology buildFaceTopology(const std::string& bodyId,
                                                   const TopoDS_Face& face,
                                                   kernel::elementmap::ElementMap& elementMap) const;
    Settings settings_{};
};

} // namespace onecad::render

#endif // ONECAD_RENDER_TESSELLATION_TESSELLATIONCACHE_H
