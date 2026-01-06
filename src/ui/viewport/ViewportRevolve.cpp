#include "Viewport.h"
#include "../tools/ModelingToolManager.h"
#include "../../app/selection/SelectionManager.h"
#include "../../app/selection/SelectionTypes.h"

namespace onecad::ui {

bool Viewport::activateRevolveTool() {
    if (m_inSketchMode || !m_selectionManager || !m_modelingToolManager) {
        setRevolveToolActive(false);
        return false;
    }

    if (m_revolveToolActive) {
        setExtrudeToolActive(false);
        return true;
    }

    const auto& selection = m_selectionManager->selection();
    if (selection.size() == 1 &&
        (selection.front().kind == app::selection::SelectionKind::SketchRegion ||
         selection.front().kind == app::selection::SelectionKind::Face)) {
        m_modelingToolManager->activateRevolve(selection.front());
        setExtrudeToolActive(false);
        setRevolveToolActive(true);
        return true;
    }

    setRevolveToolActive(false);
    return false;
}

void Viewport::setRevolveToolActive(bool active) {
    if (m_revolveToolActive == active) {
        return;
    }
    m_revolveToolActive = active;
    updateModelSelectionFilter();
    emit revolveToolActiveChanged(active);
}

} // namespace onecad::ui
