//! HistoryRow + RollbackMarker (c-hist): feature timeline rows in the Inspector.

use egui::text::{LayoutJob, TextFormat};
use egui::{
    Color32, CornerRadius, CursorIcon, FontFamily, FontId, Rect, Response, Sense, Ui, UiBuilder,
    Widget, pos2, vec2,
};

use crate::components::actions::common::{fitted, focus_ring, galley, inset};
use crate::components::inputs::NumberField;
use crate::fonts;
use crate::icon::{self, IconStyle};
use crate::metrics::{BODY, RADIUS_SM, SECTION};
use crate::theme::look;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HistoryState {
    #[default]
    Default,
    Selected,
    Editing,
    Failed,
    Suppressed,
    RolledBack,
}

pub struct HistoryRow<'a> {
    index: usize,
    icon: &'a str,
    label: &'a str,
    meta: Option<&'a str>,
    state: HistoryState,
}

pub struct HistoryOutput {
    pub response: Response,
    pub double_clicked: bool,
    /// Editing field lost focus without Escape.
    pub committed: bool,
}

impl<'a> HistoryRow<'a> {
    pub fn new(index: usize, icon: &'a str, label: &'a str) -> Self {
        Self {
            index,
            icon,
            label,
            meta: None,
            state: HistoryState::Default,
        }
    }
    pub fn meta(mut self, meta: Option<&'a str>) -> Self {
        self.meta = meta;
        self
    }
    pub fn state(mut self, state: HistoryState) -> Self {
        self.state = state;
        self
    }

    /// `edit` is the field text while `state` is `Editing`.
    pub fn show(self, ui: &mut Ui, edit: Option<&mut String>) -> HistoryOutput {
        let l = look(ui.ctx());
        let t = &l.theme;
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
        let st = self.state;
        let op = if st == HistoryState::RolledBack {
            0.45
        } else {
            1.0
        };
        let hovered = response.hovered() && st == HistoryState::Default;
        let fill = match st {
            HistoryState::Selected | HistoryState::Editing => t.sel_soft,
            HistoryState::Failed => t.danger_surface,
            _ if hovered => t.hover2,
            _ => Color32::TRANSPARENT,
        };
        let p = ui.painter().clone();
        let p = &p;
        p.rect_filled(rect, CornerRadius::same(RADIUS_SM), fill);
        match st {
            HistoryState::Selected | HistoryState::Editing => {
                inset(p, rect, RADIUS_SM, t.sel_border)
            }
            HistoryState::Failed => inset(p, rect, RADIUS_SM, t.danger_border),
            _ => {}
        }
        if response.has_focus() {
            focus_ring(p, t, rect, RADIUS_SM);
        }
        let cy = rect.center().y;
        let idx = galley(p, &self.index.to_string(), SECTION, fonts::mono());
        p.galley(
            pos2(rect.left() + 8.0, cy - idx.size().y / 2.0),
            idx,
            t.ink7.gamma_multiply(op),
        );
        let ic = Rect::from_center_size(
            pos2(rect.left() + 8.0 + 12.0 + 9.0 + 10.0, cy),
            vec2(20.0, 20.0),
        );
        let style = IconStyle {
            tint: t.ink3,
            bg: fill,
            grey: false,
        };
        icon::paint(p, ic, self.icon, l.dark, style, op);
        let x = ic.right() + 9.0;
        let mut right = rect.right() - 8.0;
        let mut committed = false;
        if st == HistoryState::Failed && icon::exists("failed") {
            let f = Rect::from_center_size(pos2(right - 9.0, cy), vec2(18.0, 18.0));
            icon::paint(
                p,
                f,
                "failed",
                l.dark,
                IconStyle {
                    tint: t.danger_strong,
                    bg: fill,
                    grey: false,
                },
                1.0,
            );
            right -= 18.0 + 9.0;
        }
        match (st, edit) {
            (HistoryState::Editing, Some(text)) => {
                let field = Rect::from_min_size(pos2(right - 78.0, cy - 13.0), vec2(78.0, 26.0));
                committed = edit_field(ui, self.index, field, text);
                right -= 78.0 + 9.0;
            }
            _ => {
                if let Some(meta) = self.meta {
                    let g = galley(p, meta, 11.5, fonts::mono());
                    let pos = pos2(right - g.size().x, cy - g.size().y / 2.0);
                    p.galley(pos, g.clone(), t.ink5.gamma_multiply(op));
                    right -= g.size().x + 9.0;
                }
            }
        }
        let (family, colour) = match st {
            HistoryState::Selected | HistoryState::Editing => (fonts::medium(), t.sel_text),
            HistoryState::Failed => (fonts::medium(), t.danger_strong),
            HistoryState::Suppressed => (FontFamily::Proportional, t.ink5),
            _ => (FontFamily::Proportional, t.tree_label),
        };
        let colour = colour.gamma_multiply(op);
        let p = &ui.painter().clone();
        let g = if st == HistoryState::Suppressed {
            struck(p, self.label, colour, right - x)
        } else {
            fitted(p, self.label, FontId::new(BODY, family), colour, right - x)
        };
        p.galley(pos2(x, cy - g.size().y / 2.0), g, colour);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ListItem, true, self.label));
        HistoryOutput {
            double_clicked: response.double_clicked(),
            response,
            committed,
        }
    }
}

fn struck(
    p: &egui::Painter,
    text: &str,
    colour: Color32,
    max_w: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::simple_singleline(
        text.to_owned(),
        FontId::new(BODY, FontFamily::Proportional),
        colour,
    );
    if let Some(section) = job.sections.first_mut() {
        section.format = TextFormat {
            strikethrough: egui::Stroke::new(1.0, colour),
            ..section.format.clone()
        };
    }
    job.wrap.max_width = max_w.max(0.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    p.layout_job(job)
}

/// Inline value editor; focuses itself the first frame it appears.
fn edit_field(ui: &mut Ui, index: usize, rect: Rect, text: &mut String) -> bool {
    let seen = ui.make_persistent_id(("history-edit-seen", index));
    let first = ui.data_mut(|d| d.get_temp::<bool>(seen).is_none());
    ui.data_mut(|d| d.insert_temp(seen, true));
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    let r = child.add(
        NumberField::new(("history-edit", index), text)
            .unit("mm")
            .width(78.0)
            .compact(true),
    );
    if first {
        r.request_focus();
    }
    let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
    let done = r.lost_focus() && !escape;
    if r.lost_focus() {
        ui.data_mut(|d| d.remove::<bool>(seen));
    }
    done
}

/// Accent line marking the rollback position; drag it to roll back (caller decides).
pub struct RollbackMarker;

impl RollbackMarker {
    pub fn new() -> Self {
        Self
    }
}

impl Default for RollbackMarker {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for RollbackMarker {
    fn ui(self, ui: &mut Ui) -> Response {
        let l = look(ui.ctx());
        let a = l.theme.accent;
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::drag());
        let response = response.on_hover_cursor(CursorIcon::Grab);
        let p = ui.painter();
        let cy = rect.center().y;
        p.rect_filled(
            Rect::from_center_size(pos2(rect.left() + 4.0 + 7.0, cy), vec2(14.0, 14.0)),
            CornerRadius::same(4),
            a,
        );
        let g = galley(p, "Rollback", 10.5, fonts::medium());
        let tx = rect.right() - 4.0 - g.size().x;
        p.rect_filled(
            Rect::from_min_max(
                pos2(rect.left() + 4.0 + 14.0 + 6.0, cy - 1.0),
                pos2(tx - 6.0, cy + 1.0),
            ),
            CornerRadius::same(1),
            a,
        );
        p.galley(pos2(tx, cy - g.size().y / 2.0), g, a);
        if response.has_focus() {
            focus_ring(p, &l.theme, rect, 4);
        }
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Slider, true, "Rollback marker"));
        response
    }
}
