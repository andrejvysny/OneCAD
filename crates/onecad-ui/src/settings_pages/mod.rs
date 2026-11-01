//! Native Settings page inside the Start screen: category column plus the General,
//! Navigation and Keyboard shortcuts pages. Changes apply immediately; nothing is saved
//! here because `settings::Store` persists the edited preferences every frame.
use crate::{
    app::App,
    ui_state::{StartPage, UiState},
};
use egui::{Rect, Sense, Stroke, Ui, UiBuilder, pos2, vec2};
use onecad_widgets::{
    components::inputs::SearchField,
    fonts,
    metrics::{CONTROL, RADIUS_SM},
    theme::look,
};

mod general;
mod navigation;
pub(crate) mod recording;
pub(crate) mod rows;
mod shortcut_cells;
mod shortcuts;
#[cfg(test)]
mod tests;

pub use recording::ShortcutsUi;

const COLUMN_W: f32 = 208.0;
const PAD_X: f32 = 40.0;
const GUTTER: f32 = 24.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Category {
    #[default]
    General,
    Viewport,
    Navigation,
    Sketch,
    Shortcuts,
    Files,
    Advanced,
}

impl Category {
    fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Viewport => "Viewport",
            Self::Navigation => "Navigation",
            Self::Sketch => "Sketch",
            Self::Shortcuts => "Keyboard shortcuts",
            Self::Files => "Files & recovery",
            Self::Advanced => "Advanced",
        }
    }

    fn available(self) -> bool {
        matches!(self, Self::General | Self::Navigation | Self::Shortcuts)
    }

    /// Row labels and terms a search should find on the category's page.
    fn keywords(self) -> &'static [&'static str] {
        match self {
            Self::General => &[
                "theme",
                "appearance",
                "light",
                "dark",
                "match system",
                "interface density",
                "units",
                "length",
                "angle",
                "displayed decimals",
                "startup",
                "on launch",
                "check for updates",
            ],
            Self::Navigation => &[
                "preset",
                "mouse",
                "trackpad",
                "orbit",
                "pan",
                "zoom",
                "fit all",
                "camera",
                "orbit around",
                "zoom toward cursor",
                "invert scroll zoom",
                "view transitions",
            ],
            Self::Shortcuts => &["commands", "keys", "hotkeys", "reset all", "recording"],
            _ => &[],
        }
    }

    /// Case-insensitive substring match on the label or a row keyword.
    fn matches(self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        query.is_empty()
            || self.label().to_lowercase().contains(&query)
            || self.keywords().iter().any(|word| word.contains(&query))
    }
}

/// Page-local state; not persisted.
#[derive(Clone, Debug, Default)]
pub struct SettingsUi {
    pub category: Category,
    pub search: String,
    pub shortcuts: ShortcutsUi,
}

pub fn show(app: &mut App, ui: &mut Ui) {
    let area = ui.max_rect();
    let column = Rect::from_min_size(
        area.min,
        vec2(
            column_width(ui.ctx().content_rect().width()).min(area.width() * 0.4),
            area.height(),
        ),
    );
    category_column(&mut app.ui, ui, column);
    let inner = Rect::from_min_max(
        pos2(column.right() + PAD_X, area.top() + 28.0),
        pos2(area.right() - PAD_X, area.bottom() - 24.0),
    );
    let appearance = scroll_content(ui, inner, &mut app.ui).0;
    if let Some(choice) = appearance {
        let ctx = ui.ctx().clone();
        crate::input::set_appearance(app, &ctx, choice);
    }
}

/// Scrolling content column ending at `inner.right()`; the bar sits in the gutter
/// beyond it. Returns the laid-out content rect for overflow checks.
fn scroll_content(
    ui: &mut Ui,
    inner: Rect,
    state: &mut UiState,
) -> (Option<crate::ui_state::Appearance>, Rect) {
    let scroll = Rect::from_min_max(inner.min, pos2(inner.right() + GUTTER, inner.bottom()));
    let mut out = (None, Rect::NOTHING);
    ui.scope_builder(UiBuilder::new().max_rect(scroll), |ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
        egui::ScrollArea::vertical()
            .id_salt("settings-content")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_max_width(inner.width());
                out.0 = content(ui, state);
                out.1 = ui.min_rect();
            });
    });
    out
}

/// Whether a shortcut recording may continue; pure so the rules are testable.
fn recording_allowed(state: &UiState, text_focus: bool, popup_open: bool) -> bool {
    state.start_open
        && state.start_page == StartPage::Settings
        && state.settings.category == Category::Shortcuts
        && !text_focus
        && !popup_open
        && !state.extensions.native.palette_open
        && shortcuts::recording_visible(&state.settings.shortcuts, &state.shortcut_overrides)
}

/// Once per frame: drops a recording whose page, focus or row is gone, so a key can
/// never be captured invisibly.
pub(crate) fn sync_recording(app: &mut App, ctx: &egui::Context, raw: &egui::RawInput) {
    if !app.ui.settings.shortcuts.capturing() {
        return;
    }
    // Chords never reach the page: the app consumes them first. The row must also
    // have been drawn on screen since the last sync.
    let on_screen = recording::row_on_screen(app.ui.settings.shortcuts.row.take());
    let allowed = recording_allowed(
        &app.ui,
        ctx.text_edit_focused(),
        egui::Popup::is_any_open(ctx),
    );
    if recording::chord_pressed(&raw.events) || !on_screen || !allowed {
        app.ui.settings.shortcuts.cancel();
    }
}

/// Category column width: proportional to the host, within the design's 150..208.
fn column_width(host_width: f32) -> f32 {
    (host_width * 0.22).clamp(150.0, COLUMN_W)
}

/// Draws the selected category; returns an appearance the caller must apply.
fn content(ui: &mut Ui, state: &mut UiState) -> Option<crate::ui_state::Appearance> {
    match state.settings.category {
        Category::Navigation => navigation::show(ui, &mut state.navigation),
        Category::Shortcuts => shortcuts::show(
            ui,
            &mut state.settings.shortcuts,
            &mut state.shortcut_overrides,
        ),
        _ => return general::show(ui, state.appearance, &mut state.display_decimals),
    }
    None
}

enum Item {
    Page(Category),
    Divider,
    Extensions,
}

const ITEMS: [Item; 9] = [
    Item::Page(Category::General),
    Item::Page(Category::Viewport),
    Item::Page(Category::Navigation),
    Item::Page(Category::Sketch),
    Item::Page(Category::Shortcuts),
    Item::Page(Category::Files),
    Item::Divider,
    Item::Extensions,
    Item::Page(Category::Advanced),
];

fn category_column(state: &mut UiState, ui: &mut Ui, column: Rect) {
    let t = look(ui.ctx()).theme;
    ui.painter().vline(
        column.right() - 0.5,
        column.y_range(),
        Stroke::new(1.0, t.border),
    );
    let inner = Rect::from_min_max(
        column.min + vec2(10.0, 16.0),
        pos2(column.right() - 11.0, column.bottom()),
    );
    ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        let search = &mut state.settings.search;
        ui.add(
            SearchField::new("settings-search", search)
                .placeholder("Search settings")
                .width(inner.width()),
        );
        ui.add_space(12.0);
        for item in ITEMS {
            match item {
                Item::Page(category) => page_item(state, ui, category),
                Item::Divider => divider(ui),
                Item::Extensions => extensions_item(state, ui),
            }
        }
    });
}

fn divider(ui: &mut Ui) {
    let t = look(ui.ctx()).theme;
    ui.add_space(8.0);
    let y = ui.cursor().top();
    let (x0, x1) = (ui.max_rect().left() + 4.0, ui.max_rect().right() - 4.0);
    ui.painter()
        .hline(x0..=x1, y, Stroke::new(1.0, t.border_subtle));
    ui.add_space(9.0);
}

fn page_item(state: &mut UiState, ui: &mut Ui, category: Category) {
    let settings = &mut state.settings;
    let dimmed = !settings.search.is_empty() && !category.matches(&settings.search);
    let enabled = category.available() && !dimmed;
    let response = item_row(
        ui,
        category.label(),
        settings.category == category,
        enabled,
        "",
    );
    if !category.available() {
        rows::reason(response, category.label(), "Not available yet");
    } else if enabled && response.clicked() && settings.category != category {
        settings.category = category;
    }
}

fn extensions_item(state: &mut UiState, ui: &mut Ui) {
    if item_row(ui, "Extension settings", false, true, "↗").clicked() {
        state.start_page = StartPage::Extensions;
    }
}

fn item_row(
    ui: &mut Ui,
    label: &str,
    selected: bool,
    enabled: bool,
    suffix: &str,
) -> egui::Response {
    let t = look(ui.ctx()).theme;
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), rows::interactive_height()),
        sense,
    );
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, RADIUS_SM, t.well);
    } else if response.hovered() && enabled {
        p.rect_filled(rect, RADIUS_SM, t.hover);
    }
    let (family, ink) = match (selected, enabled) {
        (true, _) => (fonts::medium(), t.ink),
        (false, true) => (egui::FontFamily::Proportional, t.ink2),
        (false, false) => (egui::FontFamily::Proportional, t.ink2.gamma_multiply(0.4)),
    };
    let font = egui::FontId::new(CONTROL + 0.5, family);
    p.text(
        pos2(rect.left() + 10.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        font,
        ink,
    );
    if !suffix.is_empty() {
        p.text(
            pos2(rect.right() - 10.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            suffix,
            egui::FontId::new(CONTROL, egui::FontFamily::Proportional),
            t.ink6,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::RadioButton, enabled, selected, label)
    });
    response
}
