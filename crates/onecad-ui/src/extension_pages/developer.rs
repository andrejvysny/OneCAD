//! Developer section (22a): unpacked extension card with manifest check and log.
use super::{
    kit::{self, fid},
    state::{ExtensionsUi, mock_clock},
};
use egui::{Align, Align2, CornerRadius, Layout, Rect, Sense, Ui, UiBuilder, pos2, vec2};
use onecad_widgets::{
    components::button::Button,
    fonts,
    theme::{Look, look},
};

const HEAD_H: f32 = 60.0;
const BODY_H: f32 = 136.0;

pub fn section(ui: &mut Ui, x: &mut ExtensionsUi) {
    let l = look(ui.ctx());
    let th = &l.theme;
    let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
    kit::caption_at(
        ui.painter(),
        head.left_center() - vec2(0.0, 6.0),
        "Developer",
        th.ink5,
    );
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(head)
            .layout(Layout::right_to_left(Align::Center)),
    );
    child.add(Button::new("Load from folder…").icon(Some("folder")));
    ui.add_space(8.0);
    let (card, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), HEAD_H + BODY_H), Sense::hover());
    kit::bordered(ui.painter(), card, 12, th.surface, th.border);
    top_row(
        ui,
        x,
        &l,
        Rect::from_min_size(card.min, vec2(card.width(), HEAD_H)),
    );
    let body = Rect::from_min_max(pos2(card.left(), card.top() + HEAD_H), card.max);
    ui.painter().hline(
        body.x_range(),
        body.top() - 0.5,
        egui::Stroke::new(1.0, th.border),
    );
    let split = body.left() + body.width() * 0.52;
    manifest(
        ui,
        &l,
        Rect::from_min_max(body.min, pos2(split, body.bottom())),
    );
    log(
        ui,
        &l,
        Rect::from_min_max(pos2(split, body.top()), body.max),
    );
    ui.add_space(12.0);
    ui.add(egui::Label::new(egui::RichText::new("Dev extensions get every permission without prompting and never sync to other machines. Reload keeps the open document and selection.").size(12.5).color(th.ink5)).selectable(false));
}

fn top_row(ui: &mut Ui, x: &mut ExtensionsUi, l: &Look, rect: Rect) {
    let th = &l.theme;
    let p = ui.painter().clone();
    kit::well(
        &p,
        Rect::from_min_size(
            pos2(rect.left() + 14.0, rect.center().y - 16.0),
            vec2(32.0, 32.0),
        ),
        "component",
        8,
        18.0,
        th,
        l.dark,
    );
    let tx = rect.left() + 58.0;
    let name = kit::text(
        &p,
        pos2(tx, rect.top() + 12.0),
        "Bracket Tools",
        13.0,
        fonts::semibold(),
        th.ink,
    );
    let dev = Rect::from_min_size(
        pos2(name.right() + 8.0, rect.top() + 12.0),
        vec2(34.0, 17.0),
    );
    p.rect_filled(dev, CornerRadius::same(4), th.ink);
    p.text(
        dev.center(),
        Align2::CENTER_CENTER,
        "DEV",
        fid(10.0, fonts::mono()),
        th.surface,
    );
    kit::text(
        &p,
        pos2(dev.right() + 10.0, rect.top() + 13.0),
        "com.workshop.bracket-tools · 0.1.0",
        11.5,
        fonts::mono(),
        th.ink5,
    );
    kit::text(
        &p,
        pos2(tx, rect.top() + 33.0),
        "~/dev/bracket-tools",
        11.5,
        fonts::mono(),
        th.ink4,
    );
    let area = Rect::from_min_max(
        pos2(rect.right() - 480.0, rect.top()),
        pos2(rect.right() - 14.0, rect.bottom()),
    );
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(area)
            .layout(Layout::right_to_left(Align::Center)),
    );
    child.spacing_mut().item_spacing.x = 12.0;
    if child.add(Button::new("Reload ⌥⌘R")).clicked() {
        let now = child.ctx().input(|i| i.time) as u32;
        x.reload_developer(now);
    }
    let label = format!("Watching · reloaded {}", mock_clock(x.reloaded_at));
    child.add(
        egui::Label::new(egui::RichText::new(label).size(12.5).color(th.ok_text)).selectable(false),
    );
    let (dot, _) = child.allocate_exact_size(vec2(7.0, 7.0), Sense::hover());
    child.painter().circle_filled(dot.center(), 3.5, th.dof_ok);
}

enum Mark {
    Ok,
    Warn,
    Fail,
}

fn manifest(ui: &mut Ui, l: &Look, rect: Rect) {
    let th = &l.theme;
    kit::caption_at(
        ui.painter(),
        rect.min + vec2(14.0, 12.0),
        "Manifest check",
        th.ink5,
    );
    let rows: [(Mark, &[(&str, bool)]); 4] = [
        (
            Mark::Ok,
            &[("3 tools registered · scopes edge, face", false)],
        ),
        (
            Mark::Warn,
            &[
                ("permissions.files", true),
                (" has no reason. It will show as “No reason given”.", false),
            ],
        ),
        (
            Mark::Warn,
            &[
                ("Icon ", false),
                ("counterbore.svg", true),
                (" is 22 px, so it was scaled to 24.", false),
            ],
        ),
        (
            Mark::Fail,
            &[
                ("Slot ", false),
                ("toolbar.secondary", true),
                (" doesn't exist. Use ", false),
                ("toolbar.contextual", true),
                (".", false),
            ],
        ),
    ];
    for (i, (mark, parts)) in rows.iter().enumerate() {
        let cy = rect.top() + 44.0 + i as f32 * 23.0;
        let name = match mark {
            Mark::Ok => "check",
            Mark::Warn => "warning",
            Mark::Fail => "failed",
        };
        kit::icon_at(
            ui.painter(),
            pos2(rect.left() + 21.0, cy),
            14.0,
            name,
            th.ink,
            th.surface,
            l.dark,
        );
        let job = kit::mixed_job(parts, 12.5, th.ink2, f32::INFINITY);
        let g = ui.painter().layout_job(job);
        ui.painter()
            .galley(pos2(rect.left() + 38.0, cy - g.size().y / 2.0), g, th.ink2);
    }
}

fn log(ui: &mut Ui, l: &Look, rect: Rect) {
    let th = &l.theme;
    let p = ui.painter().clone();
    p.rect_filled(
        rect,
        CornerRadius {
            nw: 0,
            ne: 0,
            sw: 0,
            se: 11,
        },
        th.tooltip,
    );
    let dim = th.tooltip_text.gamma_multiply(0.5);
    let msg = th.tooltip_text.gamma_multiply(0.9);
    kit::caption_at(&p, rect.min + vec2(14.0, 12.0), "Log", dim);
    let lines: [(&str, &str, &str); 4] = [
        ("12:04:31", "", "reload · 3 contributions"),
        ("12:04:31", "", "tool counterbore → pill.edge p40"),
        ("12:04:32", "warn", "icon scaled 22→24"),
        ("12:04:32", "error", "unknown slot toolbar.secondary"),
    ];
    for (i, (time, level, text)) in lines.iter().enumerate() {
        let cy = rect.top() + 44.0 + i as f32 * 24.0;
        let t = p.text(
            pos2(rect.left() + 14.0, cy),
            Align2::LEFT_CENTER,
            time,
            fid(11.5, fonts::mono()),
            dim,
        );
        let mut x = t.right() + 10.0;
        let color = if *level == "warn" {
            th.created_stroke
        } else {
            th.traffic_close
        };
        if !level.is_empty() {
            x = p
                .text(
                    pos2(x, cy),
                    Align2::LEFT_CENTER,
                    level,
                    fid(11.5, fonts::mono()),
                    color,
                )
                .right()
                + 8.0;
        }
        p.text(
            pos2(x, cy),
            Align2::LEFT_CENTER,
            text,
            fid(11.5, fonts::mono()),
            msg,
        );
    }
}
