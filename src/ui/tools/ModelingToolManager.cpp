/**
 * @file ModelingToolManager.cpp
 */
#include "ModelingToolManager.h"
#include "ExtrudeTool.h"

#include "../viewport/Viewport.h"

namespace onecad::ui::tools {

ModelingToolManager::ModelingToolManager(Viewport* viewport, QObject* parent)
    : QObject(parent), viewport_(viewport) {
    extrudeTool_ = std::make_unique<ExtrudeTool>(viewport_, document_);
}

ModelingToolManager::~ModelingToolManager() = default;

void ModelingToolManager::setDocument(app::Document* document) {
    document_ = document;
    if (extrudeTool_) {
        extrudeTool_->setDocument(document_);
    }
}

void ModelingToolManager::setCommandProcessor(app::commands::CommandProcessor* processor) {
    commandProcessor_ = processor;
    if (extrudeTool_) {
        extrudeTool_->setCommandProcessor(commandProcessor_);
    }
}

bool ModelingToolManager::hasActiveTool() const {
    return activeTool_ != nullptr && activeTool_->isActive();
}

bool ModelingToolManager::isDragging() const {
    return activeTool_ != nullptr && activeTool_->isDragging();
}

void ModelingToolManager::activateExtrude(const app::selection::SelectionItem& selection) {
    if (!extrudeTool_) {
        return;
    }

    app::selection::SelectionKey key{selection.kind, selection.id};
    if (activeTool_ == extrudeTool_.get() && activeSelection_ == key && activeTool_->isActive()) {
        return;
    }

    activeSelection_ = key;
    activeTool_ = extrudeTool_.get();
    activeTool_->begin(selection);
}

void ModelingToolManager::cancelActiveTool() {
    if (activeTool_) {
        activeTool_->cancel();
    }
    activeTool_ = nullptr;
    activeSelection_ = {};
}

bool ModelingToolManager::handleMousePress(const QPoint& screenPos, Qt::MouseButton button) {
    if (!activeTool_) {
        return false;
    }
    return activeTool_->handleMousePress(screenPos, button);
}

bool ModelingToolManager::handleMouseMove(const QPoint& screenPos) {
    if (!activeTool_) {
        return false;
    }
    return activeTool_->handleMouseMove(screenPos);
}

bool ModelingToolManager::handleMouseRelease(const QPoint& screenPos, Qt::MouseButton button) {
    if (!activeTool_) {
        return false;
    }
    return activeTool_->handleMouseRelease(screenPos, button);
}

} // namespace onecad::ui::tools
