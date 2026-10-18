//! Accessible body rows for the native toolkit spike.

use crate::ui::font;
use crate::{icons, scene::Body, theme::Theme};
use egui::{
    Align2, Color32, CornerRadius, EventFilter, FontFamily, Id, Key, Modifiers, Rect, Response,
    Sense, Stroke, StrokeKind, Ui, UiBuilder, vec2,
};

pub fn show(ui: &mut Ui, theme: &Theme, bodies: &mut [Body], selected: &mut Option<usize>) {
    let tree_id = ui.make_persistent_id("body-tree");
    ui.scope_builder(
        UiBuilder::new()
            .scope_id(tree_id)
            .accessibility_role(egui::Role::Tree)
            .accessibility_label("Bodies"),
        |ui| {
            ui.ctx().accesskit_node_builder(tree_id, |node| {
                node.set_size_of_set(bodies.len());
            });
            let ids: Vec<Id> = (0..bodies.len())
                .map(|i| ui.make_persistent_id(("body", i)))
                .collect();
            navigate(ui, &ids, selected);
            for (i, (body, id)) in bodies.iter_mut().zip(ids.iter().copied()).enumerate() {
                row(ui, theme, body, selected, i, id);
            }
            // WidgetInfo::selected maps to checked/toggled, not AccessKit's selected state.
            for (i, id) in ids.into_iter().enumerate() {
                ui.ctx().accesskit_node_builder(id, |node| {
                    // A later row click can clear an earlier row's selection in this pass.
                    node.set_selected(*selected == Some(i));
                });
            }
        },
    );
}

fn navigate(ui: &mut Ui, ids: &[Id], selected: &mut Option<usize>) {
    let Some(index) = ui.memory(|memory| ids.iter().position(|id| memory.has_focus(*id))) else {
        return;
    };
    let (target, handled) = ui.input_mut(|input| {
        for (key, target) in [
            (Key::ArrowUp, index.saturating_sub(1)),
            (Key::ArrowDown, (index + 1).min(ids.len() - 1)),
            (Key::Home, 0),
            (Key::End, ids.len() - 1),
        ] {
            if input.consume_key(Modifiers::NONE, key) {
                return (Some(target), true);
            }
        }
        // These rows are leaves: horizontal arrows cannot expand or collapse them.
        let handled = input.consume_key(Modifiers::NONE, Key::ArrowLeft)
            | input.consume_key(Modifiers::NONE, Key::ArrowRight);
        (None, handled)
    });
    if handled {
        ui.memory_mut(|memory| {
            // egui schedules spatial focus movement before widgets consume the key.
            memory.move_focus(egui::FocusDirection::None);
            if let Some(target) = target {
                *selected = Some(target);
                memory.request_focus(ids[target]);
            }
        });
    }
}

fn row(ui: &mut Ui, t: &Theme, body: &mut Body, selection: &mut Option<usize>, i: usize, id: Id) {
    let selected = *selection == Some(i);
    let row = ui.scope_builder(
        UiBuilder::new()
            .scope_id(id)
            .sense(Sense::click())
            .accessibility_role(egui::Role::TreeItem),
        |ui| {
            let (_, outer) = ui.allocate_space(vec2(ui.available_width(), 33.0));
            paint_row(
                ui,
                t,
                outer.shrink2(vec2(8.0, 0.5)),
                &ui.response(),
                &body.label,
                selected,
            );
            visibility(ui, t, outer, body)
        },
    );
    let response = row.response;
    if !row.inner && response.clicked() {
        *selection = if selected { None } else { Some(i) };
        response.request_focus();
    }
    if response.has_focus() {
        response.scroll_to_me(None);
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                id,
                EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..Default::default()
                },
            );
        });
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TreeItem, true, &body.label));
    ui.ctx().accesskit_node_builder(id, |node| {
        node.set_level(0);
        node.set_position_in_set(i);
    });
}

fn paint_row(ui: &Ui, t: &Theme, rect: Rect, response: &Response, name: &str, selected: bool) {
    let bg = if selected {
        t.sel_bg
    } else if response.hovered() {
        t.hover2
    } else {
        Color32::TRANSPARENT
    };
    let fg = if selected { t.sel_text } else { t.tree_label };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(6), bg);
    if response.has_focus() {
        painter.rect_stroke(
            rect,
            CornerRadius::same(6),
            Stroke::new(1.0, t.accent),
            StrokeKind::Inside,
        );
    }
    let icon = Rect::from_center_size(rect.left_center() + vec2(15.5, 0.0), vec2(15.0, 15.0));
    icons::paint(
        painter,
        icon,
        "body",
        fg,
        if selected { t.sel_text } else { t.icon_accent },
        1.6,
    );
    painter.text(
        rect.left_center() + vec2(31.0, 0.0),
        Align2::LEFT_CENTER,
        name,
        font(13.0, FontFamily::Proportional),
        fg,
    );
}

fn visibility(ui: &mut Ui, t: &Theme, outer: Rect, body: &mut Body) -> bool {
    let rect = outer.shrink2(vec2(8.0, 0.5));
    let eye = Rect::from_center_size(rect.right_center() - vec2(16.0, 0.0), vec2(15.0, 15.0));
    let response = ui.interact(
        eye.expand(4.0),
        ui.make_persistent_id("visibility"),
        Sense::click(),
    );
    if response.clicked() {
        body.visible = !body.visible;
        response.request_focus();
    }
    let color = if response.hovered() { t.ink3 } else { t.ink6 };
    icons::paint(
        ui.painter(),
        eye,
        if body.visible { "eyeOn" } else { "eyeOff" },
        color,
        color,
        1.5,
    );
    if response.has_focus() {
        ui.painter().rect_stroke(
            eye.expand(4.0),
            CornerRadius::same(3),
            Stroke::new(1.0, t.accent),
            StrokeKind::Inside,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::Role::CheckBox,
            true,
            body.visible,
            format!("Show {}", body.label),
        )
    });
    response.clicked()
}

#[cfg(test)]
#[path = "tree_tests.rs"]
mod tests;
