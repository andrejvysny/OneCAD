// Ported from OneCAD-CPP src/kernel/topology/CoplanarFacePatch.h @ b4ddcccc (2026-07-16)
/**
 * @file CoplanarFacePatch.h
 * @brief Utilities for connected coplanar face patch extraction.
 */
#ifndef ONECAD_KERNEL_TOPOLOGY_COPLANARFACEPATCH_H
#define ONECAD_KERNEL_TOPOLOGY_COPLANARFACEPATCH_H

#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Dir.hxx>
#include <gp_Pln.hxx>

#include <vector>

namespace onecad::core::modeling {

class CoplanarFacePatch {
public:
    struct Options {
        double normalDotTolerance = 0.9999;
        double planeDistanceTolerance = 1e-3;
    };

    /**
     * @brief Collect connected planar faces sharing seed plane orientation and distance.
     */
    static std::vector<TopoDS_Face> collectConnectedFaces(const TopoDS_Shape& body,
                                                          const TopoDS_Face& seedFace,
                                                          const Options& options);

    /**
     * @brief Collect connected planar faces with default tolerances.
     */
    static std::vector<TopoDS_Face> collectConnectedFaces(const TopoDS_Shape& body,
                                                          const TopoDS_Face& seedFace);

    /**
     * @brief Collect EVERY planar face of `shape` coplanar with `plane`.
     *
     * The SAME predicate and the SAME defaults as collectConnectedFaces
     * (|n·n'| >= normalDotTolerance, |n·(p'−p)| <= planeDistanceTolerance) over an
     * ALL-FACES scan in TopExp_Explorer(shape, TopAbs_FACE) order, instead of an
     * edge-adjacency BFS. Required by face-boundary projection: a U-shaped body's
     * two prong tops are coplanar but NOT edge-connected, so a BFS finds only the
     * seed's own prong (SKETCH-ON-FACE decision D4').
     *
     * Faces come back VERBATIM — deduped by `IsSame`, explorer order, and with NO
     * orientation alignment. The BFS variant reverses a neighbour whose normal
     * opposes the seed because its consumer FUSES the patch; this one's consumer
     * WALKS WIRES, and reversing a face flips `BRepTools_WireExplorer` traversal —
     * which would make boundary emission order depend on face orientation.
     */
    static std::vector<TopoDS_Face> collectCoplanarFaces(const TopoDS_Shape& shape,
                                                         const gp_Pln& plane,
                                                         const Options& options);

    /**
     * @brief The face's exact plane + its orientation-corrected unit normal.
     *
     * Returns false for a null or non-planar face. The single planar-frame
     * extractor shared by the patch scan and the face-boundary projector (it was
     * this TU's file-local `planarFaceData`, hoisted rather than copied).
     */
    static bool planarFacePlaneAndNormal(const TopoDS_Face& face, gp_Pln& planeOut,
                                         gp_Dir& normalOut);

    /**
     * @brief Build a shape representing a patch from a set of faces.
     */
    static TopoDS_Shape makeFaceCompound(const std::vector<TopoDS_Face>& faces);
};

} // namespace onecad::core::modeling

#endif // ONECAD_KERNEL_TOPOLOGY_COPLANARFACEPATCH_H
