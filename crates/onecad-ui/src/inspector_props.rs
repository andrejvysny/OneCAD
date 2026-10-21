//! Inspector Properties tab (README 03a): title block, data rows and the footer hint.
//! Shows supported projection/session values and identifies unavailable measurements.
use crate::{
    app::App,
    pill::{body_name, selection_label},
    tool_registry::{SelectionKind, selection_kind},
};
use egui::{CornerRadius, Rect, RichText, Sense, Ui, vec2};
use onecad_widgets::{
    components::lists::{PropertyRow, SectionLabel},
    fonts,
    icon::{self, IconStyle},
    metrics::{PANEL_TITLE, SECONDARY},
};

const NONE_HINT: &str = "Select a face, edge or body in the viewport.";
const UNAVAILABLE: &str = "Not evaluated";

struct Block {
    icon: &'static str,
    title: String,
    subtitle: String,
    sections: Vec<(&'static str, Vec<(&'static str, String)>)>,
}

fn body_section(name: String) -> (&'static str, Vec<(&'static str, String)>) {
    (
        "BODY",
        vec![("Name", name), ("Measurements", "Not available".into())],
    )
}

fn region_sketch(app: &App) -> Option<String> {
    let id = app
        .doc
        .interaction
        .selection
        .as_ref()?
        .profile
        .sketch
        .to_string();
    let display = app.doc.display.as_ref()?;
    display
        .projection
        .sketches
        .values()
        .find(|s| s.id == id)
        .map(|s| s.name.clone())
}

fn sketch_block(app: &App) -> Block {
    let name = app
        .doc
        .sketch
        .session
        .as_ref()
        .map_or_else(String::new, |s| s.authoritative.name.clone());
    let dof = app
        .doc
        .sketch
        .session
        .as_ref()
        .map(|session| crate::sketch_feedback::summary(&session.status).label);
    let mut rows = vec![("Name", name.clone())];
    rows.push((
        "Solve",
        dof.clone().unwrap_or_else(|| UNAVAILABLE.to_owned()),
    ));
    if let Some(session) = &app.doc.sketch.session {
        rows.push((
            "Entities",
            session.authoritative.entities().len().to_string(),
        ));
        rows.push((
            "Constraints",
            session.authoritative.constraints().len().to_string(),
        ));
    }
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
    let id = app.doc.selected_feature()?.to_string();
    let feature = app
        .doc
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
    if !app.sketch_active()
        && let Some(block) = selected_sketch_block(app)
    {
        return block;
    }
    let kind = selection_kind(app);
    let label = selection_label(app, kind);
    match kind {
        SelectionKind::SketchIdle | SelectionKind::SketchEntity => sketch_block(app),
        SelectionKind::Face | SelectionKind::Edge => face_edge_block(kind, label.map(|l| l.1)),
        SelectionKind::Body => body_block(app),
        SelectionKind::Region => region_block(app),
        SelectionKind::Feature => feature_block(app).unwrap_or_else(none_block),
        SelectionKind::None | SelectionKind::Mixed => none_block(),
    }
}

fn face_edge_block(kind: SelectionKind, body: Option<String>) -> Block {
    let face = kind == SelectionKind::Face;
    let (title, geometry) = if face {
        ("Face", vec![("Measurements", "Not available".into())])
    } else {
        ("Edge", vec![("Measurements", "Not available".into())])
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
        .doc
        .selected_body()
        .map(|id| body_name(app, id))
        .unwrap_or_default();
    let mut section = body_section(name.clone());
    if let Some(body) = app.doc.selected_body().and_then(|id| {
        app.doc
            .display
            .as_ref()?
            .projection
            .bodies
            .values()
            .find(|body| body.id == id.to_string())
    }) {
        section.1.push((
            "Visibility",
            if body.visible { "Shown" } else { "Hidden" }.into(),
        ));
        section.1.push(("Health", format!("{:?}", body.health)));
    }
    Block {
        icon: "body",
        title: name.clone(),
        subtitle: "Body".into(),
        sections: vec![section],
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

pub fn properties(app: &mut App, ui: &mut Ui, _panel: Rect) {
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
    hint(app, ui);
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

fn hint(app: &App, ui: &mut Ui) {
    ui.add_space(6.0);
    ui.add(
        egui::Label::new(
            RichText::new("Esc clears selection")
                .size(11.5)
                .color(app.theme.ink5),
        )
        .wrap(),
    );
}

fn selected_sketch_block(app: &App) -> Option<Block> {
    let id = app.doc.selected_sketch()?.to_string();
    let sketch = app
        .doc
        .display
        .as_ref()?
        .projection
        .sketches
        .values()
        .find(|sketch| sketch.id == id)?;
    let solve = sketch.dof.map_or_else(
        || "Not evaluated".into(),
        |dof| format!("{dof} degrees of freedom"),
    );
    let mut rows = vec![
        ("Name", sketch.name.clone()),
        (
            "Visibility",
            if sketch.visible { "Shown" } else { "Hidden" }.into(),
        ),
        ("Solve", solve),
    ];
    if let Some(host) = &sketch.host_face {
        rows.push(("Support body", host.body_id.clone()));
        rows.push(("Support face", host.element_id.clone()));
    }
    Some(Block {
        icon: "sketch",
        title: "Sketch".into(),
        subtitle: sketch.name.clone(),
        sections: vec![("SKETCH", rows)],
    })
}
