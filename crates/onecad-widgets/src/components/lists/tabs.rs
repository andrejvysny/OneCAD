//! TabBar: underline tabs with optional muted counts (component detail, extension detail).

use egui::{FontFamily, IdSalt, Rect, Response, Sense, Ui, Widget, pos2, vec2};

use crate::components::actions::common::{focus_ring, galley};
use crate::fonts;
use crate::metrics::BODY;
use crate::theme::look;

const HEIGHT: f32 = 36.0;
const GAP: f32 = 20.0;
const COUNT_GAP: f32 = 5.0;

#[derive(Clone, Copy, Debug)]
pub struct Tab<'a> {
    pub label: &'a str,
    pub count: Option<usize>,
}

pub struct TabBar<'a> {
    id_salt: IdSalt,
    selected: &'a mut usize,
    tabs: &'a [Tab<'a>],
}

impl<'a> TabBar<'a> {
    pub fn new(
        id_salt: impl std::hash::Hash + std::fmt::Debug,
        selected: &'a mut usize,
        tabs: &'a [Tab<'a>],
    ) -> Self {
        Self {
            id_salt: IdSalt::new(id_salt),
            selected,
            tabs,
        }
    }
}

impl Widget for TabBar<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let l = look(ui.ctx());
        let t = &l.theme;
        let base = ui.make_persistent_id(self.id_salt);
        let (rect, mut response) =
            ui.allocate_exact_size(vec2(ui.available_width(), HEIGHT), Sense::hover());
        let p = ui.painter();
        p.hline(
            rect.x_range(),
            rect.bottom() - 0.5,
            egui::Stroke::new(1.0, t.border),
        );
        let mut x = rect.left();
        for (i, tab) in self.tabs.iter().enumerate() {
            let on = *self.selected == i;
            let family = if on {
                fonts::semibold()
            } else {
                FontFamily::Proportional
            };
            let label = galley(p, tab.label, BODY, family);
            let count = tab
                .count
                .map(|c| galley(p, &c.to_string(), BODY, FontFamily::Proportional));
            let width = label.size().x + count.as_ref().map_or(0.0, |c| COUNT_GAP + c.size().x);
            let tab_rect = Rect::from_min_size(pos2(x, rect.top()), vec2(width, HEIGHT));
            x += width + GAP;
            let r = ui.interact(tab_rect, base.with(i), Sense::click());
            if r.clicked() && !on {
                *self.selected = i;
                response.mark_changed();
            }
            let ink = match (on, r.hovered()) {
                (true, _) => t.ink,
                (false, true) => t.ink2,
                (false, false) => t.ink4,
            };
            let cy = rect.center().y - 1.0;
            p.galley(
                pos2(tab_rect.left(), cy - label.size().y / 2.0),
                label.clone(),
                ink,
            );
            if let Some(c) = count {
                let pos = pos2(
                    tab_rect.left() + label.size().x + COUNT_GAP,
                    cy - c.size().y / 2.0,
                );
                p.galley(pos, c, t.ink5);
            }
            if on {
                let bar = Rect::from_min_max(
                    pos2(tab_rect.left(), rect.bottom() - 2.0),
                    pos2(tab_rect.right(), rect.bottom()),
                );
                p.rect_filled(bar, 0.0, t.accent);
            }
            if r.has_focus() {
                focus_ring(p, t, tab_rect, 4);
            }
            r.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, on, tab.label));
        }
        response
    }
}
