/**
 * @file AddOperationCommand.cpp
 * @brief Implementation of AddOperationCommand.
 */
#include "AddOperationCommand.h"
#include "OperationCommandUtils.h"
#include "../document/Document.h"

#include <algorithm>

namespace onecad::app::commands {

AddOperationCommand::AddOperationCommand(Document* document, OperationRecord record)
    : document_(document)
    , record_(std::move(record)) {
}

bool AddOperationCommand::execute() {
    if (!document_) {
        return false;
    }
    if (document_->findOperation(record_.opId)) {
        return false;
    }

    const std::size_t insertIndex = std::min(document_->appliedOpCount(),
                                             document_->operations().size());
    if (!document_->insertOperation(insertIndex, record_)) {
        return false;
    }
    document_->setAppliedOpCount(insertIndex + 1);

    if (!regenerateDocument(document_)) {
        document_->removeOperation(record_.opId);
        regenerateDocument(document_);
        return false;
    }

    return true;
}

bool AddOperationCommand::undo() {
    if (!document_) {
        return false;
    }

    if (!document_->removeOperation(record_.opId)) {
        return false;
    }

    if (!regenerateDocument(document_)) {
        document_->insertOperation(document_->operations().size(), record_);
        regenerateDocument(document_);
        return false;
    }

    return true;
}

} // namespace onecad::app::commands
