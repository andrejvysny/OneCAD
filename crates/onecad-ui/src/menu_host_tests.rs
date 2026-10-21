//! The in-app menu queue of hosts without a menu bar (`--no-default-features`).
use super::{Action, host_menu};
use crate::document_flow::ExportFormat;

#[test]
fn disabled_rows_never_queue_and_enabled_ones_drain_once() {
    let menu = host_menu().expect("hosts without a menu bar get the in-app queue");
    menu.set_history_enabled(true, false);
    menu.set_document_enabled(false, true, true, false);
    let actions = [
        Action::Undo,
        Action::Redo,
        Action::Open,
        Action::InsertStep,
        Action::Save,
        Action::Export(ExportFormat::Step),
    ];
    for action in actions {
        menu.push(action);
    }
    assert_eq!(
        menu.drain(),
        [Action::Undo, Action::InsertStep, Action::Save]
    );
    assert!(menu.drain().is_empty());
}
