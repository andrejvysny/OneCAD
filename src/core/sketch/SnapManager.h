/**
 * @file SnapManager.h
 * @brief Snap system for precision drawing in sketch mode
 *
 * Per SPECIFICATION.md §5.14:
 * - 2mm snap radius (constant in sketch coordinates)
 * - Priority order: Vertex > Endpoint > Midpoint > Center > Quadrant > Intersection > OnCurve > Grid
 * - Visual feedback via cursor change to snap icon
 */
#ifndef ONECAD_CORE_SKETCH_SNAP_MANAGER_H
#define ONECAD_CORE_SKETCH_SNAP_MANAGER_H

#include "SketchTypes.h"
#include <optional>
#include <unordered_map>
#include <unordered_set>
#include <vector>

namespace onecad::core::sketch {

class Sketch;
class SketchEntity;
class SketchPoint;
class SketchLine;
class SketchArc;
class SketchCircle;

/**
 * @brief Snap type enumeration with priority ordering
 *
 * Lower enum value = higher priority
 */
enum class SnapType {
    None = 0,
    Vertex,       // Snap to existing point (highest priority)
    Endpoint,     // Snap to line/arc endpoint
    Midpoint,     // Snap to line midpoint
    Center,       // Snap to arc/circle center
    Quadrant,     // Snap to circle quadrant points (0, 90, 180, 270 deg)
    Intersection, // Snap to intersection of two entities
    OnCurve,      // Snap to nearest point on curve
    Grid,         // Snap to grid (lowest priority)
    Perpendicular,// Perpendicular inference
    Tangent,      // Tangent inference
    Horizontal,   // Horizontal alignment inference
    Vertical,     // Vertical alignment inference
    SketchGuide,  // Guide lines/extensions
    ActiveLayer3D // 3D Geometry (Vertices/Edges)
};

/**
 * @brief Result of a snap operation
 */
struct SnapResult {
    bool snapped = false;
    SnapType type = SnapType::None;
    Vec2d position{0.0, 0.0};
    EntityID entityId;          // Entity snapped to (if any)
    EntityID secondEntityId;    // Second entity (for intersections)
    EntityID pointId;           // Existing point entity (if snap maps to a point)
    double distance = 0.0;      // Distance from cursor to snap point

    /**
     * @brief Comparison for priority sorting
     * Lower type value = higher priority; on tie, closer distance wins
     */
    bool operator<(const SnapResult& other) const {
        if (type != other.type) {
            return static_cast<int>(type) < static_cast<int>(other.type);
        }
        return distance < other.distance;
    }
};

/**
 * @brief Snap manager for precision drawing
 *
 * Provides snap-to-geometry functionality during sketch drawing.
 * Implements priority-based snap selection with configurable snap types.
 */
class SnapManager {
public:
    SnapManager();
    ~SnapManager() = default;

    // Non-copyable, movable
    SnapManager(const SnapManager&) = delete;
    SnapManager& operator=(const SnapManager&) = delete;
    SnapManager(SnapManager&&) = default;
    SnapManager& operator=(SnapManager&&) = default;

    /**
     * @brief Find best snap point near cursor
     * @param cursorPos Cursor position in sketch coordinates
     * @param sketch Sketch to snap to
     * @param excludeEntities Entities to exclude from snapping (e.g., entity being drawn)
     * @return Best snap result, or result with snapped=false if no snap found
     */
    SnapResult findBestSnap(const Vec2d& cursorPos,
                            const Sketch& sketch,
                            const std::unordered_set<EntityID>& excludeEntities = {}) const;

    /**
     * @brief Set external 3D geometry for snapping
     * @param points 3D points projected to sketch plane
     * @param edges 3D edges projected to sketch plane lines/curves
     */
    void setExternalGeometry(const std::vector<Vec2d>& points,
                             const std::vector<std::pair<Vec2d, Vec2d>>& lines);

    /**
     * @brief Find all snap points near cursor (for UI feedback)
     */
    std::vector<SnapResult> findAllSnaps(const Vec2d& cursorPos,
                                          const Sketch& sketch,
                                          const std::unordered_set<EntityID>& excludeEntities = {}) const;

    // ========== Configuration ==========

    /**
     * @brief Set snap radius in mm (default: 2.0mm per spec)
     */
    void setSnapRadius(double radiusMM) { snapRadius_ = radiusMM; }
    double getSnapRadius() const { return snapRadius_; }

    /**
     * @brief Enable/disable specific snap type
     */
    void setSnapEnabled(SnapType type, bool enabled);
    bool isSnapEnabled(SnapType type) const;

    /**
     * @brief Enable/disable all snap types at once
     */
    void setAllSnapsEnabled(bool enabled);

    /**
     * @brief Enable/disable grid snapping
     */
    void setGridSnapEnabled(bool enabled) { gridSnapEnabled_ = enabled; }
    bool isGridSnapEnabled() const { return gridSnapEnabled_; }

    /**
     * @brief Set grid size for grid snapping
     */
    void setGridSize(double sizeMM) { gridSize_ = sizeMM; }
    double getGridSize() const { return gridSize_; }

    /**
     * @brief Master enable/disable for all snapping
     */
    void setEnabled(bool enabled) { enabled_ = enabled; }
    bool isEnabled() const { return enabled_; }

    void setShowGuidePoints(bool show) { showGuidePoints_ = show; }
    bool showGuidePoints() const { return showGuidePoints_; }

    void setShowSnappingHints(bool show) { showSnappingHints_ = show; }
    bool showSnappingHints() const { return showSnappingHints_; }

    /**
     * @brief Find intersections between two entities (public for IntersectionManager)
     */
    std::vector<Vec2d> findEntityIntersections(const SketchEntity* e1,
                                                const SketchEntity* e2,
                                                const Sketch& sketch) const;

private:
    double snapRadius_ = constants::SNAP_RADIUS_MM;  // 2.0mm default
    double gridSize_ = 1.0;  // 1mm grid default
    bool gridSnapEnabled_ = true;
    bool enabled_ = true;
    std::unordered_map<SnapType, bool> snapTypeEnabled_;

    bool showGuidePoints_ = true;
    bool showSnappingHints_ = true;

    // External geometry (projected 3D entities)
    std::vector<Vec2d> extPoints_;
    std::vector<std::pair<Vec2d, Vec2d>> extLines_;

    // ========== Individual Snap Type Finders ==========

    /**
     * @brief Find snaps to existing sketch points
     */
    void findVertexSnaps(const Vec2d& cursorPos,
                         const Sketch& sketch,
                         const std::unordered_set<EntityID>& excludeEntities,
                         double radiusSq,
                         std::vector<SnapResult>& results) const;

    /**
     * @brief Find snaps to line endpoints
     */
    void findEndpointSnaps(const Vec2d& cursorPos,
                           const Sketch& sketch,
                           const std::unordered_set<EntityID>& excludeEntities,
                           double radiusSq,
                           std::vector<SnapResult>& results) const;

    /**
     * @brief Find snaps to line midpoints
     */
    void findMidpointSnaps(const Vec2d& cursorPos,
                           const Sketch& sketch,
                           const std::unordered_set<EntityID>& excludeEntities,
                           double radiusSq,
                           std::vector<SnapResult>& results) const;

    /**
     * @brief Find snaps to arc/circle centers
     */
    void findCenterSnaps(const Vec2d& cursorPos,
                         const Sketch& sketch,
                         const std::unordered_set<EntityID>& excludeEntities,
                         double radiusSq,
                         std::vector<SnapResult>& results) const;

    /**
     * @brief Find snaps to circle quadrant points (0, 90, 180, 270 degrees)
     */
    void findQuadrantSnaps(const Vec2d& cursorPos,
                           const Sketch& sketch,
                           const std::unordered_set<EntityID>& excludeEntities,
                           double radiusSq,
                           std::vector<SnapResult>& results) const;

    /**
     * @brief Find snaps to entity intersections
     */
    void findIntersectionSnaps(const Vec2d& cursorPos,
                               const Sketch& sketch,
                               const std::unordered_set<EntityID>& excludeEntities,
                               double radiusSq,
                               std::vector<SnapResult>& results) const;

    /**
     * @brief Find snaps to nearest point on curves
     */
    void findOnCurveSnaps(const Vec2d& cursorPos,
                          const Sketch& sketch,
                          const std::unordered_set<EntityID>& excludeEntities,
                          double radiusSq,
                          std::vector<SnapResult>& results) const;

    /**
     * @brief Find snap to grid
     */
    void findGridSnaps(const Vec2d& cursorPos,
                       double radiusSq,
                       std::vector<SnapResult>& results) const;

    /**
     * @brief Find snap to external geometry (3D)
     */
    void findExternalSnaps(const Vec2d& cursorPos,
                           double radiusSq,
                           std::vector<SnapResult>& results) const;

    // ========== Geometry Helpers ==========

    /**
     * @brief Calculate distance squared from point to point
     */
    static double distanceSquared(const Vec2d& a, const Vec2d& b);

    /**
     * @brief Find nearest point on line segment to given point
     */
    static Vec2d nearestPointOnLine(const Vec2d& point,
                                     const Vec2d& lineStart,
                                     const Vec2d& lineEnd);

    /**
     * @brief Find nearest point on circle to given point
     */
    static Vec2d nearestPointOnCircle(const Vec2d& point,
                                       const Vec2d& center,
                                       double radius);

    /**
     * @brief Find line-line intersection
     * @return Intersection point, or nullopt if lines are parallel
     */
    static std::optional<Vec2d> lineLineIntersection(const Vec2d& p1, const Vec2d& p2,
                                                      const Vec2d& p3, const Vec2d& p4);

    /**
     * @brief Find line-circle intersections
     * @return 0, 1, or 2 intersection points
     */
    static std::vector<Vec2d> lineCircleIntersection(const Vec2d& lineStart,
                                                      const Vec2d& lineEnd,
                                                      const Vec2d& center,
                                                      double radius);

    /**
     * @brief Find circle-circle intersections
     * @return 0, 1, or 2 intersection points
     */
    static std::vector<Vec2d> circleCircleIntersection(const Vec2d& center1,
                                                        double radius1,
                                                        const Vec2d& center2,
                                                        double radius2);
};

} // namespace onecad::core::sketch

#endif // ONECAD_CORE_SKETCH_SNAP_MANAGER_H
