//! Document tab strip inside the TitleBar. Pure presentation: the host owns the
//! document list and maps `TabsOut` to app state.

use egui::{Pos2, Rect, Sense, Stroke, Ui, UiBuilder, Vec2, pos2, vec2};

use super::IconButton;
use crate::components::actions::common::fitted;
use crate::components::tooltip::{self, Tip};
use crate::fonts;
use crate::glyph;
use crate::icon::{self, IconStyle};
use crate::metrics::RADIUS_SM;
use crate::theme::Look;

const TAB_H: f32 = 28.0;
const TAB_PAD: f32 = 10.0;
const TAB_MIN: f32 = 96.0;
const TAB_MAX: f32 = 200.0;
const TAB_GAP: f32 = 4.0;
const SLOT: f32 = 16.0;
const ADD: f32 = 28.0;

/// One open document.
#[derive(Clone, Copy)]
pub struct TabSpec<'a> {
    pub id: u64,
    pub title: &'a str,
    pub icon: &'a str,
    pub dirty: bool,
    pub active: bool,
    pub busy: bool,
}

#[derive(Clone, Copy)]
pub struct TabStrip<'a> {
    pub tabs: &'a [TabSpec<'a>],
    /// Home is the visible page: no tab is highlighted.
    pub home_active: bool,
    pub can_add: bool,
}

#[derive(Default)]
pub(super) struct TabsOut {
    pub activate: Option<u64>,
    pub close: Option<u64>,
    pub add: bool,
    pub rects: Vec<(u64, Rect)>,
    pub ids: Vec<(u64, egui::Id)>,
    pub add_id: Option<egui::Id>,
}

/// A tab's content width: icon, title and the trailing close/dirty slot.
fn natural_width(ui: &Ui, spec: &TabSpec<'_>) -> f32 {
    let font = egui::FontId::new(13.0, fonts::semibold());
    let title = ui
        .painter()
        .layout_no_wrap(spec.title.to_owned(), font, egui::Color32::WHITE)
        .size()
        .x;
    (TAB_PAD + 16.0 + 6.0 + title + 6.0 + SLOT + TAB_PAD).clamp(TAB_MIN, TAB_MAX)
}

/// Content-sized tabs; when they do not fit, the widest shrink first (a common
/// cap), never below the minimum. Past that the strip clips.
fn tab_widths(naturals: &[f32], avail: f32) -> Vec<f32> {
    let room = avail - ADD - naturals.len() as f32 * TAB_GAP;
    if naturals.iter().sum::<f32>() <= room {
        return naturals.to_vec();
    }
    let mut sorted = naturals.to_vec();
    sorted.sort_by(f32::total_cmp);
    let mut left = room;
    let mut cap = TAB_MIN;
    for (i, w) in sorted.iter().enumerate() {
        let share = left / (sorted.len() - i) as f32;
        if *w > share {
            cap = share.max(TAB_MIN);
            break;
        }
        left -= w;
        cap = *w;
    }
    naturals.iter().map(|w| w.min(cap).max(TAB_MIN)).collect()
}

/// Lays the strip centred on `centre_x` within `area`; `sym_w` is the symmetric room.
pub(super) fn show_strip(
    ui: &mut Ui,
    area: Rect,
    centre_x: f32,
    sym_w: f32,
    strip: TabStrip<'_>,
    l: &Look,
) -> TabsOut {
    let n = strip.tabs.len();
    let naturals: Vec<f32> = strip.tabs.iter().map(|t| natural_width(ui, t)).collect();
    let widths = tab_widths(&naturals, sym_w);
    let total = widths.iter().map(|w| w + TAB_GAP).sum::<f32>() + ADD;
    let x0 = (centre_x - total / 2.0).max(area.left());
    let cy = area.center().y;
    let mut out = TabsOut::default();
    let builder = UiBuilder::new().max_rect(area).id_salt("titlebar-tabs");
    ui.scope_builder(builder, |ui| {
        ui.set_clip_rect(area.intersect(ui.clip_rect()));
        let mut x = x0;
        for (i, spec) in strip.tabs.iter().enumerate() {
            let w = widths[i];
            let rect = Rect::from_min_size(pos2(x, cy - TAB_H / 2.0), vec2(w, TAB_H));
            let hl = highlighted(&strip, i);
            tab(ui, rect, spec, hl, l, &mut out);
            if i + 1 < n && !hl && !highlighted(&strip, i + 1) {
                separator(ui, pos2(x + w + TAB_GAP / 2.0, cy), l);
            }
            out.rects.push((spec.id, rect));
            x += w + TAB_GAP;
        }
        let r = Rect::from_min_size(pos2(x, cy - ADD / 2.0), Vec2::splat(ADD));
        let b = IconButton::new("plus", "New document", ADD, 16.0)
            .enabled(strip.can_add)
            .gap_bg(l.theme.titlebar)
            .tip(Some(Tip::new("New document")));
        let add = ui.put(r, b);
        out.add = add.clicked();
        out.add_id = Some(add.id);
    });
    out
}

fn highlighted(strip: &TabStrip<'_>, i: usize) -> bool {
    strip.tabs[i].active && !strip.home_active
}

fn separator(ui: &Ui, c: Pos2, l: &Look) {
    let line = Rect::from_center_size(c, vec2(1.0, 16.0));
    ui.painter().rect_filled(line, 0.0, l.theme.border);
}

fn tab(ui: &mut Ui, rect: Rect, spec: &TabSpec<'_>, hl: bool, l: &Look, out: &mut TabsOut) {
    let id = ui.scope_id().with(("tab", spec.id));
    let body = ui.interact(rect, id, Sense::click());
    let hover = ui.rect_contains_pointer(rect);
    // A dirty or busy tab keeps its marker until the pointer arrives.
    let show_x = hover || (hl && !spec.dirty && !spec.busy);
    let centre = pos2(rect.right() - TAB_PAD - SLOT / 2.0, rect.center().y);
    let slot = Rect::from_center_size(centre, Vec2::splat(SLOT));
    let x_hit = show_x.then(|| ui.interact(slot, id.with("x"), Sense::click()));
    body.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, spec.title));
    out.ids.push((spec.id, body.id));
    let pose = Pose { hl, hover };
    paint_tab(ui, rect, slot, spec, pose, x_hit.as_ref(), l);
    if x_hit.as_ref().is_some_and(|r| r.clicked()) || body.middle_clicked() {
        out.close = Some(spec.id);
    } else if body.clicked() {
        out.activate = Some(spec.id);
    }
    tooltip::attach(body, Tip::new(spec.title));
}

#[derive(Clone, Copy)]
struct Pose {
    hl: bool,
    hover: bool,
}

fn paint_tab(
    ui: &Ui,
    rect: Rect,
    slot: Rect,
    spec: &TabSpec<'_>,
    pose: Pose,
    x_hit: Option<&egui::Response>,
    l: &Look,
) {
    let t = &l.theme;
    let p = ui.painter();
    let radius = egui::CornerRadius::same(RADIUS_SM);
    let fill = if pose.hl {
        let border = Stroke::new(1.0, t.border);
        p.rect(rect, radius, t.surface, border, egui::StrokeKind::Inside);
        t.surface
    } else if pose.hover {
        p.rect_filled(rect, radius, t.hover);
        t.hover
    } else {
        t.titlebar
    };
    let cy = rect.center().y;
    let gc = pos2(rect.left() + TAB_PAD + 8.0, cy);
    let glyph_box = Rect::from_center_size(gc, Vec2::splat(16.0));
    let style = IconStyle {
        tint: t.ink3,
        bg: fill,
        grey: false,
    };
    icon::paint(p, glyph_box, spec.icon, l.dark, style, 1.0);
    let family = if pose.hl {
        fonts::semibold()
    } else {
        egui::FontFamily::Proportional
    };
    let colour = if pose.hl { t.ink } else { t.titlebar_text };
    let tx = glyph_box.right() + 6.0;
    let max_w = slot.left() - 6.0 - tx;
    let font = egui::FontId::new(13.0, family);
    let galley = fitted(p, spec.title, font, colour, max_w);
    p.galley(pos2(tx, cy - galley.size().y / 2.0), galley, colour);
    paint_trailing(p, slot, spec, x_hit, l);
}

/// Close × when `x_hit` exists, else the dirty / busy dot.
fn paint_trailing(
    p: &egui::Painter,
    slot: Rect,
    spec: &TabSpec<'_>,
    x_hit: Option<&egui::Response>,
    l: &Look,
) {
    let t = &l.theme;
    if let Some(r) = x_hit {
        if r.hovered() {
            p.rect_filled(slot, egui::CornerRadius::same(4), t.hover2);
        }
        glyph::close(p, slot.center(), 12.0, t.ink3);
    } else if spec.dirty || spec.busy {
        let dot = if spec.busy { t.accent } else { t.ink4 };
        p.circle_filled(slot.center(), 3.0, dot);
    }
}

#[cfg(test)]
mod width_tests {
    use super::*;

    #[test]
    fn tabs_keep_their_content_width_until_the_strip_overflows() {
        assert_eq!(tab_widths(&[120.0, 150.0], 1000.0), vec![120.0, 150.0]);
        let squeezed = tab_widths(&[120.0, 200.0, 200.0], 400.0);
        assert_eq!(
            squeezed[0], 120.0,
            "a short tab is not stretched or squeezed first"
        );
        assert!(squeezed[1] < 200.0 && squeezed[1] == squeezed[2]);
        let room = 400.0 - ADD - 3.0 * TAB_GAP;
        assert!(squeezed.iter().sum::<f32>() <= room + 0.01);
        assert!(tab_widths(&[200.0; 8], 300.0).iter().all(|w| *w == TAB_MIN));
    }
}
