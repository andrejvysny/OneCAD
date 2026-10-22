//! Passive captions fit their full measured text around editor chrome and handle tips.
use crate::{app::App, layout::Chrome};
use egui::{Color32, FontId, Galley, Painter, Pos2, Rect, Stroke, Vec2, pos2, vec2};
use std::sync::Arc;

const FONT_SIZE: f32 = 12.0;
const PADDING: Vec2 = vec2(4.0, 3.0);
const GAP: f32 = 4.0;
const TIP_GAP: f32 = 14.0;
const MIN_WRAP: f32 = 48.0;

pub(super) fn number(value: f64) -> String {
    let rounded = if value.is_finite() {
        (value * 100.0).round() / 100.0
    } else {
        0.0
    };
    format!("{:.2}", if rounded == 0.0 { 0.0 } else { rounded })
}

pub(super) fn caption_text(
    target: &super::Target,
    drag: Option<&super::Drag>,
    value: f64,
) -> String {
    drag.map_or(target, |drag| &drag.target).label(value)
}

pub(super) fn paint(app: &App, ui: &egui::Ui, viewport: Rect, captions: &[(Pos2, String)]) {
    let painter = ui.painter().with_clip_rect(viewport);
    let tips: Vec<_> = captions.iter().map(|(tip, _)| *tip).collect();
    let mut layout = Layout::new(viewport, chrome(app, ui.ctx()), &tips);
    for (tip, text) in captions {
        let Some(caption) = layout.caption(&painter, text, *tip, app.theme.ink) else {
            continue;
        };
        let nearest = caption.bounds.clamp(*tip);
        if nearest.distance(*tip) > TIP_GAP + 4.0 {
            let start = *tip + (nearest - *tip).normalized() * 9.0;
            painter.line_segment(
                [start, nearest],
                Stroke::new(1.0, app.theme.accent.gamma_multiply(0.65)),
            );
        }
        painter.rect_filled(caption.bounds, 4.0, app.theme.floating);
        painter.galley(caption.bounds.min + PADDING, caption.galley, app.theme.ink);
    }
}

fn chrome(app: &App, ctx: &egui::Context) -> Vec<Rect> {
    let running = app
        .doc
        .interaction
        .tool
        .as_ref()
        .is_some_and(|t| !t.editing_existing);
    let chrome = crate::layout::chrome(ctx.content_rect(), &app.ui);
    let chrome = if running {
        chrome.with_feature_card()
    } else {
        chrome
    };
    let mut covers = vec![chrome.view_controls, cube(&chrome)];
    covers.push(if app.ui.explorer_open {
        chrome.explorer
    } else {
        chrome.explorer_toggle
    });
    if app.ui.inspector_open {
        covers.push(chrome.inspector);
    }
    if let Some(toggle) = chrome.inspector_toggle {
        covers.push(toggle);
    }
    if running {
        covers.push(chrome.feature_card);
    }
    for (id, fallback) in [
        ("tool-pill", toolbar(&chrome)),
        (
            "nav-pill",
            Rect::from_min_size(chrome.nav_pill - vec2(0.0, 38.0), vec2(174.0, 38.0)),
        ),
    ] {
        covers.push(
            ctx.memory(|memory| memory.area_rect(egui::Id::unique(id)))
                .unwrap_or(fallback),
        );
    }
    covers
}
fn cube(chrome: &Chrome) -> Rect {
    crate::navigation_cube::rect(Rect::from_min_max(
        pos2(chrome.viewport.left(), chrome.view_cluster.y),
        pos2(chrome.view_cluster.x, chrome.viewport.bottom()),
    ))
}
fn toolbar(chrome: &Chrome) -> Rect {
    let left = chrome.toolbar_left;
    let top = chrome.viewport.top() + 10.0;
    let mut right = chrome.view_cluster.x - crate::layout::CUBE - 10.0;
    if chrome.view_controls.top() < top + 48.0 {
        right = right.min(chrome.view_controls.left() - 10.0);
    }
    Rect::from_min_size(pos2(left, top), vec2((right - left).max(120.0), 48.0))
}

struct Caption {
    bounds: Rect,
    galley: Arc<Galley>,
}
struct Layout {
    viewport: Rect,
    blocked: Vec<Rect>,
    placed: Vec<Rect>,
}
impl Layout {
    fn new(viewport: Rect, covers: Vec<Rect>, tips: &[Pos2]) -> Self {
        let mut blocked: Vec<_> = covers.into_iter().map(|rect| rect.expand(GAP)).collect();
        blocked.extend(
            tips.iter()
                .map(|&tip| Rect::from_center_size(tip, vec2(20.0, 20.0))),
        );
        Self {
            viewport: viewport.shrink(GAP),
            blocked,
            placed: Vec::new(),
        }
    }
    fn caption(
        &mut self,
        painter: &Painter,
        text: &str,
        tip: Pos2,
        color: Color32,
    ) -> Option<Caption> {
        let font = FontId::proportional(FONT_SIZE);
        let full = painter.layout_no_wrap(text.to_owned(), font.clone(), color);
        let widths = self.wrap_widths(full.size().x);
        for avoid_labels in [true, false] {
            let wrapped = widths
                .iter()
                .map(|width| painter.layout(text.to_owned(), font.clone(), color, *width));
            for galley in std::iter::once(full.clone()).chain(wrapped) {
                if let Some(bounds) = self.bounds(galley.size() + 2.0 * PADDING, tip, avoid_labels)
                {
                    self.placed.push(bounds.expand(GAP));
                    return Some(Caption { bounds, galley });
                }
            }
        }
        None
    }
    fn bounds(&self, size: Vec2, tip: Pos2, avoid_labels: bool) -> Option<Rect> {
        let mut blocked = self.blocked.clone();
        if avoid_labels {
            blocked.extend(&self.placed);
        }
        place(size, tip, self.viewport, &blocked)
    }
    fn wrap_widths(&self, full: f32) -> Vec<f32> {
        let mut edges = vec![self.viewport.left(), self.viewport.right()];
        edges.extend(
            self.blocked
                .iter()
                .chain(&self.placed)
                .flat_map(|r| [r.left(), r.right()]),
        );
        let mut widths: Vec<_> = edges
            .iter()
            .flat_map(|left| {
                edges
                    .iter()
                    .map(move |right| right - left - 2.0 * PADDING.x)
            })
            .filter(|w| *w >= MIN_WRAP && *w < full)
            .collect();
        widths.push(MIN_WRAP.min(full));
        widths.sort_by(|a, b| b.total_cmp(a));
        widths.dedup_by(|a, b| (*a - *b).abs() < 1.0);
        widths
    }
}

fn place(size: Vec2, tip: Pos2, viewport: Rect, blocked: &[Rect]) -> Option<Rect> {
    if !size.is_finite() || size.x > viewport.width() || size.y > viewport.height() {
        return None;
    }
    let mut xs = vec![
        tip.x + TIP_GAP,
        tip.x - TIP_GAP - size.x,
        tip.x - size.x / 2.0,
        viewport.left(),
        viewport.right() - size.x,
    ];
    let mut ys = vec![
        tip.y - TIP_GAP - size.y,
        tip.y + TIP_GAP,
        tip.y - size.y / 2.0,
        viewport.top(),
        viewport.bottom() - size.y,
    ];
    for obstacle in blocked {
        xs.extend([obstacle.left() - size.x, obstacle.right()]);
        ys.extend([obstacle.top() - size.y, obstacle.bottom()]);
    }
    let mut best: Option<(f32, Rect)> = None;
    for x in xs {
        for &y in &ys {
            let rect = Rect::from_min_size(
                pos2(
                    x.clamp(viewport.left(), viewport.right() - size.x),
                    y.clamp(viewport.top(), viewport.bottom() - size.y),
                ),
                size,
            );
            if blocked.iter().any(|cover| overlaps(rect, *cover)) {
                continue;
            }
            let score = rect.clamp(tip).distance_sq(tip);
            if best.is_none_or(|(previous, _)| score < previous) {
                best = Some((score, rect));
            }
        }
    }
    best.map(|(_, rect)| rect)
}
fn overlaps(a: Rect, b: Rect) -> bool {
    a.left() < b.right() && a.right() > b.left() && a.top() < b.bottom() && a.bottom() > b.top()
}

#[cfg(test)]
#[path = "labels_tests.rs"]
mod tests;
