//! Inspector Properties tab (README 03a): title block, data rows and the footer hint.
//! Only values already present in the projection / sketch session are shown; the rest
//! of the design's rows render "—".
use crate::{
    app::App,
    pill::{body_name, selection_label},
    tool_registry::{SelectionKind, selection_kind},
};
use egui::{CornerRadius, Rect, RichText, Sense, Ui, pos2, vec2};
use onecad_widgets::{
    components::lists::{PropertyRow, SectionLabel},
    fonts,
    icon::{self, IconStyle},
    metrics::{PANEL_TITLE, SECONDARY},
};

const NONE_HINT: &str = "Select a face, edge or body in the viewport.";
const DASH: &str = "—";

struct Block {
    icon: &'static str,
    title: String,
    subtitle: String,
    sections: Vec<(&'static str, Vec<(&'static str, String)>)>,
}

fn dash_rows(labels: &[&'static str]) -> Vec<(&'static str, String)> {
    labels.iter().map(|l| (*l, DASH.to_owned())).collect()
}

fn body_section(name: String) -> (&'static str, Vec<(&'static str, String)>) {
    let mut rows = vec![("Name", name)];
    rows.extend(dash_rows(&["Material", "Mass", "Volume"]));
    ("BODY", rows)
}

fn region_sketch(app: &App) -> Option<String> {
    let id = app
        .interaction
        .selection
        .as_ref()?
        .profile
        .sketch
        .to_string();
    let display = app.display.as_ref()?;
    display
        .projection
        .sketches
        .values()
        .find(|s| s.id == id)
        .map(|s| s.name.clone())
}

fn sketch_block(app: &App) -> Block {
    let name = app
        .sketch
        .session
        .as_ref()
        .map_or_else(String::new, |s| s.authoritative.name.clone());
    let dof = app
        .sketch
        .session
        .as_ref()
        .map(|session| crate::sketch_feedback::summary(&session.status).label);
    let mut rows = vec![("Name", name.clone())];
    rows.push(("Solve", dof.clone().unwrap_or_else(|| DASH.to_owned())));
    Block {
        icon: "sketch",
        title: "Sketch".into(),
        subtitle: [Some(name), dof]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · "),
        sections: vec![("SKETCH", rows)],
    }
}

fn feature_block(app: &App) -> Option<Block> {
    let id = app.selected_feature?.to_string();
    let feature = app
        .display
        .as_ref()?
        .projection
        .features
        .iter()
        .find(|f| f.id == id)?;
    let mut rows = vec![
        ("Type", feature.op_type.clone()),
        ("Value", feature.value_text.clone()),
    ];
    if let Some(message) = &feature.status_message {
        rows.push(("Status", message.clone()));
    }
    Some(Block {
        icon: crate::inspector_history::feature_icon(&feature.op_type),
        title: feature.label.clone(),
        subtitle: feature.value_text.clone(),
        sections: vec![("FEATURE", rows)],
    })
}

fn block(app: &App) -> Block {
    let kind = selection_kind(app);
    let label = selection_label(app, kind);
    match kind {
        SelectionKind::SketchIdle | SelectionKind::SketchEntity => sketch_block(app),
        SelectionKind::Face | SelectionKind::Edge => face_edge_block(kind, label.map(|l| l.1)),
        SelectionKind::Body => body_block(app),
        SelectionKind::Region => region_block(app),
        SelectionKind::Feature => feature_block(app).unwrap_or_else(none_block),
        SelectionKind::None => none_block(),
    }
}

fn face_edge_block(kind: SelectionKind, body: Option<String>) -> Block {
    let face = kind == SelectionKind::Face;
    let (title, geometry) = if face {
        ("Face", dash_rows(&["Area", "Perimeter", "Normal"]))
    } else {
        ("Edge", dash_rows(&["Length"]))
    };
    let body = body.unwrap_or_default();
    Block {
        icon: if face { "body" } else { "line" },
        title: title.into(),
        subtitle: format!("on {body}"),
        sections: vec![("GEOMETRY", geometry), body_section(body)],
    }
}

fn body_block(app: &App) -> Block {
    let name = app
        .selected_body
        .map(|id| body_name(app, id))
        .unwrap_or_default();
    Block {
        icon: "body",
        title: name.clone(),
        subtitle: "Body".into(),
        sections: vec![body_section(name)],
    }
}

fn region_block(app: &App) -> Block {
    let sketch = region_sketch(app);
    Block {
        icon: "sketchEntity",
        title: "Profile".into(),
        subtitle: sketch.clone().unwrap_or_else(|| "Sketch profile".into()),
        sections: sketch
            .map(|name| vec![("SKETCH", vec![("Sketch", name)])])
            .unwrap_or_default(),
    }
}

fn none_block() -> Block {
    Block {
        icon: "select",
        title: "Nothing selected".into(),
        subtitle: NONE_HINT.into(),
        sections: Vec::new(),
    }
}

pub fn properties(app: &mut App, ui: &mut Ui, panel: Rect) {
    let b = block(app);
    title_block(app, ui, &b);
    for (name, rows) in &b.sections {
        ui.add(SectionLabel::new(name));
        for (label, value) in rows {
            ui.add(PropertyRow::new(label).value(value));
        }
        ui.add_space(4.0);
    }
    crate::inspector_ext::sheet_metal(app, ui);
    hint(app, ui, panel);
}

fn title_block(app: &App, ui: &mut Ui, b: &Block) {
    let t = &app.theme;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let (well, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
        ui.painter()
            .rect_filled(well, CornerRadius::same(8), t.well);
        let style = IconStyle {
            tint: t.ink4,
            bg: t.well,
            grey: false,
        };
        icon::paint(ui.painter(), well.shrink(6.0), b.icon, app.dark, style, 1.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            let title = RichText::new(&b.title)
                .size(PANEL_TITLE)
                .family(fonts::semibold());
            ui.label(title.color(t.ink));
            let sub = RichText::new(&b.subtitle).size(SECONDARY).color(t.ink4);
            ui.add(egui::Label::new(sub).wrap());
        });
    });
    ui.add_space(8.0);
}

/// Footer hint pinned to the panel bottom.
fn hint(app: &App, ui: &mut Ui, panel: Rect) {
    let t = &app.theme;
    let bar = Rect::from_min_max(pos2(panel.left(), panel.bottom() - 36.0), panel.max);
    let p = ui.painter();
    p.hline(
        bar.x_range(),
        bar.top(),
        egui::Stroke::new(1.0, t.border_subtle),
    );
    let style = IconStyle {
        tint: t.ink5,
        bg: t.floating,
        grey: false,
    };
    let glyph = Rect::from_center_size(pos2(bar.left() + 20.0, bar.center().y), vec2(14.0, 14.0));
    icon::paint(p, glyph, "help", app.dark, style, 1.0);
    p.text(
        pos2(bar.left() + 34.0, bar.center().y),
        egui::Align2::LEFT_CENTER,
        "⇧-click adds faces · Esc clears",
        egui::FontId::proportional(11.5),
        t.ink5,
    );
}
