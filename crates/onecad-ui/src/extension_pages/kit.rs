//! Small painting helpers shared by the extensions pages (chips, links, tabs, icon wells).
use super::catalog::Trust;
use egui::{
    Align2, Color32, CornerRadius, FontFamily, FontId, Painter, Pos2, Rect, Response, Sense,
    Stroke, StrokeKind, Ui, Vec2, pos2,
    text::{LayoutJob, TextFormat},
    vec2,
};
use onecad_widgets::{
    fonts,
    icon::{self, IconStyle},
    metrics::{SECTION, SECTION_TRACKING},
    theme::{Look, Theme, look},
};

pub fn fid(size: f32, family: FontFamily) -> FontId {
    FontId::new(size, family)
}

/// Text with its top-left at `pos`; returns the painted rect.
pub fn text(
    p: &Painter,
    pos: Pos2,
    s: &str,
    size: f32,
    family: FontFamily,
    color: Color32,
) -> Rect {
    p.text(pos, Align2::LEFT_TOP, s, fid(size, family), color)
}

/// Text right-aligned to `right` with its vertical centre at `cy`.
pub fn text_right(p: &Painter, right: f32, cy: f32, s: &str, size: f32, color: Color32) -> Rect {
    p.text(
        pos2(right, cy),
        Align2::RIGHT_CENTER,
        s,
        fid(size, FontFamily::Proportional),
        color,
    )
}

/// Wrapped paragraph with a fixed line height; returns its size.
pub fn paragraph(
    p: &Painter,
    pos: Pos2,
    s: &str,
    size: f32,
    line: f32,
    width: f32,
    color: Color32,
) -> Vec2 {
    let mut job = LayoutJob::single_section(
        s.to_owned(),
        TextFormat {
            font_id: fid(size, FontFamily::Proportional),
            color,
            line_height: Some(line),
            ..TextFormat::default()
        },
    );
    job.wrap.max_width = width;
    let g = p.layout_job(job);
    let size = g.size();
    p.galley(pos, g, color);
    size
}

/// Wrapped paragraph vertically centred on `cy`, starting at `x`.
pub fn paragraph_centered(
    p: &Painter,
    at: Pos2,
    s: &str,
    size: f32,
    line: f32,
    width: f32,
    color: Color32,
) {
    let mut job = LayoutJob::single_section(
        s.to_owned(),
        TextFormat {
            font_id: fid(size, FontFamily::Proportional),
            color,
            line_height: Some(line),
            ..TextFormat::default()
        },
    );
    job.wrap.max_width = width;
    let g = p.layout_job(job);
    p.galley(pos2(at.x, at.y - g.size().y / 2.0), g, color);
}

/// Fragments (text, is_mono) laid out as one wrapped line of mixed fonts.
pub fn mixed_job(parts: &[(&str, bool)], size: f32, color: Color32, width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    for (s, mono) in parts {
        let family = if *mono {
            fonts::mono()
        } else {
            FontFamily::Proportional
        };
        job.append(
            s,
            0.0,
            TextFormat {
                font_id: fid(size, family),
                color,
                ..TextFormat::default()
            },
        );
    }
    job.wrap.max_width = width;
    job
}

pub fn well(
    p: &Painter,
    rect: Rect,
    icon_name: &str,
    radius: u8,
    icon_px: f32,
    th: &Theme,
    dark: bool,
) {
    p.rect_filled(rect, CornerRadius::same(radius), th.well);
    let style = IconStyle {
        tint: th.ink2,
        bg: th.well,
        grey: false,
    };
    let r = Rect::from_center_size(rect.center(), Vec2::splat(icon_px));
    icon::paint(p, r, icon_name, dark, style, 1.0);
}

pub fn icon_at(
    p: &Painter,
    center: Pos2,
    px: f32,
    name: &str,
    tint: Color32,
    bg: Color32,
    dark: bool,
) {
    let r = Rect::from_center_size(center, Vec2::splat(px));
    icon::paint(
        p,
        r,
        name,
        dark,
        IconStyle {
            tint,
            bg,
            grey: false,
        },
        1.0,
    );
}

/// 11/500 caps caption with 0.07em tracking, top-left at `pos`.
pub fn caption_at(p: &Painter, pos: Pos2, s: &str, color: Color32) -> Rect {
    let mut job = LayoutJob::single_section(
        s.to_uppercase(),
        TextFormat {
            font_id: fid(SECTION, fonts::medium()),
            color,
            extra_letter_spacing: SECTION_TRACKING,
            ..TextFormat::default()
        },
    );
    job.wrap.max_width = f32::INFINITY;
    let g = p.layout_job(job);
    let rect = Rect::from_min_size(pos, g.size());
    p.galley(pos, g, color);
    rect
}

/// Caption row allocated in the layout.
pub fn caption(ui: &mut Ui, s: &str) {
    let color = look(ui.ctx()).theme.ink5;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
    caption_at(ui.painter(), rect.min, s, color);
}

fn chip_colors(th: &Theme, trust: Trust) -> (Color32, Color32) {
    match trust {
        Trust::Verified => (th.dof_ok.gamma_multiply(0.14), th.ok_text),
        Trust::Community => (th.chip, th.ink4),
    }
}

pub fn trust_label(trust: Trust) -> &'static str {
    match trust {
        Trust::Verified => "Verified",
        Trust::Community => "Community",
    }
}

/// Trust chip (h18, r4) with its left edge at `pos.x` and vertical centre at `pos.y`.
pub fn trust_chip(
    p: &Painter,
    pos: Pos2,
    trust: Trust,
    check: bool,
    th: &Theme,
    dark: bool,
) -> Rect {
    let (bg, fg) = chip_colors(th, trust);
    let g = p.layout_no_wrap(
        trust_label(trust).to_owned(),
        fid(11.0, fonts::medium()),
        fg,
    );
    let check_w = if check && trust == Trust::Verified {
        13.0
    } else {
        0.0
    };
    let rect = Rect::from_min_size(
        pos2(pos.x, pos.y - 9.0),
        vec2(g.size().x + 12.0 + check_w, 18.0),
    );
    p.rect_filled(rect, CornerRadius::same(4), bg);
    if check_w > 0.0 {
        icon_at(
            p,
            pos2(rect.left() + 11.0, rect.center().y),
            10.0,
            "check",
            fg,
            bg,
            dark,
        );
    }
    p.galley(
        pos2(
            rect.left() + 6.0 + check_w,
            rect.center().y - g.size().y / 2.0,
        ),
        g,
        fg,
    );
    rect
}

/// "Publisher [chip] tail" meta line; returns the right edge.
pub fn publisher_line(
    p: &Painter,
    left: f32,
    cy: f32,
    publisher: &str,
    trust: Trust,
    tail: Option<&str>,
    l: &Look,
) -> f32 {
    let th = &l.theme;
    let r = p.text(
        pos2(left, cy),
        Align2::LEFT_CENTER,
        publisher,
        fid(12.5, FontFamily::Proportional),
        th.ink4,
    );
    let chip = trust_chip(
        p,
        pos2(r.right() + 6.0, cy),
        trust,
        tail.is_some(),
        th,
        l.dark,
    );
    let Some(tail) = tail else {
        return chip.right();
    };
    p.text(
        pos2(chip.right() + 8.0, cy),
        Align2::LEFT_CENTER,
        tail,
        fid(11.5, fonts::mono()),
        th.ink5,
    )
    .right()
}

/// Inline accent link; returns its response.
pub fn link(ui: &mut Ui, s: &str, color: Color32, size: f32) -> Response {
    let g = ui
        .painter()
        .layout_no_wrap(s.to_owned(), fid(size, FontFamily::Proportional), color);
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x, 20.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let shade = if resp.hovered() {
        color.gamma_multiply(0.8)
    } else {
        color
    };
    ui.painter().galley(
        pos2(rect.left(), rect.center().y - g.size().y / 2.0),
        g,
        shade,
    );
    resp
}

/// Accent-soft button (h30, r6) used for Update/Review.
pub fn soft_button(ui: &mut Ui, label: &str) -> Response {
    let th = look(ui.ctx()).theme;
    let g = ui
        .painter()
        .layout_no_wrap(label.to_owned(), fid(12.5, fonts::medium()), th.sel_text);
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 24.0, 30.0), Sense::click());
    let fill = if resp.hovered() {
        th.sel_bg
    } else {
        th.sel_soft
    };
    let p = ui.painter();
    p.rect(
        rect,
        CornerRadius::same(6),
        fill,
        Stroke::new(1.0, th.sel_border),
        StrokeKind::Inside,
    );
    p.galley(
        pos2(rect.left() + 12.0, rect.center().y - g.size().y / 2.0),
        g,
        th.sel_text,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Underline tabs; each label may carry a muted count or an accent badge. Returns a clicked index.
pub struct TabLabel<'a> {
    pub label: &'a str,
    pub count: Option<usize>,
    pub accent_badge: bool,
}

pub fn tabs(ui: &mut Ui, id: &str, items: &[TabLabel<'_>], selected: usize) -> Option<usize> {
    let th = look(ui.ctx()).theme;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::hover());
    let p = ui.painter().clone();
    p.hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, th.border),
    );
    let mut x = rect.left();
    let mut clicked = None;
    for (i, item) in items.iter().enumerate() {
        let on = i == selected;
        let family = if on {
            fonts::semibold()
        } else {
            FontFamily::Proportional
        };
        let color = if on { th.ink } else { th.ink4 };
        let g = p.layout_no_wrap(item.label.to_owned(), fid(13.0, family), color);
        let extra = item.count.map_or(0.0, |_| 22.0);
        let w = g.size().x + extra;
        let r = Rect::from_min_size(pos2(x, rect.top()), vec2(w, rect.height()));
        let resp = ui.interact(r, ui.scope_id().with((id, i)), Sense::click());
        let cy = rect.center().y - 1.0;
        p.galley(pos2(x, cy - g.size().y / 2.0), g.clone(), color);
        if let Some(n) = item.count {
            let c = pos2(x + g.size().x + 6.0, cy);
            if item.accent_badge {
                p.circle_filled(c + vec2(8.0, 0.0), 8.0, th.accent);
                p.text(
                    c + vec2(8.0, 0.0),
                    Align2::CENTER_CENTER,
                    n.to_string(),
                    fid(10.5, fonts::semibold()),
                    th.on_accent,
                );
            } else {
                p.text(
                    c,
                    Align2::LEFT_CENTER,
                    n.to_string(),
                    fid(11.0, fonts::mono()),
                    th.ink5,
                );
            }
        }
        if on {
            let line = Rect::from_min_max(pos2(x, rect.bottom() - 2.0), pos2(x + w, rect.bottom()));
            p.rect_filled(line, CornerRadius::ZERO, th.accent);
        }
        if resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            clicked = Some(i);
        }
        x += w + 24.0;
    }
    clicked
}

/// Diagonal hatch clipped to `rect`, the placeholder-screenshot texture.
pub fn hatch(p: &Painter, rect: Rect, color: Color32) {
    let clip = p.with_clip_rect(rect);
    let h = rect.height();
    let mut x = rect.left() - h;
    while x < rect.right() {
        clip.line_segment(
            [pos2(x, rect.bottom()), pos2(x + h, rect.top())],
            Stroke::new(1.0, color),
        );
        x += 12.0;
    }
}

/// Quarter-open spinner arc rotating with the context clock.
pub fn spinner(ui: &Ui, center: Pos2, r: f32, color: Color32) {
    let t = ui.ctx().input(|i| i.time) as f32;
    let start = t * std::f32::consts::TAU;
    let pts: Vec<Pos2> = (0..=12)
        .map(|k| {
            let a = start + k as f32 / 12.0 * std::f32::consts::PI * 1.5;
            center + vec2(a.cos(), a.sin()) * r
        })
        .collect();
    ui.painter()
        .add(egui::Shape::line(pts, Stroke::new(1.6, color)));
    ui.ctx().request_repaint();
}

pub fn bordered(p: &Painter, rect: Rect, radius: u8, fill: Color32, border: Color32) {
    p.rect(
        rect,
        CornerRadius::same(radius),
        fill,
        Stroke::new(1.0, border),
        StrokeKind::Inside,
    );
}
