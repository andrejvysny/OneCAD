//! Explorer Library mode (Final 11a): static catalogue, filtering and row drawing.
use super::EditorFacade;
use crate::app::App;
use egui::{Color32, CornerRadius, FontId, Rect, Response, Sense, Stroke, Ui, pos2, vec2};
use onecad_widgets::{
    components::{
        inputs::{SearchField, Select},
        lists::{SectionLabel, Segment, SegmentedControl},
    },
    fonts,
    icon::{self, IconStyle},
    theme::look,
};

pub const SOURCES: [&str; 4] = [
    "All sources",
    "My components",
    "This project",
    "Standard parts",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Mine,
    Project,
    Standard,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Recent,
    Standard,
    Project,
}

pub struct Item {
    pub name: &'static str,
    pub meta: &'static str,
    pub icon: &'static str,
    pub source: Source,
    pub section: Section,
    pub expandable: bool,
}

const fn item(
    name: &'static str,
    meta: &'static str,
    icon: &'static str,
    source: Source,
    section: Section,
) -> Item {
    Item {
        name,
        meta,
        icon,
        source,
        section,
        expandable: false,
    }
}

pub const ITEMS: [Item; 6] = [
    item(
        "Mounting bracket",
        "fx 7 · v3",
        "component",
        Source::Mine,
        Section::Recent,
    ),
    item(
        "Socket head M4×12",
        "ISO 4762 · A2",
        "thread",
        Source::Standard,
        Section::Recent,
    ),
    Item {
        expandable: true,
        ..item(
            "Socket head cap screw",
            "ISO 4762 · 84 sizes",
            "thread",
            Source::Standard,
            Section::Standard,
        )
    },
    item(
        "Hex nut",
        "ISO 4032",
        "polygon",
        Source::Standard,
        Section::Standard,
    ),
    item(
        "Deep groove bearing",
        "ISO 15 · 608",
        "revolve",
        Source::Standard,
        Section::Standard,
    ),
    item(
        "Pi 5 standoff set",
        "Static · 4 bodies",
        "hole",
        Source::Project,
        Section::Project,
    ),
];

/// Size sub-rows of the expandable screw: (size, meta).
const SIZES: [(&str, &str); 3] = [
    ("M3×10", "ISO 4762 · A2"),
    ("M4×12", "ISO 4762 · A2"),
    ("M5×16", "ISO 4762 · A2"),
];

const SECTIONS: [(Section, &str); 3] = [
    (Section::Recent, "RECENT"),
    (Section::Standard, "STANDARD PARTS"),
    (Section::Project, "THIS PROJECT"),
];

fn source_matches(index: usize, source: Source) -> bool {
    matches!(
        (index, source),
        (0, _) | (1, Source::Mine) | (2, Source::Project) | (3, Source::Standard)
    )
}

/// Items passing the source filter and a case-insensitive name search.
pub fn visible(source: usize, query: &str) -> Vec<&'static Item> {
    let query = query.trim().to_lowercase();
    ITEMS
        .iter()
        .filter(|i| source_matches(source, i.source))
        .filter(|i| query.is_empty() || i.name.to_lowercase().contains(&query))
        .collect()
}

/// Model | Library switch in the Explorer's first row.
pub fn mode_switch(f: &mut EditorFacade, ui: &mut Ui) {
    if !super::PREVIEW_FACADES {
        return;
    }
    let mut index = usize::from(f.library_mode);
    let segments = [
        Segment {
            label: "Model",
            icon: Some("layers"),
        },
        Segment {
            label: "Library",
            icon: Some("component"),
        },
    ];
    let width = ui.available_width() - 4.0;
    ui.add(SegmentedControl::new("explorer-mode", &mut index, &segments).width(width));
    f.library_mode = index == 1;
    ui.add_space(6.0);
}

pub fn list(app: &mut App, ui: &mut Ui) {
    if !super::PREVIEW_FACADES {
        return;
    }
    ui.spacing_mut().item_spacing.y = 0.0;
    controls(&mut app.ui.editor_facade, ui);
    let height = (ui.available_height() - 36.0).max(0.0);
    let mut picked = None;
    egui::ScrollArea::vertical()
        .max_height(height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            picked = sections(&mut app.ui.editor_facade, ui);
        });
    if let Some(name) = picked
        && super::placement_allowed(app)
    {
        app.ui.editor_facade.begin_placement(&name);
    }
}

fn controls(f: &mut EditorFacade, ui: &mut Ui) {
    let width = ui.available_width() - 4.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let field = SearchField::new("library-search", &mut f.search)
            .placeholder("Search library")
            .width(width - 36.0);
        ui.add(field);
        menu_square(ui);
    });
    ui.add_space(6.0);
    let out = Select::new("library-source", SOURCES[f.source], &SOURCES).width(width);
    if let Some(i) = out.show(ui).chosen {
        f.source = i;
    }
    ui.add_space(4.0);
}

/// 30px square with the list-options glyph; the options themselves are not built yet.
fn menu_square(ui: &mut Ui) {
    let l = look(ui.ctx());
    let (rect, r) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::click());
    let fill = if r.hovered() {
        l.theme.hover2
    } else {
        l.theme.chip
    };
    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
    let style = IconStyle {
        tint: l.theme.ink3,
        bg: fill,
        grey: false,
    };
    let glyph = Rect::from_center_size(rect.center(), vec2(14.0, 14.0));
    icon::paint(ui.painter(), glyph, "menu", l.dark, style, 1.0);
}

/// Draws the sections; returns the component name chosen for placement.
fn sections(f: &mut EditorFacade, ui: &mut Ui) -> Option<String> {
    let items = visible(f.source, &f.search);
    let mut picked = None;
    for (section, title) in SECTIONS {
        let rows: Vec<_> = items.iter().filter(|i| i.section == section).collect();
        if rows.is_empty() {
            continue;
        }
        ui.add(SectionLabel::new(title));
        for it in rows {
            let selected = f.placing.as_ref().is_some_and(|p| p.name == it.name);
            let chevron = it.expandable.then_some(f.expanded);
            let r = row(ui, it.name, it.meta, Some(it.icon), selected, chevron);
            if r.clicked() {
                if it.expandable {
                    f.expanded = !f.expanded;
                } else {
                    picked = Some(it.name.to_owned());
                }
            }
            if it.expandable && f.expanded {
                picked = picked.or(size_rows(f, ui));
            }
        }
    }
    picked
}

fn size_rows(f: &EditorFacade, ui: &mut Ui) -> Option<String> {
    let mut picked = None;
    for (size, meta) in SIZES {
        let name = format!("Socket head {size}");
        let selected = f.placing.as_ref().is_some_and(|p| p.name == name);
        if row(ui, size, meta, None, selected, None).clicked() {
            picked = Some(name);
        }
    }
    picked
}

/// 52px row (34px without a thumbnail): 40px well, name 13, meta 11.5, optional chevron.
fn row(
    ui: &mut Ui,
    name: &str,
    meta: &str,
    thumb: Option<&str>,
    selected: bool,
    chevron: Option<bool>,
) -> Response {
    let l = look(ui.ctx());
    let t = &l.theme;
    let h = if thumb.is_some() { 52.0 } else { 40.0 };
    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
    let fill = match (selected, r.hovered()) {
        (true, _) => t.sel_bg,
        (false, true) => t.hover,
        _ => Color32::TRANSPARENT,
    };
    let p = ui.painter();
    p.rect_filled(rect.shrink2(vec2(2.0, 0.0)), CornerRadius::same(8), fill);
    let mut x = rect.left() + 10.0;
    match thumb {
        Some(name) => {
            let well = Rect::from_min_size(pos2(x, rect.center().y - 20.0), vec2(40.0, 40.0));
            p.rect_filled(well, CornerRadius::same(8), t.well);
            let style = IconStyle {
                tint: t.ink3,
                bg: t.well,
                grey: false,
            };
            let glyph = Rect::from_center_size(well.center(), vec2(24.0, 24.0));
            icon::paint(p, glyph, name, l.dark, style, 1.0);
            x += 40.0 + 10.0;
        }
        None => x += 50.0,
    }
    let clip = Rect::from_min_max(
        pos2(x, rect.top()),
        pos2(rect.right() - 26.0, rect.bottom()),
    );
    let p = p.with_clip_rect(clip);
    let ink = if selected { t.sel_text } else { t.ink };
    let cy = rect.center().y;
    p.text(
        pos2(x, cy - 8.0),
        egui::Align2::LEFT_CENTER,
        name,
        FontId::new(13.0, fonts::medium()),
        ink,
    );
    p.text(
        pos2(x, cy + 8.0),
        egui::Align2::LEFT_CENTER,
        meta,
        FontId::proportional(11.5),
        t.ink5,
    );
    if let Some(open) = chevron {
        chevron_glyph(ui, pos2(rect.right() - 16.0, cy), open, t.ink5);
    }
    r
}

fn chevron_glyph(ui: &Ui, c: egui::Pos2, up: bool, col: Color32) {
    let d = if up { -2.5 } else { 2.5 };
    let pts = [
        pos2(c.x - 4.0, c.y - d),
        pos2(c.x, c.y + d),
        pos2(c.x + 4.0, c.y - d),
    ];
    ui.painter()
        .add(egui::Shape::line(pts.to_vec(), Stroke::new(1.5, col)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_is_case_insensitive_on_name() {
        let names: Vec<_> = visible(0, "SOCKET").iter().map(|i| i.name).collect();
        assert_eq!(names, vec!["Socket head M4×12", "Socket head cap screw"]);
        assert!(visible(0, "zzz").is_empty());
    }

    #[test]
    fn source_filter_selects_by_origin() {
        assert_eq!(visible(0, "").len(), ITEMS.len());
        let mine: Vec<_> = visible(1, "").iter().map(|i| i.name).collect();
        assert_eq!(mine, vec!["Mounting bracket"]);
        assert_eq!(visible(2, "").len(), 1);
        assert_eq!(visible(3, "").len(), 4);
        assert!(visible(1, "nut").is_empty());
    }
}
