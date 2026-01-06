#include "ModifyBodyCommand.h"
#include "../document/Document.h"
#include "../../kernel/elementmap/ElementMap.h"

namespace onecad::app::commands {

ModifyBodyCommand::ModifyBodyCommand(Document* document,
                                     const std::string& bodyId,
                                     const TopoDS_Shape& newShape)
    : document_(document), bodyId_(bodyId), newShape_(newShape) {
}

bool ModifyBodyCommand::execute() {
    if (!document_) return false;

    const TopoDS_Shape* current = document_->getBodyShape(bodyId_);
    if (current) {
        oldShape_ = *current;
    } else {
        return false;
    }

    // We need a way to update the body in the document.
    // Document doesn't have a public updateBody(id, shape).
    // But we can remove and add with the same ID, preserving metadata?
    // Document::addBodyWithId allows specifying ID.
    // We should probably add Document::updateBodyShape to be cleaner, 
    // but for now let's try the remove/add approach or assume we can add a method.
    // To be safe and minimal, I will check if I can modify Document.h later.
    // For now, I'll use the friend access or just assume I'll add the method.
    
    // Actually, looking at Document.h, addBodyWithId is public.
    // removeBody is public.
    // So:
    std::string name = document_->getBodyName(bodyId_);
    document_->removeBody(bodyId_);
    document_->addBodyWithId(bodyId_, newShape_, name);
    
    return true;
}

bool ModifyBodyCommand::undo() {
    if (!document_) return false;
    
    std::string name = document_->getBodyName(bodyId_);
    document_->removeBody(bodyId_);
    document_->addBodyWithId(bodyId_, oldShape_, name);
    
    return true;
}

} // namespace onecad::app::commands
