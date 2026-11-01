//! Keyboard shortcuts (design 24c): remappable tool keys per context plus read-only
//! menu shortcuts. The recording rules live in `recording`.
use super::recording::{Filter, ShortcutsUi, ToolRow, tool_rows};
use super::rows;
use super::shortcut_cells::{
    Cmd, FIXED, Fixed, PAD, columns, context_label, fixed_row, label_of, tool_row, warning_row,
};
use crate::shortcut_prefs::{self, KeyContext, ShortcutOverride};
use egui::{Align, Align2, FontId, Layout, RichText, Sense, Stroke, Ui, pos2, vec2};
use onecad_widgets::{
    components::{button::Button, inputs::SearchField, tooltip::Tip},
    fonts,
    metrics::{SECONDARY, SECTION},
    theme::look,
};

pub fn show(ui: &mut Ui, state: &mut ShortcutsUi, overrides: &mut Vec<ShortcutOverride>) {
    if state.capturing() && !ui.ctx().text_edit_focused() {
        let events = ui.ctx().input(|input| input.events.clone());
        state.keys(overrides, &events);
    }
    let mut cmd = header(ui, shortcut_prefs::changed_count(overrides));
    toolbar(ui, state, overrides);
    ui.add_space(20.0);
    cmd = table(ui, state, overrides).or(cmd);
    footer(ui);
    if let Some(cmd) = cmd {
        apply(ui, state, overrides, cmd);
    }
}

fn apply(ui: &mut Ui, state: &mut ShortcutsUi, overrides: &mut Vec<ShortcutOverride>, cmd: Cmd) {
    match cmd {
        Cmd::Start(context, tool) => {
            // The search field must not keep eating the keys being recorded.
            ui.memory_mut(|memory| {
                if let Some(id) = memory.focused() {
                    memory.surrender_focus(id);
                }
            });
            state.start(context, tool);
        }
        Cmd::Cancel => state.cancel(),
        Cmd::Reset(context, tool) => {
            shortcut_prefs::reset(overrides, context, tool);
            state.cancel();
        }
        Cmd::Confirm => state.confirm(overrides),
        Cmd::ResetAll => {
            shortcut_prefs::reset_all(overrides);
            state.cancel();
        }
    }
}

fn header(ui: &mut Ui, changed: usize) -> Option<Cmd> {
    let mut cmd = None;
    ui.horizontal(|ui| {
        let button_w = 96.0;
        ui.allocate_ui(vec2((ui.available_width() - button_w).max(0.0), 0.0), |ui| {
            ui.vertical(|ui| {
                rows::title_block(
                    ui,
                    "Keyboard shortcuts",
                    "Click a shortcut to change it. A key can mean different things in Modeling and Sketch.",
                );
            });
        });
        ui.with_layout(Layout::right_to_left(Align::Max), |ui| {
            let mut reset = Button::new("Reset all")
                .enabled(changed > 0)
                .hit_height(rows::hit());
            if changed == 0 {
                let tip = Tip::new("Reset all").reason(Some("No shortcuts changed"));
                reset = reset.tip(tip);
            }
            if ui.add(reset).clicked() {
                cmd = Some(Cmd::ResetAll);
            }
        });
    });
    ui.add_space(20.0);
    cmd
}

fn toolbar(ui: &mut Ui, state: &mut ShortcutsUi, overrides: &[ShortcutOverride]) {
    let stacked = rows::narrow(ui.available_width());
    let layout = if stacked {
        Layout::top_down(Align::Min)
    } else {
        Layout::left_to_right(Align::Center)
    };
    ui.with_layout(layout, |ui| {
        ui.spacing_mut().item_spacing = vec2(12.0, 8.0);
        let search_w = 260.0_f32.min(ui.available_width());
        ui.add(
            SearchField::new("shortcuts-search", &mut state.search)
                .placeholder("Search commands or keys")
                .width(search_w),
        );
        let changed = shortcut_prefs::changed_count(overrides);
        let changed_label = if changed > 0 {
            format!("Changed {changed}")
        } else {
            "Changed".to_owned()
        };
        let labels = ["All", "Modeling", "Sketch", "View", changed_label.as_str()];
        let current = Filter::ALL.iter().position(|f| *f == state.filter);
        if let Some(index) = rows::segmented(ui, "shortcut-filter", current.unwrap_or(0), &labels) {
            state.filter = Filter::ALL[index];
        }
    });
}

fn table(ui: &mut Ui, state: &ShortcutsUi, overrides: &[ShortcutOverride]) -> Option<Cmd> {
    let mut cmd = None;
    table_header(ui);
    let query = state.search.trim().to_lowercase();
    for row in tool_rows() {
        if !tool_visible(row, state.filter, &query, overrides) {
            continue;
        }
        cmd = tool_row(ui, row, state, overrides).or(cmd);
        if let Some(pending) = state
            .pending
            .filter(|p| p.tool == row.tool && p.context == row.context)
        {
            cmd = warning_row(ui, pending).or(cmd);
        }
    }
    for fixed in FIXED {
        if fixed_visible(&fixed, state.filter, &query) {
            fixed_row(ui, &fixed);
        }
    }
    cmd
}

/// Whether the row being recorded is still listed under the search and filter.
pub(super) fn recording_visible(state: &ShortcutsUi, overrides: &[ShortcutOverride]) -> bool {
    let Some((context, tool)) = state.recording else {
        return false;
    };
    let query = state.search.trim().to_lowercase();
    tool_visible(ToolRow { context, tool }, state.filter, &query, overrides)
}

/// Whether a tool row passes the filter chip and the search text (already lowercase).
pub(super) fn tool_visible(
    row: ToolRow,
    filter: Filter,
    query: &str,
    overrides: &[ShortcutOverride],
) -> bool {
    let in_filter = match filter {
        Filter::All => true,
        Filter::Modeling => row.context == KeyContext::Modeling,
        Filter::Sketch => row.context == KeyContext::Sketch,
        Filter::View => false,
        Filter::Changed => row.changed(overrides),
    };
    let key = row.key(overrides).map_or("none", |k| k.name());
    in_filter
        && matches_query(
            query,
            &[&label_of(row.tool), context_label(row.context), key],
        )
}

fn fixed_visible(fixed: &Fixed, filter: Filter, query: &str) -> bool {
    let keys = fixed.keys.concat();
    matches!(filter, Filter::All | Filter::View)
        && matches_query(query, &[fixed.label, "View", keys.as_str()])
}

fn matches_query(query: &str, haystack: &[&str]) -> bool {
    query.is_empty()
        || haystack
            .iter()
            .any(|text| text.to_lowercase().contains(query))
}

fn table_header(ui: &mut Ui) {
    let t = look(ui.ctx()).theme;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    let (context_x, shortcut_x) = columns(rect);
    let compact = rows::very_narrow(rect.width());
    for (x, text) in [
        (rect.left() + PAD, "COMMAND"),
        (context_x, "CONTEXT"),
        (shortcut_x, "SHORTCUT"),
    ]
    .into_iter()
    .filter(|(_, text)| !(compact && *text == "CONTEXT"))
    {
        ui.painter().text(
            pos2(x, rect.center().y),
            Align2::LEFT_CENTER,
            text,
            FontId::new(SECTION, fonts::medium()),
            t.ink5,
        );
    }
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, t.border),
    );
}

fn footer(ui: &mut Ui) {
    let t = look(ui.ctx()).theme;
    ui.add_space(20.0);
    let y = ui.cursor().top();
    ui.painter().hline(
        ui.max_rect().x_range(),
        y,
        Stroke::new(1.0, t.border_subtle),
    );
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        let (dot, _) = ui.allocate_exact_size(vec2(7.0, 7.0), Sense::hover());
        ui.painter().circle_filled(dot.center(), 3.5, t.accent);
        let text = RichText::new(footer_text()).size(SECONDARY).color(t.ink5);
        ui.add(egui::Label::new(text).wrap());
    });
}

fn footer_text() -> String {
    let mut text = format!(
        "Changed from default. Shortcuts are stored on {}.",
        rows::device()
    );
    if rows::touch() {
        text.push_str(" Use them with a hardware keyboard.");
    }
    text
}
