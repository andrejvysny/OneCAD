#pragma once

#include <cstddef>
#include <string>
#include <span>
#include <vector>

#include <TopoDS_Shape.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>

namespace onecad::io {

// Match the existing unique-triangle admission ceiling; identity is optional
// and is refused when its source topology exceeds this bounded envelope.
inline constexpr std::size_t kMaxSourceFaceClaims = 500000;

// A claim is valid only for this exact captured blob and reader context. The
// shape is an in-memory adapter; neither it nor a traversal ordinal is an address.
struct SourceFaceClaim {
    SourceFaceClaim() = default;
    SourceFaceClaim(const SourceFaceClaim&) = delete;
    SourceFaceClaim& operator=(const SourceFaceClaim&) = delete;
    SourceFaceClaim(SourceFaceClaim&&) noexcept = default;
    SourceFaceClaim& operator=(SourceFaceClaim&&) noexcept = default;
    TopoDS_Shape face;
    std::string source_sha256;
    std::string codec;
    std::string codec_version;
    std::string reader_policy;
    std::string kernel_fingerprint;
    std::string source_unit;
    std::string target_unit;
    enum class AddressKind { StepEntity, XbfLabel };
    AddressKind address_kind = AddressKind::StepEntity;
    int step_root_entity = 0;
    int step_face_entity = 0;
    std::vector<int> step_occurrence_path;
    std::string xbf_root_label;
    std::string xbf_face_label;
    std::vector<std::string> xbf_occurrence_path;
    std::string producer_id;
    std::string body_id;
    double unit_scale = 1.0;
    bool source_face_replaced = false;
};

inline bool oriented_source_member(const TopTools_IndexedMapOfShape& faces, const TopoDS_Shape& face) {
    if (face.IsNull() || face.ShapeType() != TopAbs_FACE) return false;
    const int index = faces.FindIndex(face);
    return index > 0 && face.IsEqual(faces(index));
}

// A source stage is usable only if every face has one oriented source image.
inline bool complete_source_faces(std::span<const SourceFaceClaim> claims, const TopoDS_Shape& shape) {
    if (claims.empty()) return true;
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(shape, TopAbs_FACE, faces);
    if (claims.size() != static_cast<std::size_t>(faces.Extent())) return false;
    std::vector<bool> seen(claims.size() + 1, false);
    for (const auto& claim : claims) {
        if (claim.face.IsNull() || claim.face.ShapeType() != TopAbs_FACE) return false;
        const int index = faces.FindIndex(claim.face);
        if (index == 0 || seen[index] || !claim.face.IsEqual(faces(index))) return false;
        seen[index] = true;
    }
    return true;
}

inline void retain_complete_source_faces(std::vector<SourceFaceClaim>& claims,
                                         const TopoDS_Shape& shape) {
    if (!complete_source_faces(claims, shape)) claims.clear();
}

}  // namespace onecad::io
