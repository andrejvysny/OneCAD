/**
 * @file ModelingToolManager.cpp
 */
#include "ModelingToolManager.h"
#include "ExtrudeTool.h"
#include "RevolveTool.h"

#include "../viewport/Viewport.h"

namespace onecad::ui::tools {

ModelingToolManager::ModelingToolManager(Viewport* viewport, QObject* parent)
    : QObject(parent), viewport_(viewport) {
    extrudeTool_ = std::make_unique<ExtrudeTool>(viewport_, document_);
    revolveTool_ = std::make_unique<RevolveTool>(viewport_, document_);
}

ModelingToolManager::~ModelingToolManager() = default;

void ModelingToolManager::setDocument(app::Document* document) {
    document_ = document;
    if (extrudeTool_) {
        extrudeTool_->setDocument(document_);
    }
    if (revolveTool_) {
        revolveTool_->setDocument(document_);
    }
}

void ModelingToolManager::setCommandProcessor(app::commands::CommandProcessor* processor) {
    commandProcessor_ = processor;
    if (extrudeTool_) {
        extrudeTool_->setCommandProcessor(commandProcessor_);
    }
    if (revolveTool_) {
        revolveTool_->setCommandProcessor(commandProcessor_);
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
    
    // If we switch tools, cancel previous one
    if (activeTool_ && activeTool_ != extrudeTool_.get()) {
        activeTool_->cancel();
    }

    activeSelection_ = key;
    activeTool_ = extrudeTool_.get();
    activeTool_->begin(selection);
}

void ModelingToolManager::activateRevolve(const app::selection::SelectionItem& selection) {
    if (!revolveTool_) return;

    app::selection::SelectionKey key{selection.kind, selection.id};
    if (activeTool_ == revolveTool_.get() && activeSelection_ == key && activeTool_->isActive()) {
        return;
    }

    if (activeTool_ && activeTool_ != revolveTool_.get()) {
        activeTool_->cancel();
    }

    activeSelection_ = key;
    activeTool_ = revolveTool_.get();
    activeTool_->begin(selection);
}

void ModelingToolManager::cancelActiveTool() {
    if (activeTool_) {
        activeTool_->cancel();
    }
    activeTool_ = nullptr;
    activeSelection_ = {};
}

void ModelingToolManager::onSelectionChanged(const std::vector<app::selection::SelectionItem>& selection) {
    if (activeTool_ == revolveTool_.get()) {
        revolveTool_->onSelectionChanged(selection);
    }
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

std::optional<ModelingTool::Indicator> ModelingToolManager::activeIndicator() const {
    if (!activeTool_ || !activeTool_->isActive()) {
        return std::nullopt;
    }
    return activeTool_->indicator();
}

} // namespace onecad::ui::tools
