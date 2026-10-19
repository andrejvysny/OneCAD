//! Wizard step bodies: Source, Details, Parameters, Save.
use super::{CATEGORIES, DESTINATIONS, DIMENSIONS, WizardState};
use crate::library_pages::kit;
use egui::{
    Align, CornerRadius, FontFamily, FontId, Layout, Rect, Response, Sense, Stroke, StrokeKind, Ui,
    pos2, vec2,
};
use onecad_widgets::{
    components::{
        inputs::Select,
        lists::{Segment, SegmentedControl},
    },
    fonts,
    theme::look,
};

pub fn show(ui: &mut Ui, w: &mut WizardState) {
    match w.step {
        0 => source(ui, w),
        1 => details(ui, w),
        2 => parameters(ui, w),
        _ => save(ui, w),
    }
}

fn caption(ui: &mut Ui, text: &str) {
    let ink = look(ui.ctx()).theme.ink4;
    kit::label(ui, text, 12.0, FontFamily::Proportional, ink);
}

fn source(ui: &mut Ui, w: &mut WizardState) {
    ui.spacing_mut().item_spacing.y = 10.0;
    kit::title(ui, "What goes into the component", 14.0);
    kit::muted(ui, "1 body · Bracket, 7 features, 2 sketches", 12.5);
    ui.checkbox(&mut w.body, "Body · Bracket");
    ui.checkbox(&mut w.sketches, "Sketches · Base profile, Upright profile");
}

fn details(ui: &mut Ui, w: &mut WizardState) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 22.0;
        ui.allocate_ui_with_layout(vec2(230.0, 380.0), Layout::top_down(Align::Min), |ui| {
            thumbnail(ui, w);
        });
        let width = ui.available_width();
        ui.allocate_ui_with_layout(vec2(width, 380.0), Layout::top_down(Align::Min), |ui| {
            fields(ui, w);
        });
    });
}

fn thumbnail(ui: &mut Ui, w: &mut WizardState) {
    let t = look(ui.ctx()).theme;
    let (rect, _) = ui.allocate_exact_size(vec2(230.0, 200.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(10), t.well);
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(10),
        Stroke::new(1.0, t.border_subtle),
        StrokeKind::Inside,
    );
    ui.add_space(10.0);
    let text = if w.retaken {
        "Thumbnail updated from view"
    } else {
        "Retake thumbnail from view"
    };
    if kit::link(ui, text, 12.0).clicked() {
        w.retaken = true;
    }
    ui.add_space(14.0);
    let frame = egui::Frame::new()
        .fill(t.panel)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(12, 10));
    frame.show(ui, |ui| {
        ui.set_width(206.0);
        ui.spacing_mut().item_spacing.y = 3.0;
        kit::title(ui, "Source", 12.0);
        kit::muted(ui, "1 body · Bracket", 12.0);
        kit::muted(ui, "7 features · 2 sketches", 12.0);
    });
}

fn fields(ui: &mut Ui, w: &mut WizardState) {
    let width = ui.available_width();
    ui.spacing_mut().item_spacing.y = 5.0;
    caption(ui, "Name");
    let name = egui::TextEdit::singleline(&mut w.name)
        .font(FontId::new(12.5, fonts::mono()))
        .margin(vec2(10.0, 6.0))
        .desired_width(width);
    ui.add(name);
    ui.add_space(8.0);
    let half = (width - 12.0) / 2.0;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        ui.vertical(|ui| {
            caption(ui, "Category");
            let out = Select::new("wizard-category", CATEGORIES[w.category], &CATEGORIES)
                .width(half)
                .show(ui);
            if let Some(i) = out.chosen {
                w.category = i;
            }
        });
        ui.vertical(|ui| {
            caption(ui, "Save to");
            let segs = DESTINATIONS.map(|label| Segment { label, icon: None });
            ui.add(SegmentedControl::new("wizard-save-to", &mut w.save_to, &segs).width(half));
        });
    });
    ui.add_space(8.0);
    let t = look(ui.ctx()).theme;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        caption(ui, "Description");
        kit::label(ui, "optional", 12.0, FontFamily::Proportional, t.ink5);
    });
    let description = egui::TextEdit::multiline(&mut w.description)
        .margin(vec2(10.0, 8.0))
        .desired_rows(4)
        .desired_width(width);
    ui.add(description);
    ui.add_space(8.0);
    caption(ui, "Type");
    type_cards(ui, w, width);
}

fn type_cards(ui: &mut Ui, w: &mut WizardState, width: f32) {
    let half = (width - 12.0) / 2.0;
    let cards = [
        ("Static", "Fixed geometry. Inserts exactly as modeled."),
        ("Parametric", "Pick dimensions to expose. Next step."),
    ];
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        for (i, (title, text)) in cards.into_iter().enumerate() {
            if radio_card(ui, half, w.kind == i, title, text).clicked() {
                w.kind = i;
            }
        }
    });
}

fn radio_card(ui: &mut Ui, width: f32, on: bool, title: &str, text: &str) -> Response {
    let t = look(ui.ctx()).theme;
    let (rect, r) = ui.allocate_exact_size(vec2(width, 80.0), Sense::click());
    let (fill, border) = match (on, r.hovered()) {
        (true, _) => (t.sel_soft, t.accent),
        (false, true) => (t.hover, t.border_strong),
        (false, false) => (t.surface, t.border),
    };
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(9), fill);
    p.rect_stroke(
        rect,
        CornerRadius::same(9),
        Stroke::new(1.0, border),
        StrokeKind::Inside,
    );
    let c = pos2(rect.left() + 22.0, rect.top() + 22.0);
    p.circle_stroke(
        c,
        8.0,
        Stroke::new(1.0, if on { t.accent } else { t.border_strong }),
    );
    if on {
        p.circle_filled(c, 4.5, t.accent);
    }
    let x = rect.left() + 40.0;
    let font = FontId::new(13.0, fonts::semibold());
    p.text(
        pos2(x, rect.top() + 22.0),
        egui::Align2::LEFT_CENTER,
        title,
        font,
        t.ink,
    );
    let g = p.layout(
        text.to_owned(),
        FontId::proportional(12.0),
        t.ink4,
        rect.right() - 12.0 - x,
    );
    p.galley(pos2(x, rect.top() + 38.0), g, t.ink4);
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn parameters(ui: &mut Ui, w: &mut WizardState) {
    ui.spacing_mut().item_spacing.y = 6.0;
    let t = look(ui.ctx()).theme;
    if w.kind == 0 {
        kit::muted(ui, "Static components expose no parameters.", 12.5);
        return;
    }
    kit::muted(ui, "Pick dimensions on the model to expose them.", 12.5);
    ui.add_space(8.0);
    for (i, (name, value)) in DIMENSIONS.into_iter().enumerate() {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::hover());
        let p = ui.painter();
        p.hline(
            rect.x_range(),
            rect.top() + 0.5,
            Stroke::new(1.0, t.border_subtle),
        );
        p.text(
            pos2(rect.left() + 4.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            name,
            FontId::new(12.5, fonts::mono()),
            t.ink,
        );
        p.text(
            pos2(rect.left() + 180.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            value,
            FontId::new(12.5, fonts::mono()),
            t.ink4,
        );
        let check = Rect::from_min_size(
            pos2(rect.right() - 90.0, rect.center().y - 10.0),
            vec2(90.0, 20.0),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(check), |ui| {
            ui.checkbox(&mut w.exposed[i], "Expose");
        });
    }
}

fn save(ui: &mut Ui, w: &mut WizardState) {
    let t = look(ui.ctx()).theme;
    ui.spacing_mut().item_spacing.y = 4.0;
    let kind = if w.kind == 1 { "Parametric" } else { "Static" };
    let rows = [
        ("Name", w.name.clone()),
        ("Category", CATEGORIES[w.category].to_owned()),
        ("Save to", DESTINATIONS[w.save_to].to_owned()),
        ("Type", kind.to_owned()),
        ("Exposed parameters", w.exposed_count().to_string()),
    ];
    for (name, value) in rows {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
        let p = ui.painter();
        p.hline(
            rect.x_range(),
            rect.top() + 0.5,
            Stroke::new(1.0, t.border_subtle),
        );
        p.text(
            pos2(rect.left() + 4.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            name,
            FontId::proportional(12.5),
            t.ink4,
        );
        p.text(
            pos2(rect.left() + 180.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            value,
            FontId::proportional(12.5),
            t.ink,
        );
    }
    if w.saved {
        ui.add_space(14.0);
        let msg = format!("Saved to {}", DESTINATIONS[w.save_to]);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
            kit::paint_icon(ui.painter(), r, "check", t.surface, t.ok_text);
            kit::label(ui, &msg, 13.0, fonts::semibold(), t.ok_text);
        });
    }
}
