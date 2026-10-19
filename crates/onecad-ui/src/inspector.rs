//! Inspector: Properties (the active tool / session editor) and History tabs.
use crate::{app::App, layout::Chrome, ui_state::InspectorTab};
use egui::{Context, Margin, Order, Ui};
use onecad_widgets::{
    components::{
        lists::{Segment, SegmentedControl},
        shell::{FloatingPanel, PanelToggle, Side},
        tooltip::Tip,
    },
    frame,
};

pub fn show(app: &mut App, ctx: &Context, chrome: &Chrome) {
    if let Some(rect) = chrome.inspector_toggle {
        let dot =
            crate::tool_registry::selection_kind(app) != crate::tool_registry::SelectionKind::None;
        let tip = Tip::new("Inspector").shortcut(Some("⌘2"));
        frame::floating(
            egui::Id::unique("inspector-toggle"),
            rect.min,
            Order::Middle,
        )
        .show(ctx, |ui| {
            if ui
                .add(PanelToggle::new(Side::Right).dot(dot).tip(tip))
                .clicked()
            {
                app.ui.inspector_open = true;
            }
        });
    }
    FloatingPanel::new(egui::Id::unique("inspector"), Side::Right)
        .open(app.ui.inspector_open)
        .rect(chrome.inspector)
        .show(ctx, |ui| {
            egui::Frame::new()
                .inner_margin(Margin::same(12))
                .show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    header(app, ui);
                    ui.add_space(10.0);
                    match app.ui.inspector_tab {
                        InspectorTab::Properties => {
                            sketch_options(app, ui);
                            crate::inspector_props::properties(app, ui, chrome.inspector);
                            crate::constraint_authoring::dimension_list(app, ui);
                            crate::profile_diagnostics::show(app, ui);
                        }
                        InspectorTab::History => crate::inspector_history::history(app, ui),
                    }
                });
        });
}

fn sketch_options(app: &mut App, ui: &mut Ui) {
    if !app.sketch_active() {
        return;
    }
    crate::sketch_methods::show(app, ui);
    construction_mode(app, ui);
    ui.horizontal(|ui| {
        ui.checkbox(&mut app.ui.sketch_snap, "Snap");
        ui.add_enabled_ui(app.ui.sketch_snap, |ui| {
            ui.checkbox(&mut app.ui.sketch_grid_snap, "1 mm grid snap");
        });
    });
    let numeric = crate::sketch_numeric::availability(app);
    if ui
        .add_enabled(numeric.is_ok(), egui::Button::new("Numeric input · Tab"))
        .on_disabled_hover_text(numeric.err().unwrap_or_default())
        .clicked()
    {
        crate::sketch_numeric::open(app, ui.ctx());
    }
}

fn construction_mode(app: &mut App, ui: &mut Ui) {
    let Some(session) = app.sketch.session.as_ref() else {
        return;
    };
    let key = session.key.clone();
    let mut construction = session.construction;
    if ui
        .add_enabled(
            crate::sketch_ui::tools_enabled(app),
            egui::Checkbox::new(&mut construction, "Construction geometry"),
        )
        .changed()
    {
        app.sketch_dispatch(
            onecad_interaction::sketch::SketchAction::SetConstructionMode { key, construction },
        );
    }
}

fn header(app: &mut App, ui: &mut Ui) {
    ui.horizontal(|ui| {
        let mut tab = usize::from(app.ui.inspector_tab == InspectorTab::History);
        let segments = [
            Segment {
                label: "Properties",
                icon: None,
            },
            Segment {
                label: "History",
                icon: None,
            },
        ];
        ui.add(SegmentedControl::new("inspector-tabs", &mut tab, &segments).width(180.0));
        app.ui.inspector_tab = if tab == 1 {
            InspectorTab::History
        } else {
            InspectorTab::Properties
        };
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let tip = Tip::new("Hide inspector").shortcut(Some("⌘2"));
            let toggle = PanelToggle::new(Side::Right)
                .open(true)
                .compact(true)
                .tip(tip);
            if ui.add(toggle).clicked() {
                app.ui.inspector_open = false;
            }
        });
    });
}
