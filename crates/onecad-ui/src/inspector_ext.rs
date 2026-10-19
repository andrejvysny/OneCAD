//! Inspector section an extension contributes (Extensions 19a): Sheet Metal. Facade only.
use crate::{
    app::App,
    pill::selection_label,
    tool_registry::{SelectionKind, selection_kind},
};
use egui::{Align2, FontId, Ui, pos2, vec2};
use onecad_widgets::components::{
    actions::puzzle,
    button::{Button, ButtonKind},
    lists::SectionLabel,
};

/// Copy shown under the header; edges point at Flange, bodies at their edges.
fn intro(name: &str, kind: SelectionKind) -> String {
    let target = if kind == SelectionKind::Edge {
        "this edge"
    } else {
        "its edges"
    };
    format!(
        "{name} is a solid body. Convert it to set thickness and bend rules; then Flange works on {target}."
    )
}

/// Appends the Sheet Metal section for edge and body selections.
pub fn sheet_metal(app: &mut App, ui: &mut Ui) {
    let kind = selection_kind(app);
    if !matches!(kind, SelectionKind::Edge | SelectionKind::Body) {
        return;
    }
    let name = selection_label(app, kind).map_or_else(String::new, |l| l.1);
    let t = app.theme.clone();
    let header = ui.add(SectionLabel::new("SHEET METAL"));
    let right = pos2(header.rect.right() - 2.0, header.rect.center().y + 1.0);
    let galley_w = 58.0;
    ui.painter().text(
        right,
        Align2::RIGHT_CENTER,
        "Sheet Metal",
        FontId::proportional(11.0),
        t.ink5,
    );
    puzzle(
        ui.painter(),
        right - vec2(galley_w + 8.0, 0.0),
        12.0,
        t.accent,
        None,
    );
    ui.add_space(2.0);
    ui.add(
        egui::Label::new(
            egui::RichText::new(intro(&name, kind))
                .size(12.0)
                .color(t.ink3),
        )
        .wrap(),
    );
    ui.add_space(8.0);
    let converted = app.ui.editor_facade.converted;
    let button = if converted {
        Button::new("Converted (preview only)")
            .icon(Some("check"))
            .enabled(false)
    } else {
        Button::new("Convert to sheet metal…")
            .kind(ButtonKind::Secondary)
            .icon(Some("body"))
    };
    if ui.add(button).clicked() {
        app.ui.editor_facade.converted = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intro_names_body_and_target() {
        let edge = intro("Bracket", SelectionKind::Edge);
        assert!(edge.starts_with("Bracket is a solid body."));
        assert!(edge.ends_with("Flange works on this edge."));
        assert!(intro("Bracket", SelectionKind::Body).ends_with("its edges."));
    }
}
