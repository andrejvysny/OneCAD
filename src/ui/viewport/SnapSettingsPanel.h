#ifndef ONECAD_UI_VIEWPORT_SNAPSETTINGSPANEL_H
#define ONECAD_UI_VIEWPORT_SNAPSETTINGSPANEL_H

#include <QWidget>

class QCheckBox;
class QLabel;

namespace onecad::ui {

class SnapSettingsPanel : public QWidget {
    Q_OBJECT

public:
    struct SnapSettings {
        bool grid = true;
        bool sketchGuideLines = true;
        bool sketchGuidePoints = true;
        bool activeLayer3DPoints = true;
        bool activeLayer3DEdges = true;
        bool showGuidePoints = true;
        bool showSnappingHints = true;
    };

    explicit SnapSettingsPanel(QWidget* parent = nullptr);
    ~SnapSettingsPanel() override = default;

    void setSettings(const SnapSettings& settings);
    SnapSettings settings() const;

signals:
    void settingsChanged();

private:
    void setupUi();
    void updateTheme();
    QCheckBox* createToggle(const QString& text);

    QLabel* m_titleLabel = nullptr;
    
    // Snap to
    QCheckBox* m_snapGrid = nullptr;
    QCheckBox* m_snapSketchLines = nullptr;
    QCheckBox* m_snapSketchPoints = nullptr;
    QCheckBox* m_snap3DPoints = nullptr;
    QCheckBox* m_snap3DEdges = nullptr;
    
    // Show
    QCheckBox* m_showGuidePoints = nullptr;
    QCheckBox* m_showHints = nullptr;
};

} // namespace onecad::ui

#endif // ONECAD_UI_VIEWPORT_SNAPSETTINGSPANEL_H
