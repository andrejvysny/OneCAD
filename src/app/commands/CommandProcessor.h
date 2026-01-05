/**
 * @file CommandProcessor.h
 * @brief Command processor managing undo/redo stacks.
 */
#ifndef ONECAD_APP_COMMANDS_COMMANDPROCESSOR_H
#define ONECAD_APP_COMMANDS_COMMANDPROCESSOR_H

#include "Command.h"

#include <QObject>
#include <memory>
#include <string>
#include <vector>

namespace onecad::app::commands {

class CommandProcessor : public QObject {
    Q_OBJECT

public:
    explicit CommandProcessor(QObject* parent = nullptr);

    bool execute(std::unique_ptr<Command> command);
    void undo();
    void redo();

    bool canUndo() const;
    bool canRedo() const;

    void beginTransaction(const std::string& label = {});
    void endTransaction();
    void cancelTransaction();

signals:
    void canUndoChanged(bool canUndo);
    void canRedoChanged(bool canRedo);

private:
    void emitStateChange(bool prevUndo, bool prevRedo);

    bool inTransaction_ = false;
    std::string transactionLabel_;
    std::vector<std::unique_ptr<Command>> undoStack_;
    std::vector<std::unique_ptr<Command>> redoStack_;
    std::vector<std::unique_ptr<Command>> transaction_;
};

} // namespace onecad::app::commands

#endif // ONECAD_APP_COMMANDS_COMMANDPROCESSOR_H
