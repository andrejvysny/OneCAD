//! Every native sketch relation uses the same typed operand applicability as its dispatch.
use crate::{app::App, constraint_authoring::Relation, layout::Chrome, ui_state::UiState};
use egui::{Context, Order, Pos2, Rect, pos2};
use onecad_widgets::{
    components::lists::{MenuRow, MenuSection, menu_divider, menu_frame},
    frame,
};
const WIDTH: f32 = 270.0;
const BELOW_STRIP: f32 = 108.0;

pub fn toggle(state: &mut UiState) {
    state.constraints_open = !state.constraints_open;
}
fn outside(menu: Rect, anchor: Option<Rect>, click: Pos2) -> bool {
    !menu.contains(click) && !anchor.is_some_and(|anchor| anchor.contains(click))
}

fn rows(app: &App, ui: &mut egui::Ui, relations: &[Relation]) -> Option<Relation> {
    let mut clicked = None;
    for relation in relations {
        let applicable = crate::constraint_authoring::applicability(app, *relation);
        let response = ui
            .add(MenuRow::new(Some(relation.icon()), relation.label()).enabled(applicable.is_ok()));
        if response.clicked() && applicable.is_ok() {
            clicked = Some(*relation);
        }
        if let Err(reason) = applicable {
            response.on_hover_text(reason);
        }
    }
    clicked
}

pub fn show(app: &mut App, ctx: &Context, chrome: &Chrome, anchor: Option<Rect>) {
    if !app.ui.constraints_open {
        return;
    }
    let Some(button) = anchor.filter(|_| app.sketch_active()) else {
        app.ui.constraints_open = false;
        return;
    };
    let vp = chrome.viewport;
    let x = button
        .left()
        .min(vp.right() - WIDTH - 10.0)
        .max(vp.left() + 10.0);
    let pos = pos2(x, vp.top() + BELOW_STRIP);
    let mut clicked = None;
    let area =
        frame::floating(egui::Id::unique("constraints-menu"), pos, Order::Middle).show(ctx, |ui| {
            menu_frame(ctx).show(ui, |ui| {
                ui.set_width(WIDTH - 14.0);
                ui.spacing_mut().item_spacing.y = 0.0;
                egui::ScrollArea::vertical()
                    .id_salt("constraint-relations")
                    .max_height((vp.bottom() - pos.y - 20.0).max(80.0))
                    .show(ui, |ui| {
                        ui.add(MenuSection::new("GEOMETRIC"));
                        clicked = rows(app, ui, &Relation::GEOMETRIC);
                        menu_divider(ui);
                        ui.add(MenuSection::new("DIMENSIONAL"));
                        clicked = rows(app, ui, &Relation::DIMENSIONAL).or(clicked);
                    });
            });
        });
    if let Some(relation) = clicked {
        crate::constraint_authoring::activate_relation(app, ctx, relation);
    }
    let click = ctx.input(|input| {
        input
            .pointer
            .any_pressed()
            .then(|| input.pointer.interact_pos())
            .flatten()
    });
    if click.is_some_and(|point| outside(area.response.rect, Some(button), point)) {
        app.ui.constraints_open = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;
    #[test]
    fn toggle_flips() {
        let mut state = UiState::default();
        toggle(&mut state);
        assert!(state.constraints_open);
        toggle(&mut state);
        assert!(!state.constraints_open);
    }
    #[test]
    fn click_outside_ignores_menu_and_button() {
        let menu = Rect::from_min_size(pos2(100.0, 100.0), vec2(270.0, 400.0));
        let button = Rect::from_min_size(pos2(100.0, 50.0), vec2(34.0, 34.0));
        assert!(!outside(menu, Some(button), pos2(120.0, 200.0)));
        assert!(!outside(menu, Some(button), pos2(110.0, 60.0)));
        assert!(outside(menu, Some(button), pos2(600.0, 600.0)));
    }
    #[test]
    fn all_relation_icons_exist() {
        for relation in Relation::GEOMETRIC.into_iter().chain(Relation::DIMENSIONAL) {
            assert!(
                onecad_widgets::icon::exists(relation.icon()),
                "{}",
                relation.label()
            );
        }
    }
}
