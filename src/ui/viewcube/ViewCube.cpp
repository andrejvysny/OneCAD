#include "ViewCube.h"
#include "../../render/Camera3D.h"
#include <QPainter>
#include <QPainterPath>
#include <QPolygonF>
#include <QMouseEvent>
#include <QtMath>
#include <algorithm>
#include <limits>

namespace onecad {
namespace ui {

ViewCube::ViewCube(QWidget* parent)
    : QWidget(parent) {
    setFixedSize(static_cast<int>(m_cubeSize), static_cast<int>(m_cubeSize));
    setAttribute(Qt::WA_TranslucentBackground);
    setMouseTracking(true);
    m_cubeRotation.setToIdentity();
    m_cubeRotation.rotate(90.0f, 0.0f, 0.0f, 1.0f);
    initGeometry();
}

void ViewCube::setCamera(render::Camera3D* camera) {
    m_camera = camera;
    update();
}

void ViewCube::updateRotation() {
    update();
}

void ViewCube::initGeometry() {
    // 1. Vertices (8)
    // x, y, z in {-1, 1}
    for (int z = -1; z <= 1; z += 2) {
        for (int y = -1; y <= 1; y += 2) {
            for (int x = -1; x <= 1; x += 2) {
                CubeVertex v;
                v.pos = QVector3D(x, y, z);
                v.id = m_vertices.size();
                m_vertices.append(v);
            }
        }
    }
    // Indices:
    // 0: -1 -1 -1
    // 1:  1 -1 -1
    // 2: -1  1 -1
    // 3:  1  1 -1
    // 4: -1 -1  1
    // 5:  1 -1  1
    // 6: -1  1  1
    // 7:  1  1  1

    // 2. Faces (6)
    auto addFace = [&](int id, const QString& name, const QVector3D& normal, int v0, int v1, int v2, int v3) {
        CubeFace f;
        f.id = id;
        f.text = name;
        f.normal = normal;
        f.vIndices[0] = v0;
        f.vIndices[1] = v1;
        f.vIndices[2] = v2;
        f.vIndices[3] = v3;
        m_faces.append(f);
    };

    // Front (Y=-1): 0, 1, 5, 4 (CCW looking at face? No, vertices order matters for rendering loop)
    // Vertices with Y=-1: 0, 1, 4, 5.
    // Order to form a quad: (-1,-1,-1), (1,-1,-1), (1,-1,1), (-1,-1,1) -> 0, 1, 5, 4.
    // Normal (0,-1,0).
    addFace(0, "FRONT",  QVector3D(0, -1, 0), 4, 5, 1, 0); // Reordered for CCW from outside: TL(4), TR(5), BR(1), BL(0)

    // Back (Y=1): 2, 3, 7, 6.
    // Normal (0,1,0). CCW: 6, 7, 3, 2 (TopLeft, TopRight...)
    addFace(1, "BACK",   QVector3D(0, 1, 0), 6, 2, 3, 7); // Checked visually.

    // Right (X=1): 1, 3, 7, 5.
    // Normal (1,0,0). CCW: 5, 7, 3, 1
    addFace(2, "RIGHT",  QVector3D(1, 0, 0), 5, 7, 3, 1);

    // Left (X=-1): 0, 2, 6, 4.
    // Normal (-1,0,0). CCW: 4, 6, 2, 0
    addFace(3, "LEFT",   QVector3D(-1, 0, 0), 4, 0, 2, 6);

    // Top (Z=1): 4, 5, 7, 6.
    // Normal (0,0,1). CCW: 4, 6, 7, 5?
    // -1 -1 1 (4), -1 1 1 (6), 1 1 1 (7), 1 -1 1 (5).
    addFace(4, "TOP",    QVector3D(0, 0, 1), 6, 7, 5, 4);

    // Bottom (Z=-1): 0, 1, 3, 2.
    // Normal (0,0,-1). CCW: 0, 1, 3, 2.
    addFace(5, "BOTTOM", QVector3D(0, 0, -1), 0, 2, 3, 1);

    // 3. Edges (12)
    // Unique edges between vertices.
    auto addEdge = [&](int v1, int v2) {
        CubeEdge e;
        e.v1 = v1;
        e.v2 = v2;
        e.id = m_edges.size();
        m_edges.append(e);
    };
    
    // Bottom Ring (Z=-1)
    addEdge(0, 1); addEdge(1, 3); addEdge(3, 2); addEdge(2, 0);
    // Top Ring (Z=1)
    addEdge(4, 5); addEdge(5, 7); addEdge(7, 6); addEdge(6, 4);
    // Pillars
    addEdge(0, 4); addEdge(1, 5); addEdge(2, 6); addEdge(3, 7);
}

QPointF ViewCube::project(const QVector3D& point, const QMatrix4x4& viewRot, float scale) {
    QVector3D transformed = viewRot * (m_cubeRotation * point);
    float x = transformed.x() * scale + width() / 2.0f;
    float y = -transformed.y() * scale + height() / 2.0f; 
    return QPointF(x, y);
}

ViewCube::Hit ViewCube::hitTest(const QPoint& pos) {
    if (!m_camera) return {};

    QMatrix4x4 view = m_camera->viewMatrix();
    view.setColumn(3, QVector4D(0, 0, 0, 1));
    QVector3D forward = m_camera->forward().normalized();
    float scale = (std::min(width(), height()) / 2.0f) * m_scale;
    auto rotatePoint = [this](const QVector3D& point) {
        return m_cubeRotation * point;
    };
    auto rotateNormal = [this](const QVector3D& normal) {
        return m_cubeRotation * normal;
    };
    auto isPointVisible = [&forward, &rotatePoint](const QVector3D& point) {
        return QVector3D::dotProduct(rotatePoint(point), forward) < 0.0f;
    };
    auto isFaceVisible = [&forward, &rotateNormal](const QVector3D& normal) {
        return QVector3D::dotProduct(rotateNormal(normal), forward) < -0.001f;
    };

    // 1. Check Corners (Highest Priority)
    for (const auto& v : m_vertices) {
        if (!isPointVisible(v.pos)) {
            continue;
        }
        QPointF p = project(v.pos, view, scale);
        float dist = QVector2D(QPointF(pos) - p).length();
        if (dist < 12.0f) { // 12px radius
            return {ElementType::Corner, v.id};
        }
    }

    // 2. Check Edges
    // Only check visible edges? Or all? All is fine if we check Z, but 2D proj implies front.
    // Actually, we should only pick visible elements.
    // But for edges, the "closest" one is best.
    // Distance to segment.
    float bestEdgeDist = 10.0f; // Threshold
    int bestEdgeIdx = -1;
    float bestEdgeDepth = -std::numeric_limits<float>::infinity();

    for (const auto& e : m_edges) {
        QVector3D mid = (m_vertices[e.v1].pos + m_vertices[e.v2].pos) * 0.5f;
        if (!isPointVisible(mid)) {
            continue;
        }
        QPointF p1 = project(m_vertices[e.v1].pos, view, scale);
        QPointF p2 = project(m_vertices[e.v2].pos, view, scale);
        
        // Distance from point to segment
        QVector2D p(pos);
        QVector2D v(p2 - p1);
        float lenSq = v.lengthSquared();
        if (lenSq < 1e-4f) continue; // Degenerate edge
        
        QVector2D w(p - QVector2D(p1));
        
        float t = QVector2D::dotProduct(w, v) / lenSq;
        t = qBound(0.0f, t, 1.0f);
        QVector2D closest = QVector2D(p1) + v * t;
        float dist = (p - closest).length();
        
        if (dist < bestEdgeDist) {
            // Check Z depth to ensure we pick the front one if they overlap?
            // Midpoint Z
            float depth = -QVector3D::dotProduct(rotatePoint(mid), forward);
            // Larger depth is closer
            if (depth > bestEdgeDepth) {
                bestEdgeDist = dist;
                bestEdgeIdx = e.id;
                bestEdgeDepth = depth;
            }
        }
    }
    
    if (bestEdgeIdx != -1) {
        return {ElementType::Edge, bestEdgeIdx};
    }

    // 3. Check Faces (only visible ones)
    struct RenderFace {
        int index;
        float zDepth;
    };
    QVector<RenderFace> visibleFaces;

    for (int i = 0; i < m_faces.size(); ++i) {
        if (isFaceVisible(m_faces[i].normal)) {
            QVector3D center = (m_vertices[m_faces[i].vIndices[0]].pos + m_vertices[m_faces[i].vIndices[2]].pos) * 0.5f;
            float depth = -QVector3D::dotProduct(rotatePoint(center), forward);
            visibleFaces.append({i, depth});
        }
    }

    // Sort closest first
    std::sort(visibleFaces.begin(), visibleFaces.end(), [](const RenderFace& a, const RenderFace& b) {
        return a.zDepth > b.zDepth;
    });

    for (const auto& rf : visibleFaces) {
        QPolygonF poly;
        for (int k = 0; k < 4; ++k) {
            poly << project(m_vertices[m_faces[rf.index].vIndices[k]].pos, view, scale);
        }
        if (poly.containsPoint(pos, Qt::OddEvenFill)) {
            return {ElementType::Face, rf.index};
        }
    }

    return {};
}

void ViewCube::paintEvent(QPaintEvent* event) {
    Q_UNUSED(event);
    if (!m_camera) return;

    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing);

    QMatrix4x4 view = m_camera->viewMatrix();
    view.setColumn(3, QVector4D(0, 0, 0, 1));
    QVector3D forward = m_camera->forward().normalized();
    float scale = (std::min(width(), height()) / 2.0f) * m_scale;
    auto rotatePoint = [this](const QVector3D& point) {
        return m_cubeRotation * point;
    };
    auto rotateNormal = [this](const QVector3D& normal) {
        return m_cubeRotation * normal;
    };
    auto isFaceVisible = [&forward, &rotateNormal](const QVector3D& normal) {
        return QVector3D::dotProduct(rotateNormal(normal), forward) < -0.001f;
    };

    // Sort Faces
    struct RenderFace {
        int index;
        float zDepth;
    };
    QVector<RenderFace> visibleFaces;

    for (int i = 0; i < m_faces.size(); ++i) {
        if (isFaceVisible(m_faces[i].normal)) {
            QVector3D center = (m_vertices[m_faces[i].vIndices[0]].pos + m_vertices[m_faces[i].vIndices[2]].pos) * 0.5f;
            float depth = -QVector3D::dotProduct(rotatePoint(center), forward);
            visibleFaces.append({i, depth});
        }
    }
    // Furthest first (Painter's algo)
    std::sort(visibleFaces.begin(), visibleFaces.end(), [](const RenderFace& a, const RenderFace& b) {
        return a.zDepth < b.zDepth;
    });

    // Draw Faces
    for (const auto& rf : visibleFaces) {
        const CubeFace& face = m_faces[rf.index];
        QPolygonF poly;
        for (int k = 0; k < 4; ++k) {
            poly << project(m_vertices[face.vIndices[k]].pos, view, scale);
        }

        bool isHovered = (m_hoveredHit.type == ElementType::Face && m_hoveredHit.index == face.id);
        
        QColor fillColor = isHovered ? QColor(64, 128, 255, 191) : QColor(220, 220, 220, 191);
        QColor textColor = isHovered ? Qt::white : Qt::black;

        QPainterPath path;
        path.addPolygon(poly);
        painter.fillPath(path, fillColor);
        
        // Face border (part of edge, but drawn with face for clean look)
        painter.setPen(QPen(QColor(150, 150, 150, 191), 1));
        painter.drawPath(path);

        // Text
        QPointF center = project((m_vertices[face.vIndices[0]].pos + m_vertices[face.vIndices[2]].pos) * 0.5f, view, scale);
        painter.setPen(textColor);
        QFont font = painter.font();
        font.setBold(true);
        font.setPointSize(9);
        painter.setFont(font);
        QRectF textRect(center.x() - 30, center.y() - 15, 60, 30);
        painter.drawText(textRect, Qt::AlignCenter, face.text);
    }

    // Axes from front-left-bottom corner (origin for view cube)
    {
        const QVector3D origin(-1.0f, -1.0f, -1.0f);
        const QVector3D xAxisEnd(1.0f, -1.0f, -1.0f);
        const QVector3D yAxisEnd(-1.0f, 1.0f, -1.0f);
        const QVector3D zAxisEnd(-1.0f, -1.0f, 1.0f);

        QPointF o = project(origin, view, scale);
        QPointF x = project(xAxisEnd, view, scale);
        QPointF y = project(yAxisEnd, view, scale);
        QPointF z = project(zAxisEnd, view, scale);

        painter.setBrush(Qt::NoBrush);
        painter.setPen(QPen(QColor(220, 80, 80, 191), 2, Qt::SolidLine, Qt::RoundCap));
        painter.drawLine(o, x);
        painter.setPen(QPen(QColor(80, 200, 120, 191), 2, Qt::SolidLine, Qt::RoundCap));
        painter.drawLine(o, y);
        painter.setPen(QPen(QColor(80, 120, 220, 191), 2, Qt::SolidLine, Qt::RoundCap));
        painter.drawLine(o, z);

        QFont axisFont = painter.font();
        axisFont.setPointSize(8);
        axisFont.setBold(true);
        painter.setFont(axisFont);
        painter.setPen(QColor(220, 80, 80, 191));
        painter.drawText(QRectF(x.x() - 8, x.y() - 8, 16, 16), Qt::AlignCenter, "X");
        painter.setPen(QColor(80, 200, 120, 191));
        painter.drawText(QRectF(y.x() - 8, y.y() - 8, 16, 16), Qt::AlignCenter, "Y");
        painter.setPen(QColor(80, 120, 220, 191));
        painter.drawText(QRectF(z.x() - 8, z.y() - 8, 16, 16), Qt::AlignCenter, "Z");
    }

    // Highlight Edge if hovered
    if (m_hoveredHit.type == ElementType::Edge) {
        const CubeEdge& e = m_edges[m_hoveredHit.index];
        QPointF p1 = project(m_vertices[e.v1].pos, view, scale);
        QPointF p2 = project(m_vertices[e.v2].pos, view, scale);
        
        painter.setPen(QPen(QColor(64, 128, 255, 191), 4, Qt::SolidLine, Qt::RoundCap));
        painter.drawLine(p1, p2);
    }
    
    // Highlight Corner if hovered
    if (m_hoveredHit.type == ElementType::Corner) {
        const CubeVertex& v = m_vertices[m_hoveredHit.index];
        QPointF p = project(v.pos, view, scale);
        
        painter.setBrush(QColor(64, 128, 255, 191));
        painter.setPen(Qt::NoPen);
        painter.drawEllipse(p, 6, 6);
    }
}

void ViewCube::snapToView(const Hit& hit) {
    if (!m_camera) return;
    
    if (hit.type == ElementType::Face) {
        switch(hit.index) {
            case 0: m_camera->setFrontView(); break;
            case 1: m_camera->setBackView(); break;
            case 2: m_camera->setRightView(); break;
            case 3: m_camera->setLeftView(); break;
            case 4: m_camera->setTopView(); break;
            case 5: m_camera->setBottomView(); break;
        }
    } else if (hit.type == ElementType::Corner) {
        // Isometric from corner
        QVector3D dir = m_vertices[hit.index].pos.normalized();
        float dist = m_camera->distance();
        m_camera->setTarget(QVector3D(0,0,0));
        m_camera->setPosition(dir * dist);
        m_camera->setUp(QVector3D(0, 0, 1));
    } else if (hit.type == ElementType::Edge) {
        const CubeEdge& e = m_edges[hit.index];
        QVector3D mid = (m_vertices[e.v1].pos + m_vertices[e.v2].pos) * 0.5f;
        QVector3D dir = mid.normalized();
        float dist = m_camera->distance();
        m_camera->setTarget(QVector3D(0,0,0));
        m_camera->setPosition(dir * dist);
        m_camera->setUp(QVector3D(0, 0, 1));
    }
    
    emit viewChanged();
    update();
}

void ViewCube::mousePressEvent(QMouseEvent* event) {
    if (event->button() == Qt::LeftButton) {
        m_isDragging = false;
        m_lastMousePos = event->pos();
    }
}

void ViewCube::mouseMoveEvent(QMouseEvent* event) {
    if (event->buttons() & Qt::LeftButton) {
        if (!m_isDragging && (event->pos() - m_lastMousePos).manhattanLength() > 2) {
            m_isDragging = true;
        }
        
        if (m_isDragging) {
            QPoint delta = event->pos() - m_lastMousePos;
            if (m_camera) {
                m_camera->orbit(delta.x() * 0.5f, delta.y() * 0.5f);
                emit viewChanged();
                update();
            }
            m_lastMousePos = event->pos();
            return;
        }
    }

    Hit hit = hitTest(event->pos());
    if (hit != m_hoveredHit) {
        m_hoveredHit = hit;
        update();
    }
}

void ViewCube::mouseReleaseEvent(QMouseEvent* event) {
    if (event->button() == Qt::LeftButton && !m_isDragging) {
        Hit hit = hitTest(event->pos());
        if (hit.type != ElementType::None) {
            snapToView(hit);
        }
    }
    m_isDragging = false;
}

void ViewCube::enterEvent(QEnterEvent* event) {
    Q_UNUSED(event);
    update();
}

void ViewCube::leaveEvent(QEvent* event) {
    Q_UNUSED(event);
    m_hoveredHit = {};
    update();
}

} // namespace ui
} // namespace onecad
