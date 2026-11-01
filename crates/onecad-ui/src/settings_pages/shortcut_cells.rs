//! Painted cells and rows of the shortcut table: key caps, the recording box and the
//! conflict warning. State changes are returned as [`Cmd`], never applied here.
use super::recording::{Pending, ShortcutsUi, ToolRow};
use super::rows;
use crate::{
    shortcut_prefs::{KeyContext, ShortcutOverride},
    tool_registry::{self, ToolId},
};
use egui::{
    Align, Align2, FontId, Layout, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder,
    pos2, vec2,
};
use onecad_widgets::{
    components::{
        button::{Button, ButtonKind},
        tooltip::{self, Tip},
    },
    fonts,
    icon::{self, IconStyle},
    metrics::{CONTROL, MONO_VALUE, RADIUS_SM, SECONDARY, focus_ring},
    theme::{Theme, look},
};

/// Menu shortcuts shown read-only: the menu owns them.
pub(super) struct Fixed {
    pub(super) label: &'static str,
    pub(super) icon: &'static str,
    pub(super) keys: &'static [&'static str],
}

pub(super) const FIXED: [Fixed; 5] = [
    Fixed {
        label: "Fit all",
        icon: "fit",
        keys: &["⌘", "0"],
    },
    Fixed {
        label: "Home view",
        icon: "viewIso",
        keys: &["⌘", "⇧", "H"],
    },
    Fixed {
        label: "Show or hide Explorer",
        icon: "layers",
        keys: &["⌘", "1"],
    },
    Fixed {
        label: "Show or hide Inspector",
        icon: "panelRight",
        keys: &["⌘", "2"],
    },
    Fixed {
        label: "Settings",
        icon: "settings",
        keys: &["⌘", ","],
    },
];

const CONTEXT_W: f32 = 140.0;
const SHORTCUT_W: f32 = 220.0;
pub(super) const PAD: f32 = 12.0;

pub(super) enum Cmd {
    Start(KeyContext, ToolId),
    Cancel,
    Reset(KeyContext, ToolId),
    Confirm,
    ResetAll,
}

pub(super) fn row_height() -> f32 {
    if rows::touch() { 44.0 } else { 38.0 }
}

pub(super) fn columns(rect: Rect) -> (f32, f32) {
    let (context, shortcut) = if rows::very_narrow(rect.width()) {
        (0.0, 110.0)
    } else if rows::narrow(rect.width()) {
        (96.0, 150.0)
    } else {
        (
            CONTEXT_W.min(rect.width() * 0.25),
            SHORTCUT_W.min(rect.width() * 0.4),
        )
    };
    let shortcut_x = rect.right() - PAD - shortcut;
    (shortcut_x - context, shortcut_x)
}

pub(super) fn context_label(context: KeyContext) -> &'static str {
    if context.sketch() {
        "Sketch"
    } else {
        "Modeling"
    }
}

pub(super) fn label_of(tool: ToolId) -> String {
    tool_registry::definition(tool).map_or_else(|| format!("{tool:?}"), |d| d.label.to_owned())
}

/// Command cell: icon, label (elided before `limit_x`), an optional muted caption
/// under it and the changed dot.
pub(super) fn command_cell(
    ui: &Ui,
    rect: Rect,
    icon_name: &str,
    (label, caption): (&str, Option<&str>),
    (changed, limit_x): (bool, f32),
) {
    let l = look(ui.ctx());
    let t = &l.theme;
    let p = ui.painter();
    let icon_rect = Rect::from_center_size(
        pos2(rect.left() + PAD + 9.0, rect.center().y),
        vec2(18.0, 18.0),
    );
    let style = IconStyle {
        tint: t.ink3,
        bg: t.surface,
        grey: false,
    };
    icon::paint(p, icon_rect, icon_name, l.dark, style, 1.0);
    let left = rect.left() + PAD + 28.0;
    let mut job = egui::text::LayoutJob::simple_singleline(
        label.to_owned(),
        FontId::new(CONTROL, egui::FontFamily::Proportional),
        t.ink,
    );
    job.wrap.max_width = (limit_x - left - 8.0 - if changed { 14.0 } else { 0.0 }).max(20.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = p.layout_job(job);
    let shift = if caption.is_some() { 8.0 } else { 0.0 };
    let at = pos2(left, rect.center().y - galley.size().y / 2.0 - shift);
    let end = at.x + galley.size().x;
    p.galley(at, galley, t.ink);
    if let Some(caption) = caption {
        p.text(
            pos2(left, rect.center().y + 8.0),
            Align2::LEFT_CENTER,
            caption,
            FontId::new(11.0, egui::FontFamily::Proportional),
            t.ink5,
        );
    }
    if changed {
        p.circle_filled(pos2(end + 13.0, rect.center().y - shift), 3.5, t.accent);
    }
}

pub(super) fn tool_row(
    ui: &mut Ui,
    row: ToolRow,
    state: &ShortcutsUi,
    overrides: &[ShortcutOverride],
) -> Option<Cmd> {
    let t = look(ui.ctx()).theme;
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), row_height()), Sense::hover());
    let recording = state.recording == Some((row.context, row.tool));
    let p = ui.painter();
    if recording {
        p.rect_filled(rect, 0.0, t.sel_soft);
    }
    let changed = row.changed(overrides);
    let def = tool_registry::definition(row.tool);
    let (context_x, shortcut_x) = columns(rect);
    let compact = rows::very_narrow(rect.width());
    let context = context_label(row.context);
    command_cell(
        ui,
        rect,
        def.map_or("select", |d| d.icon),
        (&label_of(row.tool), compact.then_some(context)),
        (changed, if compact { shortcut_x } else { context_x }),
    );
    if !compact {
        ui.painter().text(
            pos2(context_x, rect.center().y),
            Align2::LEFT_CENTER,
            context,
            FontId::new(CONTROL, egui::FontFamily::Proportional),
            t.ink3,
        );
    }
    let at = pos2(shortcut_x, rect.center().y);
    let cmd = if recording {
        recording_box(ui, at, state.pending, rect.right() - PAD - at.x)
    } else {
        key_cell(ui, at, row, changed, overrides)
    };
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, t.border_subtle),
    );
    if recording || matches!(cmd, Some(Cmd::Start(..))) {
        state.row.set(Some((rect, ui.clip_rect())));
    }
    cmd
}

fn key_cell(
    ui: &mut Ui,
    at: Pos2,
    row: ToolRow,
    changed: bool,
    overrides: &[ShortcutOverride],
) -> Option<Cmd> {
    let t = look(ui.ctx()).theme;
    let key = row.key(overrides);
    let text = key.map_or("None", |k| k.name());
    let chip = paint_chip(ui.painter(), &t, at - vec2(0.0, 11.0), text, key.is_none());
    let hit_h = (chip.height() + 10.0).max(rows::hit());
    let hit = Rect::from_center_size(chip.center(), vec2(chip.width() + 8.0, hit_h));
    let response = ui.interact(
        hit,
        egui::Id::unique(("key", row.context.sketch(), row.tool as u8)),
        Sense::click(),
    );
    let name = format!(
        "{} shortcut, {}: {text}",
        label_of(row.tool),
        context_label(row.context)
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
    let response = tooltip::attach(response, Tip::new("Click to change this shortcut"));
    let mut cmd = response
        .clicked()
        .then_some(Cmd::Start(row.context, row.tool));
    if changed {
        let size = vec2(40.0, 22.0_f32.max(rows::hit()));
        let reset = Rect::from_center_size(pos2(chip.right() + 28.0, at.y), size);
        let r = ui.interact(
            reset,
            egui::Id::unique(("reset", row.context.sketch(), row.tool as u8)),
            Sense::click(),
        );
        let name = format!("Reset {} shortcut", label_of(row.tool));
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
        ui.painter().text(
            reset.left_center() + vec2(4.0, 0.0),
            Align2::LEFT_CENTER,
            "Reset",
            FontId::new(SECONDARY, egui::FontFamily::Proportional),
            if r.hovered() {
                t.accent_hover
            } else {
                t.accent
            },
        );
        if r.clicked() {
            cmd = Some(Cmd::Reset(row.context, row.tool));
        }
    }
    cmd
}

fn recording_box(ui: &mut Ui, at: Pos2, pending: Option<Pending>, room: f32) -> Option<Cmd> {
    let t = look(ui.ctx()).theme;
    let rect = Rect::from_min_size(at - vec2(0.0, 14.0), vec2(180.0_f32.min(room), 28.0));
    let hit = Rect::from_center_size(rect.center(), vec2(rect.width(), 28.0_f32.max(rows::hit())));
    let response = ui.interact(hit, egui::Id::unique("recording"), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Recording"));
    let p = ui.painter();
    p.rect_filled(rect, RADIUS_SM, t.surface);
    p.rect_stroke(
        rect,
        RADIUS_SM,
        Stroke::new(1.0, t.accent),
        StrokeKind::Inside,
    );
    let (w, ring) = focus_ring(&t);
    p.rect_stroke(
        rect.expand(w * 0.5),
        RADIUS_SM + 2,
        Stroke::new(w, ring),
        StrokeKind::Outside,
    );
    let mut x = rect.left() + 8.0;
    if let Some(pending) = pending.filter(|_| room >= 150.0) {
        x = paint_chip(
            p,
            &t,
            pos2(x, rect.center().y - 10.0),
            pending.key.name(),
            false,
        )
        .right()
            + 8.0;
    }
    p.text(
        pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        "Recording…",
        FontId::new(SECONDARY, egui::FontFamily::Proportional),
        t.ink5,
    );
    let close =
        Rect::from_center_size(pos2(rect.right() - 14.0, rect.center().y), vec2(12.0, 12.0));
    let style = IconStyle {
        tint: t.ink3,
        bg: t.surface,
        grey: false,
    };
    icon::paint(p, close, "close", look(ui.ctx()).dark, style, 1.0);
    response.clicked().then_some(Cmd::Cancel)
}

/// Mono key cap with a heavier bottom edge; returns its rect.
fn paint_chip(p: &Painter, t: &Theme, top_left: Pos2, text: &str, muted: bool) -> Rect {
    let ink = if muted { t.ink5 } else { t.ink2 };
    let galley = p.layout_no_wrap(
        text.to_owned(),
        FontId::new(MONO_VALUE - 0.5, fonts::mono()),
        ink,
    );
    let size = vec2((galley.size().x + 12.0).max(22.0), 22.0);
    let rect = Rect::from_min_size(top_left, size);
    p.rect_stroke(
        rect,
        5,
        Stroke::new(1.0, t.border_strong),
        StrokeKind::Inside,
    );
    let y = rect.bottom() - 1.0;
    p.hline(
        rect.left() + 3.0..=rect.right() - 3.0,
        y,
        Stroke::new(1.0, t.border_strong),
    );
    p.galley(
        rect.center() - galley.size() / 2.0 - vec2(0.0, 1.0),
        galley,
        ink,
    );
    rect
}

pub(super) fn fixed_row(ui: &mut Ui, fixed: &Fixed) {
    let t = look(ui.ctx()).theme;
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), row_height()), Sense::hover());
    let (context_x, shortcut_x) = columns(rect);
    let compact = rows::very_narrow(rect.width());
    command_cell(
        ui,
        rect,
        fixed.icon,
        (fixed.label, compact.then_some("View")),
        (false, if compact { shortcut_x } else { context_x }),
    );
    if !compact {
        ui.painter().text(
            pos2(context_x, rect.center().y),
            Align2::LEFT_CENTER,
            "View",
            FontId::new(CONTROL, egui::FontFamily::Proportional),
            t.ink3,
        );
    }
    let mut x = shortcut_x;
    let top = rect.center().y - 11.0;
    for key in fixed.keys {
        x = paint_chip(ui.painter(), &t, pos2(x, top), key, false).right() + 4.0;
    }
    let hit = Rect::from_min_max(pos2(shortcut_x, top), pos2(x, top + 22.0));
    let response = ui.interact(
        hit,
        egui::Id::unique(("fixed", fixed.label)),
        Sense::hover(),
    );
    tooltip::attach(response, Tip::new("Menu shortcuts cannot be changed yet"));
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, t.border_subtle),
    );
}

pub(super) fn warning_row(ui: &mut Ui, pending: Pending) -> Option<Cmd> {
    let t = look(ui.ctx()).theme;
    let width = ui.available_width();
    // Narrow columns put the buttons under the text instead of beside it.
    let stacked = rows::narrow(width);
    let buttons_w = if stacked { 0.0 } else { 200.0 };
    let text_w = (width - 40.0 - PAD - 20.0 - 22.0 - buttons_w).max(60.0);
    let mut job = warning_text(&t, pending);
    job.wrap.max_width = text_w;
    let galley = ui.painter().layout_job(job);
    let body = galley.size().y.max(30.0);
    let height = body + 32.0 + if stacked { 38.0 } else { 0.0 };
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 0.0, t.sel_soft);
    let inner = Rect::from_min_max(
        pos2(rect.left() + 40.0, rect.top() + 6.0),
        pos2(rect.right() - PAD, rect.bottom() - 6.0),
    );
    p.rect_filled(inner, 7, t.warn_surface);
    p.rect_stroke(
        inner,
        7,
        Stroke::new(1.0, t.warn_border),
        StrokeKind::Inside,
    );
    let text_pos = pos2(inner.left() + 32.0, inner.top() + 10.0);
    let icon_rect = Rect::from_min_size(pos2(inner.left() + 10.0, text_pos.y), vec2(14.0, 14.0));
    p.galley(text_pos, galley, t.warn);
    let style = IconStyle {
        tint: t.warn,
        bg: t.warn_surface,
        grey: false,
    };
    icon::paint(p, icon_rect, "warning", look(ui.ctx()).dark, style, 1.0);
    let area = if stacked {
        Rect::from_min_max(
            pos2(inner.left() + 10.0, inner.bottom() - 36.0),
            pos2(inner.right() - 10.0, inner.bottom() - 6.0),
        )
    } else {
        Rect::from_min_max(
            pos2(inner.right() - 10.0 - buttons_w, inner.top()),
            pos2(inner.right() - 10.0, inner.bottom()),
        )
    };
    warning_buttons(ui, area, pending)
}

fn warning_buttons(ui: &mut Ui, area: Rect, pending: Pending) -> Option<Cmd> {
    let mut cmd = None;
    let child = UiBuilder::new()
        .max_rect(area)
        .layout(Layout::right_to_left(Align::Center));
    ui.scope_builder(child, |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let use_label = format!("Use for {}", label_of(pending.tool));
        if ui
            .add(
                Button::new(&use_label)
                    .kind(ButtonKind::Primary)
                    .hit_height(rows::hit()),
            )
            .clicked()
        {
            cmd = Some(Cmd::Confirm);
        }
        if ui
            .add(
                Button::new("Cancel")
                    .kind(ButtonKind::Ghost)
                    .hit_height(rows::hit()),
            )
            .clicked()
        {
            cmd = Some(Cmd::Cancel);
        }
    });
    cmd
}

fn warning_text(t: &Theme, pending: Pending) -> egui::text::LayoutJob {
    let loser = label_of(pending.loser);
    let context = context_label(pending.context);
    let plain = egui::TextFormat {
        font_id: FontId::new(SECONDARY, egui::FontFamily::Proportional),
        color: t.warn,
        ..Default::default()
    };
    let bold = egui::TextFormat {
        font_id: FontId::new(SECONDARY, fonts::semibold()),
        ..plain.clone()
    };
    let mut job = egui::text::LayoutJob::default();
    job.append(
        &format!("{} already runs ", pending.key.name()),
        0.0,
        plain.clone(),
    );
    job.append(&loser, 0.0, bold);
    job.append(
        &format!(" in {context}. {loser} will have no shortcut."),
        0.0,
        plain,
    );
    job
}
