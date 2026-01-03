/**
 * @file SketchRenderer.h
 * @brief Rendering interface for sketch visualization
 *
 * Per SPECIFICATION.md §5.16, §5.18:
 * - VBO-based OpenGL rendering for performance
 * - Adaptive arc tessellation based on zoom
 * - Constraint icon atlas rendering
 * - DOF indicator visualization
 * - Viewport culling for large sketches
 *
 * IMPLEMENTATION STATUS: PLACEHOLDER
 * Actual rendering implementation in Phase 4.
 * Requires integration with existing OpenGL infrastructure.
 */
#ifndef ONECAD_CORE_SKETCH_SKETCH_RENDERER_H
#define ONECAD_CORE_SKETCH_SKETCH_RENDERER_H

#include "SketchTypes.h"
#include <memory>
#include <vector>
#include <functional>

// Forward declarations for OpenGL types
class QOpenGLShaderProgram;
class QOpenGLBuffer;
class QOpenGLVertexArrayObject;
class QMatrix4x4;

namespace onecad::core::sketch {

class Sketch;
class SketchEntity;
class SketchConstraint;

/**
 * @brief Snap type enumeration
 *
 * Per SPECIFICATION.md §5.14:
 * Different snap types with priority order
 */
enum class SnapType {
    None = 0,
    Vertex,       // Snap to existing point (highest priority)
    Endpoint,     // Snap to line/arc endpoint
    Midpoint,     // Snap to line midpoint
    Center,       // Snap to arc/circle center
    Quadrant,     // Snap to circle quadrant points
    Intersection, // Snap to intersection of two entities
    OnCurve,      // Snap to nearest point on curve
    Grid,         // Snap to grid (lowest priority)
    Perpendicular,// Perpendicular snap (inference)
    Tangent,      // Tangent snap (inference)
    Horizontal,   // Horizontal inference
    Vertical      // Vertical inference
};

/**
 * @brief Color definitions for sketch rendering
 */
struct SketchColors {
    // Entity colors
    Vec3d normalGeometry{1.0, 1.0, 1.0};       // White
    Vec3d constructionGeometry{0.5, 0.8, 0.5}; // Light green
    Vec3d selectedGeometry{0.2, 0.6, 1.0};     // Blue
    Vec3d previewGeometry{0.7, 0.7, 0.7};      // Gray
    Vec3d errorGeometry{1.0, 0.3, 0.3};        // Red

    // Constraint colors
    Vec3d constraintIcon{0.9, 0.7, 0.2};       // Orange
    Vec3d dimensionText{0.2, 0.8, 0.2};        // Green
    Vec3d conflictHighlight{1.0, 0.0, 0.0};    // Red

    // DOF indicator
    Vec3d fullyConstrained{0.0, 1.0, 0.0};     // Green
    Vec3d underConstrained{1.0, 0.5, 0.0};     // Orange
    Vec3d overConstrained{1.0, 0.0, 0.0};      // Red

    // Background
    Vec3d gridMajor{0.3, 0.3, 0.3};
    Vec3d gridMinor{0.15, 0.15, 0.15};
};

/**
 * @brief Rendering style options
 */
struct SketchRenderStyle {
    // Line widths (in pixels)
    float normalLineWidth = 1.5f;
    float constructionLineWidth = 1.0f;
    float selectedLineWidth = 2.5f;
    float previewLineWidth = 1.0f;

    // Point sizes (in pixels)
    float pointSize = 6.0f;
    float selectedPointSize = 10.0f;
    float snapPointSize = 8.0f;

    // Constraint icon size (in pixels)
    float constraintIconSize = 16.0f;

    // Dimension text
    float dimensionFontSize = 12.0f;

    // Colors
    SketchColors colors;

    // Arc tessellation
    int minArcSegments = 8;
    int maxArcSegments = 128;
    float arcTessellationAngle = 5.0f; // degrees per segment

    // Dash pattern for construction geometry
    float dashLength = 5.0f;
    float gapLength = 3.0f;
};

/**
 * @brief Viewport information for culling
 */
struct Viewport {
    Vec2d center;
    Vec2d size;
    double zoom = 1.0;

    Vec2d getMin() const {
        return {center.x - size.x / 2, center.y - size.y / 2};
    }

    Vec2d getMax() const {
        return {center.x + size.x / 2, center.y + size.y / 2};
    }

    bool contains(const Vec2d& point) const {
        Vec2d min = getMin();
        Vec2d max = getMax();
        return point.x >= min.x && point.x <= max.x &&
               point.y >= min.y && point.y <= max.y;
    }

    bool intersects(const Vec2d& boundsMin, const Vec2d& boundsMax) const {
        Vec2d vpMin = getMin();
        Vec2d vpMax = getMax();
        return !(boundsMax.x < vpMin.x || boundsMin.x > vpMax.x ||
                 boundsMax.y < vpMin.y || boundsMin.y > vpMax.y);
    }
};

/**
 * @brief Selection state for entities
 */
enum class SelectionState {
    None,
    Hover,       // Mouse hovering over
    Selected,    // Part of selection set
    Dragging     // Being dragged
};

/**
 * @brief Render data for a single entity
 */
struct EntityRenderData {
    EntityID id;
    EntityType type;
    SelectionState selection = SelectionState::None;
    bool isConstruction = false;
    bool hasError = false;

    // Cached geometry for rendering
    std::vector<Vec2d> vertices;    // For lines: 2 points; for arcs: tessellated points
    Vec2d bounds[2];                // Bounding box [min, max]
};

/**
 * @brief Render data for a constraint icon
 */
struct ConstraintRenderData {
    ConstraintID id;
    ConstraintType type;
    Vec2d position;         // Icon position in sketch space
    bool isConflicting = false;
    bool isRedundant = false;

    // For dimensional constraints
    std::optional<double> value;
    std::optional<std::string> expression;
};

/**
 * @brief Sketch renderer class
 *
 * Handles all visual representation of sketch geometry and constraints.
 *
 * IMPLEMENTATION STATUS: PLACEHOLDER
 * Key rendering systems to implement:
 * - VBO management for entity geometry
 * - Texture atlas for constraint icons
 * - Adaptive arc tessellation
 * - Viewport culling
 * - Selection highlighting
 */
// Forward declare PIMPL class
class SketchRendererImpl;

class SketchRenderer {
public:
    SketchRenderer();
    ~SketchRenderer();

    // Non-copyable
    SketchRenderer(const SketchRenderer&) = delete;
    SketchRenderer& operator=(const SketchRenderer&) = delete;

    /**
     * @brief Initialize OpenGL resources
     *
     * Must be called after OpenGL context is available.
     * Creates shaders, VBOs, textures.
     *
     * PLACEHOLDER: OpenGL initialization
     */
    bool initialize();

    /**
     * @brief Release OpenGL resources
     */
    void cleanup();

    /**
     * @brief Set the sketch to render
     */
    void setSketch(Sketch* sketch);

    /**
     * @brief Update render data from sketch
     *
     * Call when sketch geometry changes.
     * Rebuilds VBO data.
     */
    void updateGeometry();

    /**
     * @brief Update only constraint visualization
     *
     * Lighter update when only constraint state changes.
     */
    void updateConstraints();

    /**
     * @brief Render the sketch
     * @param viewMatrix View transformation matrix
     * @param projMatrix Projection matrix
     *
     * PLACEHOLDER: OpenGL rendering calls
     *
     * Per SPECIFICATION.md §5.18:
     * 1. Frustum cull entities outside viewport
     * 2. Render construction geometry (dashed)
     * 3. Render normal geometry (solid)
     * 4. Render selected geometry (highlighted)
     * 5. Render preview geometry (if any)
     * 6. Render constraint icons
     * 7. Render dimension text
     * 8. Render DOF indicator
     */
    void render(const QMatrix4x4& viewMatrix, const QMatrix4x4& projMatrix);

    // ========== Selection & Highlighting ==========

    /**
     * @brief Set entity selection state
     */
    void setEntitySelection(EntityID id, SelectionState state);

    /**
     * @brief Clear all selections
     */
    void clearSelection();

    /**
     * @brief Set hover entity
     */
    void setHoverEntity(EntityID id);

    /**
     * @brief Set entities to highlight as conflicting
     */
    void setConflictingConstraints(const std::vector<ConstraintID>& ids);

    // ========== Preview Geometry ==========

    /**
     * @brief Set preview line (for drawing tool)
     */
    void setPreviewLine(const Vec2d& start, const Vec2d& end);

    /**
     * @brief Set preview arc
     */
    void setPreviewArc(const Vec2d& center, double radius,
                       double startAngle, double endAngle);

    /**
     * @brief Set preview circle (full arc 0 to 2π)
     */
    void setPreviewCircle(const Vec2d& center, double radius);

    /**
     * @brief Set preview rectangle (4 lines)
     */
    void setPreviewRectangle(const Vec2d& corner1, const Vec2d& corner2);

    /**
     * @brief Clear preview geometry
     */
    void clearPreview();

    // ========== Snap Indicators ==========

    /**
     * @brief Show snap indicator at point
     *
     * Per SPECIFICATION.md §5.14:
     * Ghost icon showing snap type (vertex, grid, etc.)
     */
    void showSnapIndicator(const Vec2d& pos, SnapType type);

    /**
     * @brief Hide snap indicator
     */
    void hideSnapIndicator();

    // ========== Style & Configuration ==========

    /**
     * @brief Set rendering style
     */
    void setStyle(const SketchRenderStyle& style);
    const SketchRenderStyle& getStyle() const { return style_; }

    /**
     * @brief Set viewport for culling
     */
    void setViewport(const Viewport& viewport);

    /**
     * @brief Set pixel-to-sketch scale (for consistent line widths)
     */
    void setPixelScale(double scale);

    // ========== DOF Indicator ==========

    /**
     * @brief Update DOF display
     */
    void setDOF(int dof);

    /**
     * @brief Show/hide DOF indicator
     */
    void setShowDOF(bool show);

    // ========== Hit Testing ==========

    /**
     * @brief Find entity at screen position
     * @param screenPos Position in screen coordinates
     * @param tolerance Pick tolerance in pixels
     * @return EntityID of nearest entity, or 0 if none
     *
     * PLACEHOLDER: Hit testing algorithm
     */
    EntityID pickEntity(const Vec2d& screenPos, double tolerance = 5.0) const;

    /**
     * @brief Find constraint at screen position
     */
    ConstraintID pickConstraint(const Vec2d& screenPos, double tolerance = 5.0) const;

private:
    // PIMPL for OpenGL internals
    std::unique_ptr<SketchRendererImpl> impl_;

    Sketch* sketch_ = nullptr;
    SketchRenderStyle style_;
    Viewport viewport_;
    double pixelScale_ = 1.0;

    // Selection state
    std::unordered_map<EntityID, SelectionState> entitySelections_;
    EntityID hoverEntity_;  // Empty string by default
    std::vector<ConstraintID> conflictingConstraints_;

    // Preview geometry
    struct {
        bool active = false;
        EntityType type = EntityType::Line;
        std::vector<Vec2d> vertices;
    } preview_;

    // Snap indicator
    struct {
        bool active = false;
        Vec2d position{0.0, 0.0};
        SnapType type = SnapType::None;
    } snapIndicator_;

    // DOF indicator
    int currentDOF_ = 0;
    bool showDOF_ = true;

    // Cached render data
    std::vector<EntityRenderData> entityRenderData_;
    std::vector<ConstraintRenderData> constraintRenderData_;
    bool geometryDirty_ = true;
    bool constraintsDirty_ = true;
    bool vboDirty_ = true;

    // ========== OpenGL Resources (PLACEHOLDERS) ==========
    // These will be initialized in Phase 4

    // Shaders
    // std::unique_ptr<QOpenGLShaderProgram> lineShader_;
    // std::unique_ptr<QOpenGLShaderProgram> pointShader_;
    // std::unique_ptr<QOpenGLShaderProgram> iconShader_;

    // VBOs
    // std::unique_ptr<QOpenGLBuffer> lineVBO_;
    // std::unique_ptr<QOpenGLBuffer> pointVBO_;
    // std::unique_ptr<QOpenGLVertexArrayObject> lineVAO_;
    // std::unique_ptr<QOpenGLVertexArrayObject> pointVAO_;

    // Texture atlas for constraint icons
    // unsigned int constraintIconAtlas_ = 0;

    /**
     * @brief Tessellate arc into line segments
     *
     * Per SPECIFICATION.md §5.18:
     * Adaptive tessellation based on zoom level
     *
     * PLACEHOLDER: Arc tessellation algorithm
     * segments = clamp(arc_angle / tessellation_angle, min, max)
     */
    std::vector<Vec2d> tessellateArc(const Vec2d& center, double radius,
                                      double startAngle, double endAngle) const;

    /**
     * @brief Build VBO data from entity render data
     *
     * PLACEHOLDER: VBO building
     */
    void buildVBOs();

    /**
     * @brief Check if entity is visible in current viewport
     */
    bool isEntityVisible(const EntityRenderData& data) const;

    /**
     * @brief Calculate constraint icon position
     *
     * Per SPECIFICATION.md §5.16:
     * Position icons near constrained geometry
     */
    Vec2d calculateConstraintIconPosition(const SketchConstraint* constraint) const;
};

/**
 * @brief Snap result from snap system
 */
struct SnapResult {
    bool snapped = false;
    SnapType type = SnapType::None;
    Vec2d position;
    EntityID entityId;          // Entity snapped to (if any)
    EntityID secondEntity;      // For intersections
    double distance = 0.0;      // Distance from cursor

    bool operator<(const SnapResult& other) const {
        // Priority: type first, then distance
        if (type != other.type) {
            return static_cast<int>(type) < static_cast<int>(other.type);
        }
        return distance < other.distance;
    }
};

/**
 * @brief Snap manager for auto-constraining
 *
 * Per SPECIFICATION.md §5.14:
 * 2mm snap radius, priority order
 *
 * PLACEHOLDER: Snap implementation in Phase 5
 */
class SnapManager {
public:
    /**
     * @brief Find best snap position
     * @param cursorPos Current cursor position
     * @param sketch Sketch to snap to
     * @param excludeEntity Entity to exclude (e.g., entity being drawn)
     */
    SnapResult findSnap(const Vec2d& cursorPos, const Sketch& sketch,
                        EntityID excludeEntity = {}) const;

    /**
     * @brief Set snap radius in mm
     */
    void setSnapRadius(double radius) { snapRadius_ = radius; }

    /**
     * @brief Enable/disable specific snap types
     */
    void setSnapEnabled(SnapType type, bool enabled);

    /**
     * @brief Enable/disable grid snapping
     */
    void setGridSnap(bool enabled, double gridSize = 1.0);

private:
    double snapRadius_ = 2.0;  // mm
    double gridSize_ = 1.0;    // mm
    bool gridSnapEnabled_ = true;
    std::unordered_map<SnapType, bool> snapEnabled_;

    /**
     * @brief Find all potential snap points
     */
    std::vector<SnapResult> findAllSnaps(const Vec2d& cursorPos,
                                          const Sketch& sketch,
                                          EntityID excludeEntity) const;
};

} // namespace onecad::core::sketch

#endif // ONECAD_CORE_SKETCH_SKETCH_RENDERER_H
