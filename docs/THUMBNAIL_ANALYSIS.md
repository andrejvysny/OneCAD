# Thumbnail Implementation Analysis

## Overview
This document outlines the plan to implement project thumbnails in OneCAD. The goal is to capture a screenshot of the 3D viewport when saving a project, store it within the `.onecad` file, and display it in the startup dialog to help users identify their projects.

## Architecture

### 1. File Format
The `.onecad` file is a ZIP archive (handled by `ZipPackage`).
*   **Current Content:** `manifest.json`, geometry files, etc.
*   **New Content:** `thumbnail.png` will be added to the root of the archive.
*   **Format:** PNG (lossless, good compression for UI graphics).
*   **Resolution:** 512x512 pixels (scaled from viewport), sufficient for high-DPI thumbnails without excessive file size.

### 2. I/O Layer (`src/io/`)
The `OneCADFileIO` class acts as the facade for file operations. It needs to be extended to support:
*   **Writing:** Accepting a `QImage` during the save process and writing it to the package.
*   **Reading:** A lightweight method `getThumbnail(path)` that opens the package, extracts *only* the `thumbnail.png`, and returns it as a `QImage`. This avoids parsing the full JSON manifest or geometry data during the startup listing.

### 3. Capture (`src/ui/mainwindow/`, `src/ui/viewport/`)
The `Viewport` class (inheriting `QOpenGLWidget`) provides `grabFramebuffer()`, which returns the current rendered frame as a `QImage`.
*   **Trigger:** The screenshot should be captured in `MainWindow::onSaveDocument` (or `saveDocumentToPath`) immediately before calling the IO layer.
*   **Processing:** The image should be scaled to 512x512 (keeping aspect ratio or cropping to square) to ensure consistent file size performance.

### 4. User Interface (`src/ui/start/`)
The `StartOverlay` currently uses simple text-based `QPushButton` elements. This needs to be upgraded to a gallery-style view.
*   **Layout:** Switch from a vertical list of buttons to a `QGridLayout` or a flow layout.
*   **Components:** Create a custom `ProjectTile` widget containing:
    *   `QLabel` for the thumbnail.
    *   `QLabel` for the project name.
    *   `QLabel` for the file path/date.

## Implementation Plan

### Phase 1: I/O Infrastructure

**File:** `src/io/OneCADFileIO.h` / `.cpp`

1.  **Modify `save` signature:**
    ```cpp
    // Old
    static FileIOResult save(const QString& filepath, const app::Document* document);
    
    // New
    static FileIOResult save(const QString& filepath, 
                             const app::Document* document, 
                             const QImage& thumbnail = QImage());
    ```

2.  **Implement Thumbnail Writing:**
    *   In `save()` implementation, if `!thumbnail.isNull()`:
        *   Convert `QImage` to `QByteArray` (PNG format).
        *   Call `package->writeFile("thumbnail.png", data)`.

3.  **Implement Thumbnail Reading:**
    *   Add `static QImage readThumbnail(const QString& filepath);`
    *   Implementation:
        *   `Package::openForRead(filepath)`
        *   `package->readFile("thumbnail.png")`
        *   Return `QImage::fromData()`.
    *   *Performance Note:* `ZipPackage` should support random access effectively so this doesn't read the whole file.

### Phase 2: Capture Logic

**File:** `src/ui/mainwindow/MainWindow.cpp`

1.  **Update `saveDocumentToPath`:**
    ```cpp
    // Capture
    QImage thumbnail = m_viewport->grabFramebuffer();
    
    // Resize (Maintain aspect ratio, fit within 512x512)
    thumbnail = thumbnail.scaled(512, 512, Qt::KeepAspectRatio, Qt::SmoothTransformation);
    
    // Save
    auto result = io::OneCADFileIO::save(filePath, m_document.get(), thumbnail);
    ```

### Phase 3: Start Screen UI

**File:** `src/ui/start/StartOverlay.cpp`

1.  **Update `rebuildRecentGrid`:**
    *   Instead of `QPushButton`, create a custom widget layout.
    *   Loop through projects:
        *   Call `io::OneCADFileIO::readThumbnail(path)`.
        *   If image is null, use a default "placeholder" icon (e.g., `:/icons/ic_body.svg`).
        *   Create a tile widget displaying the image and text.
    *   *Optimization:* For a large number of recent files, consider loading thumbnails asynchronously, but for < 10 items, synchronous loading is acceptable.

## Dependencies
*   **Qt GUI:** `QImage`, `QPixmap`, `QPainter`.
*   **OneCAD IO:** `Package` interface.

## Future Considerations
*   **Cache:** If thumbnail reading proves slow on network drives, implement a local thumbnail cache in `~/.onecad/cache/`.
*   **3D Preview:** In the future, we could store a lightweight mesh (GLB) instead of a PNG for an interactive 3D spinner in the start menu, but PNG is the standard MVP approach.



---

# Thumbnail Implementation Plan for OneCAD

## Summary

Add project thumbnails: capture viewport screenshot on save, store in .onecad package, display in StartOverlay gallery.

## Specifications (Resolved)

| Spec | Decision |
|------|----------|
| Grid columns | 4 fixed |
| Thumbnail resolution | 512x512px |
| Expected project count | <10 (sync loading OK) |
| Date display | Show modification date |
| Sketch mode behavior | Capture sketch view as-is |
| Placeholder icon | Create new `ic_project_placeholder.svg` |
| Tile style | Subtle drop shadow |
| Aspect ratio | Fit with letterboxing (no crop) |

---

## Phase 1: I/O Layer

### 1.1 OneCADFileIO.h

```cpp
// Add include
#include <QImage>

// Modify save signature
static FileIOResult save(const QString& filepath,
                         const app::Document* document,
                         const QImage& thumbnail = QImage());

// Add new method
static QImage readThumbnail(const QString& filepath);
```

### 1.2 OneCADFileIO.cpp

```cpp
// Add includes
#include <QImage>
#include <QBuffer>

// In save(), after writing document, before finalize():
if (!thumbnail.isNull()) {
    QByteArray pngData;
    QBuffer buffer(&pngData);
    buffer.open(QIODevice::WriteOnly);
    thumbnail.save(&buffer, "PNG");
    buffer.close();

    if (!package->writeFile("thumbnail.png", pngData)) {
        qWarning() << "Thumbnail write failed:" << package->errorString();
    }
}

// New readThumbnail()
QImage OneCADFileIO::readThumbnail(const QString& filepath) {
    auto package = Package::openForRead(filepath);
    if (!package) return {};

    QByteArray data = package->readFile("thumbnail.png");
    if (data.isEmpty()) return {};  // Graceful if missing

    return QImage::fromData(data, "PNG");
}
```

---

## Phase 2: Viewport Capture

### 2.1 Viewport.h

```cpp
public:
    QImage captureThumbnail(int maxSize = 512) const;
```

### 2.2 Viewport.cpp

```cpp
QImage Viewport::captureThumbnail(int maxSize) const {
    // Qt6 grabFramebuffer() handles MSAA internally
    QImage frame = const_cast<Viewport*>(this)->grabFramebuffer();
    if (frame.isNull()) return {};

    if (frame.width() > maxSize || frame.height() > maxSize) {
        frame = frame.scaled(maxSize, maxSize,
                             Qt::KeepAspectRatio,
                             Qt::SmoothTransformation);
    }
    return frame;
}
```

### 2.3 MainWindow.cpp - saveDocumentToPath()

```cpp
bool MainWindow::saveDocumentToPath(const QString& filePath) {
    if (!m_document) return false;

    if (m_toolStatus) m_toolStatus->setText(tr("Saving..."));

    // Capture thumbnail
    QImage thumbnail;
    if (m_viewport) {
        m_viewport->update();
        QCoreApplication::processEvents();  // Ensure paintGL runs
        thumbnail = m_viewport->captureThumbnail(512);
    }

    auto result = io::OneCADFileIO::save(filePath, m_document.get(), thumbnail);
    // ... rest unchanged
}
```

---

## Phase 3: ProjectTile Widget

### 3.1 Create src/ui/start/ProjectTile.h

```cpp
#pragma once
#include <QWidget>

class QLabel;
class QGraphicsDropShadowEffect;

class ProjectTile : public QWidget {
    Q_OBJECT
public:
    explicit ProjectTile(const QString& path,
                         const QImage& thumbnail = QImage(),
                         QWidget* parent = nullptr);
    QString filePath() const { return m_path; }

signals:
    void clicked(const QString& path);

protected:
    void mousePressEvent(QMouseEvent* event) override;
    void enterEvent(QEnterEvent* event) override;
    void leaveEvent(QEvent* event) override;
    void paintEvent(QPaintEvent* event) override;

private:
    void applyTheme();
    QString m_path;
    QLabel* m_thumbnailLabel = nullptr;
    QLabel* m_nameLabel = nullptr;
    QLabel* m_pathLabel = nullptr;
    QLabel* m_dateLabel = nullptr;
    bool m_hovered = false;
    QGraphicsDropShadowEffect* m_shadowEffect = nullptr;
};
```

### 3.2 Create src/ui/start/ProjectTile.cpp

Key specs:
- Tile size: 160x200px (taller for date row)
- Thumbnail display: 120x120px (scaled from 512, letterboxed)
- Placeholder: `:/icons/ic_project_placeholder.svg` (new icon)
- Theme-aware hover background
- Truncate path >30 chars with "..."
- Show modification date: "Modified: Jan 10, 2026"
- Subtle drop shadow on thumbnail (QGraphicsDropShadowEffect or QPainter)

```cpp
// Date formatting
QFileInfo info(path);
QString dateStr = info.lastModified().toString("MMM d, yyyy");
m_dateLabel = new QLabel(QString("Modified: %1").arg(dateStr), this);
m_dateLabel->setObjectName("projectDate");

// Shadow effect on thumbnail
m_shadowEffect = new QGraphicsDropShadowEffect(this);
m_shadowEffect->setBlurRadius(8);
m_shadowEffect->setOffset(0, 2);
m_shadowEffect->setColor(QColor(0, 0, 0, 40));
m_thumbnailLabel->setGraphicsEffect(m_shadowEffect);
```

---

## Phase 4: StartOverlay Integration

### 4.1 StartOverlay.h

```cpp
// Change member
QGridLayout* recentLayout_ = nullptr;  // was QVBoxLayout*
```

### 4.2 StartOverlay.cpp

```cpp
#include "ProjectTile.h"
#include "../../io/OneCADFileIO.h"

// Constructor: change to QGridLayout
recentLayout_ = new QGridLayout(recentContainer_);
recentLayout_->setSpacing(12);

// rebuildRecentGrid():
void StartOverlay::rebuildRecentGrid() {
    // Clear existing widgets
    while (QLayoutItem* item = recentLayout_->takeAt(0)) {
        if (item->widget()) item->widget()->deleteLater();
        delete item;
    }

    if (projects_.isEmpty()) {
        recentEmptyLabel_ = new QLabel(tr("No projects yet."));
        recentEmptyLabel_->setObjectName("emptyState");
        recentLayout_->addWidget(recentEmptyLabel_, 0, 0);
        return;
    }

    constexpr int kColumns = 4;
    int row = 0, col = 0;

    for (const QString& path : projects_) {
        QImage thumb = io::OneCADFileIO::readThumbnail(path);
        auto* tile = new ProjectTile(path, thumb, recentContainer_);
        connect(tile, &ProjectTile::clicked,
                this, &StartOverlay::handleRecentClicked);
        recentLayout_->addWidget(tile, row, col);

        if (++col >= kColumns) { col = 0; row++; }
    }
    recentLayout_->setRowStretch(row + 1, 1);
}
```

---

## Phase 5: Placeholder Icon

### 5.1 Create resources/icons/ic_project_placeholder.svg

Simple 3D cube/box silhouette:
```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">
  <path d="M32 8 L56 20 L56 44 L32 56 L8 44 L8 20 Z"
        fill="none" stroke="#9CA3AF" stroke-width="2"/>
  <path d="M32 8 L32 32 M8 20 L32 32 L56 20"
        fill="none" stroke="#9CA3AF" stroke-width="2"/>
  <path d="M32 32 L32 56"
        fill="none" stroke="#9CA3AF" stroke-width="2"/>
</svg>
```

Add to `resources/resources.qrc`:
```xml
<file>icons/ic_project_placeholder.svg</file>
```

---

## Files Summary

| File | Action |
|------|--------|
| `src/io/OneCADFileIO.h` | Modify |
| `src/io/OneCADFileIO.cpp` | Modify |
| `src/ui/viewport/Viewport.h` | Modify |
| `src/ui/viewport/Viewport.cpp` | Modify |
| `src/ui/mainwindow/MainWindow.cpp` | Modify |
| `src/ui/start/ProjectTile.h` | **Create** |
| `src/ui/start/ProjectTile.cpp` | **Create** |
| `src/ui/start/StartOverlay.h` | Modify |
| `src/ui/start/StartOverlay.cpp` | Modify |
| `src/CMakeLists.txt` | Add ProjectTile sources |
| `resources/icons/ic_project_placeholder.svg` | **Create** |
| `resources/resources.qrc` | Add icon entry |

---

## Verification

1. **Build & Run:** `make run`
2. **Create project** with geometry (e.g., extrude a rectangle)
3. **Save project** - confirm "Saving..." shows in status
4. **Check .onecad file:** `unzip -l project.onecad` → should list `thumbnail.png`
5. **Return to home** (Home button) - confirm thumbnail displays in tile
6. **Test missing thumbnail:** Open old project without thumbnail → placeholder icon shown
7. **Test edge cases:** Empty document, very large viewport

---

## Final Spec Additions

| Spec | Decision |
|------|----------|
| Shadow | QGraphicsDropShadowEffect |
| Letterbox bg | Same as panel bg (seamless) |
| Icon creation | Generate SVG code in implementation |


---

⏺ Implementation complete. Build succeeded.

  Summary

  Implemented project thumbnails for OneCAD:

  Files Modified
  ┌───────────────────────────────────────┬─────────────────────────────────────────────────────┐
  │                 File                  │                       Changes                       │
  ├───────────────────────────────────────┼─────────────────────────────────────────────────────┤
  │ src/io/OneCADFileIO.h                 │ Added QImage param to save(), added readThumbnail() │
  ├───────────────────────────────────────┼─────────────────────────────────────────────────────┤
  │ src/io/OneCADFileIO.cpp               │ Thumbnail write (PNG to ZIP) + read logic           │
  ├───────────────────────────────────────┼─────────────────────────────────────────────────────┤
  │ src/ui/viewport/Viewport.h            │ Added captureThumbnail(int maxSize)                 │
  ├───────────────────────────────────────┼─────────────────────────────────────────────────────┤
  │ src/ui/viewport/Viewport.cpp:1573     │ Implemented via grabFramebuffer() + scale           │
  ├───────────────────────────────────────┼─────────────────────────────────────────────────────┤
  │ src/ui/mainwindow/MainWindow.cpp:1235 │ Capture before save with processEvents()            │
  ├───────────────────────────────────────┼─────────────────────────────────────────────────────┤
  │ src/ui/start/StartOverlay.h           │ Changed to QGridLayout*                             │
  ├───────────────────────────────────────┼─────────────────────────────────────────────────────┤
  │ src/ui/start/StartOverlay.cpp         │ Uses ProjectTile, 4-column grid                     │
  ├───────────────────────────────────────┼─────────────────────────────────────────────────────┤
  │ src/ui/CMakeLists.txt                 │ Added ProjectTile.cpp                               │
  ├───────────────────────────────────────┼─────────────────────────────────────────────────────┤
  │ resources/resources.qrc               │ Added placeholder icon                              │
  └───────────────────────────────────────┴─────────────────────────────────────────────────────┘
  Files Created
  ┌────────────────────────────────────────────┬───────────────────────────────────────────────────────┐
  │                    File                    │                      Description                      │
  ├────────────────────────────────────────────┼───────────────────────────────────────────────────────┤
  │ src/ui/start/ProjectTile.h                 │ Widget class declaration                              │
  ├────────────────────────────────────────────┼───────────────────────────────────────────────────────┤
  │ src/ui/start/ProjectTile.cpp               │ 160x200 tile with thumbnail, name, path, date, shadow │
  ├────────────────────────────────────────────┼───────────────────────────────────────────────────────┤
  │ resources/icons/ic_project_placeholder.svg │ 3D cube silhouette placeholder                        │
  └────────────────────────────────────────────┴───────────────────────────────────────────────────────┘
  How It Works

  1. Save: saveDocumentToPath() captures viewport → 512x512 PNG → writes to thumbnail.png in package
  2. Load: rebuildRecentGrid() calls readThumbnail() → displays in ProjectTile grid
  3. Fallback: Missing thumbnails show placeholder icon

  To Verify

  make run
  # 1. Create project with geometry
  # 2. Save it
  # 3. Check: unzip -l ~/OneCAD/Projects/project.onecad | grep thumbnail
  # 4. Click Home → thumbnail should appear in tile

✻ Baked for 17m 52s
