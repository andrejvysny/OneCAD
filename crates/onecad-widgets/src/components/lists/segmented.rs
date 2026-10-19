//! SegmentedControl (c-tabs): h30 well track r7 pad 2, white thumb r5, 12.5 labels.

use egui::epaint::Shadow;
use egui::{
    Color32, CornerRadius, FontFamily, FontId, IdSalt, Rect, Response, Sense, Ui, Vec2, Widget,
    pos2, vec2,
};

use crate::components::actions::common::{fitted, focus_ring, galley};
use crate::fonts;
use crate::icon::{self, IconStyle};
use crate::metrics::CONTROL;
use crate::theme::look;

#[derive(Clone, Copy, Debug)]
pub struct Segment<'a> {
    pub label: &'a str,
    pub icon: Option<&'a str>,
}

pub struct SegmentedControl<'a> {
    id_salt: IdSalt,
    selected: &'a mut usize,
    segments: &'a [Segment<'a>],
    width: Option<f32>,
}

impl<'a> SegmentedControl<'a> {
    pub fn new(
        id_salt: impl std::hash::Hash + std::fmt::Debug,
        selected: &'a mut usize,
        segments: &'a [Segment<'a>],
    ) -> Self {
        Self {
            id_salt: IdSalt::new(id_salt),
            selected,
            segments,
            width: None,
        }
    }
    /// Fixed total width, split equally; unset = content width.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }
}

const THUMB_SHADOW: Shadow = Shadow {
    offset: [0, 1],
    blur: 2,
    spread: 0,
    color: Color32::from_black_alpha(20),
};

/// Label + icon content width of a segment (semibold, the widest state).
fn content_width(ui: &Ui, s: &Segment<'_>, size: f32) -> f32 {
    let text = galley(ui.painter(), s.label, size, fonts::semibold())
        .size()
        .x;
    text + s.icon.map_or(0.0, |_| 16.0 + 5.0)
}

const PAD: f32 = 20.0;
const PAD_MIN: f32 = 6.0;
/// Smaller label size used when padded labels at `CONTROL` size do not fit.
const CONTROL_TIGHT: f32 = 11.5;

/// Equal split of `avail`, except segments whose `floor` exceeds the share keep it.
fn waterfill(floors: &[f32], avail: f32) -> Vec<f32> {
    let mut fixed = vec![false; floors.len()];
    loop {
        let rest = avail
            - floors
                .iter()
                .zip(&fixed)
                .filter(|(_, f)| **f)
                .map(|(w, _)| w)
                .sum::<f32>();
        let free = fixed.iter().filter(|f| !**f).count();
        if free == 0 {
            return floors.to_vec();
        }
        let share = rest / free as f32;
        let mut grew = false;
        for (i, f) in floors.iter().enumerate() {
            if !fixed[i] && *f > share {
                fixed[i] = true;
                grew = true;
            }
        }
        if !grew {
            return floors
                .iter()
                .zip(&fixed)
                .map(|(w, f)| if *f { *w } else { share })
                .collect();
        }
    }
}

/// Segment widths for `contents` inside `avail`: padded equal split when it fits,
/// squeezed padding next, and only then proportional shares (labels get elided).
fn segment_widths(contents: &[f32], avail: f32) -> Vec<f32> {
    let ideal: Vec<f32> = contents.iter().map(|c| c + PAD).collect();
    let floor: Vec<f32> = contents.iter().map(|c| c + PAD_MIN).collect();
    let (ideal_sum, floor_sum) = (ideal.iter().sum::<f32>(), floor.iter().sum::<f32>());
    if ideal_sum <= avail {
        waterfill(&ideal, avail)
    } else if floor_sum <= avail {
        let span = ideal_sum - floor_sum;
        let k = (avail - floor_sum) / span.max(f32::EPSILON);
        floor
            .iter()
            .zip(&ideal)
            .map(|(f, i)| f + (i - f) * k)
            .collect()
    } else {
        let total = contents.iter().sum::<f32>().max(f32::EPSILON);
        contents.iter().map(|c| avail * c / total).collect()
    }
}

impl Widget for SegmentedControl<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let l = look(ui.ctx());
        let t = &l.theme;
        let measure = |size| -> Vec<f32> {
            let v = self.segments.iter().map(|s| content_width(ui, s, size));
            v.collect()
        };
        let mut font = CONTROL;
        let mut contents = measure(font);
        if let Some(w) = self.width {
            let need = contents.iter().sum::<f32>() + PAD_MIN * contents.len() as f32;
            if need > w - 4.0 {
                font = CONTROL_TIGHT;
                contents = measure(font);
            }
        }
        let widths = match self.width {
            Some(w) => segment_widths(&contents, w - 4.0),
            None => contents.iter().map(|c| c + PAD).collect(),
        };
        let total = widths.iter().sum::<f32>() + 4.0;
        let base = ui.make_persistent_id(self.id_salt);
        let (rect, mut response) = ui.allocate_exact_size(vec2(total, 30.0), Sense::hover());
        ui.painter()
            .rect_filled(rect, CornerRadius::same(7), t.well);
        let mut x = rect.left() + 2.0;
        let mut changed = false;
        for (i, (seg, w)) in self.segments.iter().zip(&widths).enumerate() {
            let seg_rect = Rect::from_min_size(pos2(x, rect.top() + 2.0), vec2(*w, 26.0));
            x += w;
            let r = ui.interact(seg_rect, base.with(i), Sense::click());
            let on = *self.selected == i;
            if r.clicked() && !on {
                *self.selected = i;
                changed = true;
            }
            let p = ui.painter();
            if on {
                p.add(THUMB_SHADOW.as_shape(seg_rect, CornerRadius::same(5)));
                p.rect_filled(seg_rect, CornerRadius::same(5), t.surface);
            }
            if r.has_focus() {
                focus_ring(p, t, seg_rect, 5);
            }
            let (family, colour) = match (on, r.hovered()) {
                (true, _) => (fonts::semibold(), t.ink),
                (false, true) => (FontFamily::Proportional, t.ink2),
                (false, false) => (FontFamily::Proportional, t.ink4),
            };
            let icon_w = seg.icon.map_or(0.0, |_| 16.0 + 5.0);
            let g = fitted(
                p,
                seg.label,
                FontId::new(font, family),
                Color32::PLACEHOLDER,
                seg_rect.width() - PAD_MIN - icon_w,
            );
            let mut gx = seg_rect.center().x - (g.size().x + icon_w) / 2.0;
            if let Some(name) = seg.icon {
                let ic =
                    Rect::from_min_size(pos2(gx, seg_rect.center().y - 8.0), Vec2::splat(16.0));
                let style = IconStyle {
                    tint: colour,
                    bg: if on { t.surface } else { t.well },
                    grey: false,
                };
                icon::paint(p, ic, name, l.dark, style, 1.0);
                gx += icon_w;
            }
            p.galley(pos2(gx, seg_rect.center().y - g.size().y / 2.0), g, colour);
            r.widget_info(|| {
                egui::WidgetInfo::selected(egui::Role::RadioButton, true, on, seg.label)
            });
        }
        if changed {
            response.mark_changed();
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::segment_widths;

    #[test]
    fn widths_never_overlap_labels() {
        // "One side / Symmetric / Two" at the 170px card row.
        let contents = [48.0, 62.0, 26.0];
        for avail in [166.0, 154.0, 140.0, 80.0] {
            let w = segment_widths(&contents, avail);
            assert!(
                (w.iter().sum::<f32>() - avail).abs() < 0.01,
                "{avail}: {w:?}"
            );
            if avail >= 154.0 {
                for (w, c) in w.iter().zip(contents) {
                    assert!(*w >= c + 6.0, "{avail}: {w} < {c}");
                }
            }
        }
        let fits = segment_widths(&contents, 166.0);
        assert!(fits[1] > fits[0] && fits[0] > fits[2]);
    }
}
