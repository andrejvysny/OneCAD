//! Installed / Updates lists (21c row states).
use super::{
    kit::{self, fid},
    state::{ExtensionsUi, InstRow, RowState},
};
use egui::{Align, Layout, Rect, RichText, Sense, Ui, UiBuilder, pos2, vec2};
use onecad_widgets::{
    components::{
        button::{Button, ButtonKind},
        inputs::Toggle,
    },
    fonts,
    theme::look,
};

const ROW_H: f32 = 57.0;
const PAD: f32 = 8.0;

/// What a row asked for this frame; applied after painting so rows can be removed.
enum Act {
    Restart,
    Cancel,
    Uninstall,
    Install,
    Review,
    CopyLog,
}

pub fn list(ui: &mut Ui, x: &mut ExtensionsUi, updates_only: bool) {
    let ids: Vec<&'static str> = x
        .rows
        .iter()
        .filter(|r| !updates_only || r.update.is_some())
        .map(|r| r.id)
        .collect();
    if ids.is_empty() {
        let c = look(ui.ctx()).theme.ink5;
        ui.add(
            egui::Label::new(
                RichText::new("Everything is up to date.")
                    .size(13.0)
                    .color(c),
            )
            .selectable(false),
        );
        return;
    }
    let th = look(ui.ctx()).theme;
    let height = ids.len() as f32 * ROW_H + 2.0 * PAD;
    let (frame, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    kit::bordered(ui.painter(), frame, 12, th.surface, th.border);
    let mut acted = None;
    for (n, id) in ids.iter().enumerate() {
        let top = frame.top() + PAD + n as f32 * ROW_H;
        let rect = Rect::from_min_size(
            pos2(frame.left() + PAD, top),
            vec2(frame.width() - 2.0 * PAD, ROW_H),
        );
        if n > 0 {
            ui.painter().hline(
                rect.x_range().shrink(8.0),
                rect.top() - 0.5,
                egui::Stroke::new(1.0, th.border_subtle),
            );
        }
        let Some(idx) = x.rows.iter().position(|r| r.id == *id) else {
            continue;
        };
        if let Some(act) = row(ui, &mut x.rows[idx], rect) {
            acted = Some((*id, act));
        }
    }
    if let Some((id, act)) = acted {
        apply(x, id, act, ui.ctx().input(|i| i.time));
    }
}

fn apply(x: &mut ExtensionsUi, id: &'static str, act: Act, now: f64) {
    match act {
        Act::Restart => x.restart(id),
        Act::Cancel => x.cancel_install(id),
        Act::Uninstall => x.uninstall(id),
        Act::Install => x.request_install(id),
        Act::Review => x.request_update(id),
        Act::CopyLog => x.show_toast("Log copied".to_owned(), now),
    }
}

fn row(ui: &mut Ui, r: &mut InstRow, rect: Rect) -> Option<Act> {
    let l = look(ui.ctx());
    let th = &l.theme;
    let p = ui.painter().clone();
    if r.state == RowState::Failed {
        p.rect(
            rect,
            8,
            th.danger_surface,
            egui::Stroke::new(1.0, th.danger_border),
            egui::StrokeKind::Inside,
        );
    }
    let well = Rect::from_min_size(
        pos2(rect.left() + 10.0, rect.center().y - 16.0),
        vec2(32.0, 32.0),
    );
    kit::well(&p, well, r.icon, 8, 18.0, th, l.dark);
    let tx = well.right() + 12.0;
    let name = kit::text(
        &p,
        pos2(tx, rect.top() + 11.0),
        r.name,
        13.0,
        fonts::semibold(),
        th.ink,
    );
    kit::text(
        &p,
        pos2(name.right() + 6.0, rect.top() + 13.0),
        r.version,
        11.0,
        fonts::mono(),
        th.ink5,
    );
    status_line(ui, r, pos2(tx, rect.top() + 30.0));
    let actions = Rect::from_min_max(
        pos2(rect.right() - 340.0, rect.top()),
        pos2(rect.right() - 12.0, rect.bottom()),
    );
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(actions)
            .layout(Layout::right_to_left(Align::Center)),
    );
    actions_for(&mut child, r)
}

fn status_line(ui: &mut Ui, r: &InstRow, pos: egui::Pos2) {
    let th = look(ui.ctx()).theme;
    let p = ui.painter().clone();
    let (text, color, left) = match (r.state, r.update) {
        (RowState::Failed, _) => (r.note.to_owned(), th.danger_strong, 0.0),
        (RowState::Unsupported, _) => (r.note.to_owned(), th.warn, 0.0),
        (RowState::Installing(f), _) => {
            kit::spinner(ui, pos + vec2(6.0, 8.0), 5.0, th.accent);
            (
                format!("Installing · {}%", (f * 100.0).round() as u32),
                th.ink4,
                16.0,
            )
        }
        (RowState::Cancelled, _) => ("Installation cancelled".to_owned(), th.ink5, 0.0),
        (_, Some(v)) => (
            format!("Update {v} · adds Network permission"),
            th.accent,
            0.0,
        ),
        _ => (r.note.to_owned(), th.ink5, 0.0),
    };
    p.text(
        pos + vec2(left, 0.0),
        egui::Align2::LEFT_TOP,
        text,
        fid(12.0, egui::FontFamily::Proportional),
        color,
    );
}

/// Right-to-left, so the first widget added is the rightmost.
fn actions_for(ui: &mut Ui, r: &mut InstRow) -> Option<Act> {
    let th = look(ui.ctx()).theme;
    let mut act = None;
    match r.state {
        RowState::Normal => {
            ui.add(Toggle::new(&mut r.enabled));
            if r.update.is_some() && kit::soft_button(ui, "Review").clicked() {
                act = Some(Act::Review);
            }
        }
        RowState::Failed => {
            if ui
                .add(Button::new("Restart").kind(ButtonKind::Primary))
                .clicked()
            {
                act = Some(Act::Restart);
            }
            if kit::link(ui, "Copy log", th.accent, 12.5).clicked() {
                act = Some(Act::CopyLog);
            }
        }
        RowState::Unsupported => {
            let mut off = false;
            ui.add(Toggle::new(&mut off).enabled(false));
            kit::link(ui, "Update OneCAD", th.accent, 12.5);
        }
        RowState::Installing(_) => {
            if kit::link(ui, "Cancel", th.ink4, 12.5).clicked() {
                act = Some(Act::Cancel);
            }
        }
        RowState::Removed => {
            ui.add(Toggle::new(&mut r.enabled));
            if kit::link(ui, "Uninstall", th.danger_strong, 12.5).clicked() {
                act = Some(Act::Uninstall);
            }
        }
        RowState::Cancelled => {
            if ui
                .add(Button::new("Install").kind(ButtonKind::Secondary))
                .clicked()
            {
                act = Some(Act::Install);
            }
        }
    }
    act
}
