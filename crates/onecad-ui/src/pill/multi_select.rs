//! The visible multi-select switch: touch has no Shift or Command key.
use onecad_widgets::components::{
    actions::{ToolButton, ToolState},
    tooltip::Tip,
};

/// Draws the switch in its `on` state; true when it was tapped.
pub(super) fn button(ui: &mut egui::Ui, on: bool) -> bool {
    let (state, tip) = if on {
        (
            ToolState::Active,
            "Multi-select on: each pick adds or removes an item",
        )
    } else {
        (
            ToolState::Default,
            "Multi-select: each pick adds or removes an item",
        )
    };
    let response = ui.add(ToolButton::new("plus").state(state).tip(Tip::new(tip)));
    crate::automation::name(ui.ctx(), response.id, crate::automation::ADDITIVE_SELECT);
    let toggled = if on {
        egui::accesskit::Toggled::True
    } else {
        egui::accesskit::Toggled::False
    };
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_toggled(toggled));
    response.clicked()
}
