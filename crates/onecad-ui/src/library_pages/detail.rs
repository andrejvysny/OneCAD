//! Component detail: package preview on the left, metadata and parameter tabs on the right.
use super::{
    browser::{free_count, icon_for, meta_line, source_label},
    kit::{self, ChipKind, Thumb},
    params,
};
use crate::{
    library_catalog::{Catalog, Status},
    ui_state::{StartPage, UiState},
};
use egui::{Align, CornerRadius, Layout, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use onecad_app::dto::LibraryComponentDto;
use onecad_widgets::{
    components::{
        button::{Button, ButtonKind},
        lists::{Tab, TabBar},
    },
    fonts,
    theme::look,
};

const ISO4762: &str = "iso4762";

/// What a click on the page asks for, applied after drawing.
enum Go {
    Library,
    Version(String),
    StandardTable,
    Insert,
}

pub fn show(state: &mut UiState, ui: &mut Ui) {
    let lib = &mut state.library;
    let blocker = super::insert_blocker(lib);
    let Some(view) = lib.catalog.detail() else {
        state.start_page = StartPage::Library;
        return;
    };
    let name = view
        .component
        .as_ref()
        .map_or(view.id.as_str(), |c| c.name.as_str());
    let mut go = None;
    if kit::breadcrumb(ui, &["Component library", name]) == Some(0) {
        go = Some(Go::Library);
    }
    ui.add_space(18.0);
    let left_w = (ui.available_width() * 0.4).clamp(280.0, 400.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 28.0;
        ui.allocate_ui_with_layout(vec2(left_w, 400.0), Layout::top_down(Align::Min), |ui| {
            preview(&lib.catalog, ui);
        });
        let w = ui.available_width();
        ui.allocate_ui_with_layout(vec2(w, 400.0), Layout::top_down(Align::Min), |ui| {
            go = right_column(&mut lib.catalog, &mut lib.detail_tab, blocker, ui).or(go.take());
        });
    });
    match go {
        Some(Go::Library) => {
            lib.catalog.close_detail();
            super::go_library(state);
        }
        Some(Go::Version(version)) => {
            let id = lib
                .catalog
                .detail()
                .map(|d| d.id.clone())
                .unwrap_or_default();
            lib.catalog.open_detail(&id, &version);
        }
        Some(Go::StandardTable) => state.start_page = StartPage::StandardPart,
        Some(Go::Insert) => lib.insert_requested = true,
        None => {}
    }
}

fn preview(catalog: &Catalog, ui: &mut Ui) {
    let l = look(ui.ctx());
    let t = &l.theme;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 320.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(12), t.well);
    p.rect_stroke(
        rect,
        CornerRadius::same(12),
        Stroke::new(1.0, t.border),
        StrokeKind::Inside,
    );
    let Some(view) = catalog.detail() else { return };
    let icon = view.component.as_ref().map_or("body", icon_for);
    let thumb = catalog
        .texture(&view.id, &view.version)
        .map(|(id, [w, h])| Thumb {
            id,
            size: vec2(w as f32, h as f32),
        });
    kit::paint_thumbnail(p, rect, icon, thumb, t);
    if thumb.is_none() {
        let note = "No preview image in this package.";
        let at = pos2(rect.center().x, rect.bottom() - 18.0);
        p.text(
            at,
            egui::Align2::CENTER_CENTER,
            note,
            egui::FontId::proportional(11.5),
            t.ink5,
        );
    }
}

fn right_column(
    catalog: &mut Catalog,
    tab: &mut usize,
    blocker: Option<&'static str>,
    ui: &mut Ui,
) -> Option<Go> {
    let mut go = None;
    let view = catalog.detail()?;
    match (&view.component, &view.status) {
        (Some(component), _) => {
            let component = component.clone();
            go = summary(&component, blocker, ui);
            ui.add_space(22.0);
            let versions = view.versions.clone();
            let failure = match &view.status {
                Status::Failed(error) => Some(error.clone()),
                _ => None,
            };
            if let Some(error) = &failure {
                let warn = look(ui.ctx()).theme.warn;
                let text = format!("Could not load full details: {error}");
                kit::label(ui, &text, 12.0, egui::FontFamily::Proportional, warn);
                ui.add_space(8.0);
            }
            go = tabs(
                catalog,
                &component,
                (&versions, failure.as_deref()),
                tab,
                ui,
            )
            .or(go);
        }
        (None, Status::Failed(error)) => {
            kit::title(ui, "This component could not be loaded.", 14.0);
            ui.add_space(4.0);
            kit::muted(ui, error, 12.5);
        }
        (None, _) => {
            kit::muted(ui, "Loading component…", 13.0);
        }
    }
    go
}

fn summary(c: &LibraryComponentDto, blocker: Option<&'static str>, ui: &mut Ui) -> Option<Go> {
    kit::title(ui, &c.name, 22.0);
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let free = free_count(c);
        if free > 0 {
            kit::chip(ui, &format!("fx {free}"), ChipKind::Accent);
        }
        kit::chip(ui, &format!("v{}", c.version), ChipKind::Plain);
        kit::chip(ui, source_label(&c.source_kind), ChipKind::Plain);
        for category in &c.category {
            kit::chip(ui, category, ChipKind::Plain);
        }
    });
    ui.add_space(10.0);
    kit::muted(ui, &meta_line(c), 12.5);
    ui.add_space(14.0);
    actions(c, blocker, ui)
}

/// Insert starts a real placement; authoring is a later step, and each off control says why.
fn actions(c: &LibraryComponentDto, blocker: Option<&'static str>, ui: &mut Ui) -> Option<Go> {
    let mut go = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let insert = Button::new("Insert into open model")
            .kind(ButtonKind::Primary)
            .icon(Some("plus"))
            .enabled(blocker.is_none());
        let response = ui.add(insert);
        if let Some(reason) = blocker {
            response.on_disabled_hover_text(reason);
        } else if response.clicked() {
            go = Some(Go::Insert);
        }
        let edit = Button::new("Edit source").icon(Some("edit")).enabled(false);
        ui.add(edit)
            .on_disabled_hover_text("Editing component sources is not available yet");
        let dup = Button::new("Duplicate")
            .kind(ButtonKind::Ghost)
            .icon(Some("duplicate"))
            .enabled(false);
        ui.add(dup)
            .on_disabled_hover_text("Duplicating components is not available yet");
        if c.generator_id.as_deref() == Some(ISO4762)
            && ui.add(Button::new("Standard table")).clicked()
        {
            go = Some(Go::StandardTable);
        }
    });
    go
}

fn tabs(
    catalog: &mut Catalog,
    c: &LibraryComponentDto,
    (versions, failure): (&[String], Option<&str>),
    tab: &mut usize,
    ui: &mut Ui,
) -> Option<Go> {
    let list = [
        Tab {
            label: "Parameters",
            count: Some(c.parameters.len()),
        },
        Tab {
            label: "Versions",
            count: Some(versions.len()),
        },
        Tab {
            label: "Attachments",
            count: Some(c.attachments.len()),
        },
        Tab {
            label: "Details",
            count: None,
        },
    ];
    ui.add(TabBar::new("detail-tabs", tab, &list));
    ui.add_space(4.0);
    match *tab {
        0 => {
            params::show(catalog, ui);
            None
        }
        1 => version_rows(c, versions, failure, ui),
        2 => {
            attachment_rows(c, ui);
            None
        }
        _ => {
            detail_rows(c, ui);
            None
        }
    }
}

fn version_rows(
    c: &LibraryComponentDto,
    versions: &[String],
    failure: Option<&str>,
    ui: &mut Ui,
) -> Option<Go> {
    if versions.is_empty() {
        match failure {
            Some(error) => kit::muted(ui, &format!("Versions unavailable: {error}"), 12.5),
            None => kit::muted(ui, "Version list is loading…", 12.5),
        };
    }
    let mut go = None;
    for (i, version) in versions.iter().enumerate().rev() {
        let current = *version == c.version;
        let latest = i + 1 == versions.len();
        let tag = match (current, latest) {
            (true, true) => Some("Viewing · latest"),
            (true, false) => Some("Viewing"),
            (false, true) => Some("Latest"),
            (false, false) => None,
        };
        if row(ui, &format!("v{version}"), "", tag, !current).clicked() && !current {
            go = Some(Go::Version(version.clone()));
        }
    }
    go
}

fn attachment_rows(c: &LibraryComponentDto, ui: &mut Ui) {
    if c.attachments.is_empty() {
        kit::muted(ui, "This component declares no attachments.", 12.5);
    }
    for (key, spec) in &c.attachments {
        let accepts = format!("{} · accepts {}", spec.on, spec.accepts.join(", "));
        row(ui, key, &accepts, None, false);
    }
}

fn detail_rows(c: &LibraryComponentDto, ui: &mut Ui) {
    let generator = c
        .generator_id
        .as_ref()
        .map(|id| format!("{id} v{}", c.generator_version.unwrap_or(0)));
    let rows = [
        ("ID", Some(c.id.clone())),
        ("Version", Some(c.version.clone())),
        ("Revision", Some(c.revision.clone())),
        ("Source", Some(source_label(&c.source_kind).to_owned())),
        ("Generator", generator),
        ("Standard", c.standard.clone()),
        ("Designation", c.designation.clone()),
        ("Unit", c.unit.clone()),
        (
            "Category",
            Some(c.category.join(" / ")).filter(|s| !s.is_empty()),
        ),
        ("Tags", Some(c.tags.join(", ")).filter(|s| !s.is_empty())),
    ];
    for (label, value) in rows {
        if let Some(value) = value {
            row(ui, label, &value, None, false);
        }
    }
}

/// One 42px list row: name, mono detail, optional accent tag.
fn row(ui: &mut Ui, a: &str, b: &str, tag: Option<&str>, clickable: bool) -> egui::Response {
    let t = look(ui.ctx()).theme;
    let sense = if clickable {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 42.0), sense);
    let p = ui.painter();
    if clickable && r.hovered() {
        p.rect_filled(rect, CornerRadius::same(6), t.hover);
    }
    p.hline(
        rect.x_range(),
        rect.top() + 0.5,
        Stroke::new(1.0, t.border_subtle),
    );
    let name = egui::FontId::new(13.0, fonts::semibold());
    let g = kit::fit(p, a, name, t.ink, 120.0);
    p.galley(
        pos2(rect.left() + 4.0, rect.center().y - g.size().y / 2.0),
        g,
        t.ink,
    );
    let tag_w = tag.map_or(0.0, |s| s.len() as f32 * 6.5 + 28.0);
    let max = (rect.width() - 134.0 - tag_w).max(40.0);
    let font = egui::FontId::new(12.0, fonts::mono());
    let g = kit::fit(p, b, font, t.ink4, max);
    p.galley(
        pos2(rect.left() + 130.0, rect.center().y - g.size().y / 2.0),
        g,
        t.ink4,
    );
    if let Some(tag) = tag {
        let at = pos2(rect.right() - tag_w, rect.center().y - 10.0);
        kit::paint_chip(p, &t, at, tag, ChipKind::Accent);
    }
    if clickable {
        return r.on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    r
}
