//! Transient viewport settings, applied by the host after GPU work completes.

use crate::app::App;
use onecad_render::{DisplayMode, RenderOptions};
use onecad_scene::Projection;
use onecad_widgets::components::lists::{MenuRow, SectionLabel, Segment, SegmentedControl};

/// Menu body; hosted by the title menu and by the floating chrome's popups.
pub fn contents(app: &mut App, ui: &mut egui::Ui) {
    ui.set_width(268.0);
    ui.spacing_mut().item_spacing.y = 6.0;
    let mut options = *app
        .pending_display_options
        .as_ref()
        .unwrap_or_else(|| app.renderer.options());
    let xray_supported = app
        .gpu
        .device
        .features()
        .contains(wgpu::Features::FLOAT32_BLENDABLE);
    // The Mobile render profile has no supersampling (target memory).
    let supersampling_offered = app.renderer.profile().supersampling();
    let mut changed = display_modes(ui, &mut options);
    projection(app, ui);
    egui::CollapsingHeader::new("More settings")
        .id_salt("display-more-settings")
        .show(ui, |ui| {
            changed |= xray(ui, &mut options, xray_supported);
            if supersampling_offered {
                changed |= supersampling(ui, &mut options);
            }
            changed |= ui
                .add_enabled_ui(
                    app.doc.interaction.tool.is_none() && !app.sketch_active(),
                    |ui| section(ui, app, &mut options),
                )
                .inner;
        });
    if changed {
        app.pending_display_options = Some(options);
        app.draw_dirty = true;
        ui.ctx().request_repaint();
    }
}

/// Same camera setting as the view cube's projection toggle.
fn projection(app: &mut App, ui: &mut egui::Ui) {
    ui.separator();
    ui.add(SectionLabel::new("Projection"));
    let mut index = usize::from(app.doc.camera.projection == Projection::Orthographic);
    let segments = [
        Segment {
            label: "Perspective",
            icon: None,
        },
        Segment {
            label: "Orthographic",
            icon: None,
        },
    ];
    let before = index;
    ui.add(
        SegmentedControl::new("display-projection", &mut index, &segments)
            .accent_selected()
            .width(ui.available_width()),
    );
    if index != before {
        app.doc.camera.projection = if index == 1 {
            Projection::Orthographic
        } else {
            Projection::Perspective
        };
        app.draw_dirty = true;
    }
}

fn display_modes(ui: &mut egui::Ui, options: &mut RenderOptions) -> bool {
    ui.add(SectionLabel::new("Render style"));
    let mut changed = false;
    for (mode, label, icon) in [
        (DisplayMode::Shaded, "Shaded", "shaded"),
        (DisplayMode::ShadedEdges, "Shaded with edges", "shadedEdges"),
        (DisplayMode::Wireframe, "Wireframe", "wireframe"),
    ] {
        let response = ui.add(
            MenuRow::new(Some(icon), label)
                .checked(options.display_mode == mode)
                .highlight_checked(),
        );
        if response.clicked() {
            changed |= options.display_mode != mode;
            options.display_mode = mode;
            ui.close();
        }
    }
    ui.add(MenuRow::new(Some("hiddenLine"), "Hidden line").enabled(false))
        .on_disabled_hover_text("Hidden line rendering is not available yet.");
    changed
}

fn xray(ui: &mut egui::Ui, options: &mut RenderOptions, supported: bool) -> bool {
    let response = ui
        .add(
            MenuRow::new(Some("hiddenLine"), "X-ray")
                .enabled(supported)
                .checked(options.display_mode == DisplayMode::Xray)
                .highlight_checked(),
        )
        .on_disabled_hover_text(
            "X-ray requires FLOAT32_BLENDABLE, unavailable on this GPU device.",
        );
    if response.clicked() {
        let changed = options.display_mode != DisplayMode::Xray;
        options.display_mode = DisplayMode::Xray;
        ui.close();
        changed
    } else {
        false
    }
}

#[cfg(test)]
fn display_menu<R>(
    ui: &mut egui::Ui,
    content: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<Option<R>> {
    ui.menu_button("Display", content)
}

fn supersampling(ui: &mut egui::Ui, options: &mut RenderOptions) -> bool {
    let mut changed = false;
    ui.menu_button(format!("SSAA {}×", options.ssaa), |ui| {
        for factor in [1.0, 2.0] {
            changed |= ui
                .selectable_value(&mut options.ssaa, factor, format!("{factor}×"))
                .changed();
        }
    });
    changed
}

fn section(ui: &mut egui::Ui, app: &App, options: &mut RenderOptions) -> bool {
    let original = section_axis(options.section);
    let mut axis = original;
    section_axis_ui(ui, &mut axis);
    let mut changed = axis != original;
    if changed {
        options.section = [0.0; 4];
        if let Some(axis) = axis {
            options.section[axis] = 1.0;
            options.section[3] = center_offset(app, axis);
        }
    }
    if axis.is_some() {
        let mut offset = options.section[3];
        let response = ui.add(
            egui::DragValue::new(&mut offset)
                .prefix("Offset ")
                .suffix(" mm"),
        );
        if response.changed() && offset.is_finite() {
            options.section[3] = offset;
            changed = true;
        }
        ui.label("Display only. Selection is disabled while a section is active.");
    }
    changed
}

fn section_axis_ui(ui: &mut egui::Ui, axis: &mut Option<usize>) -> egui::Response {
    // egui has one memory-backed popup per viewport; nested choices need submenu state.
    ui.menu_button(format!("Section {}", section_label(*axis)), |ui| {
        for candidate in [None, Some(2), Some(1), Some(0)] {
            ui.selectable_value(axis, candidate, section_label(candidate));
        }
    })
    .response
}

fn section_axis(section: [f32; 4]) -> Option<usize> {
    section[..3].iter().position(|normal| *normal != 0.0)
}

fn section_label(axis: Option<usize>) -> &'static str {
    match axis {
        Some(2) => "XY",
        Some(1) => "XZ",
        Some(0) => "YZ",
        _ => "Off",
    }
}

fn center_offset(app: &App, axis: usize) -> f32 {
    let center = app
        .doc
        .display
        .as_ref()
        .and_then(|display| display.scene.bounds())
        .map(|bounds| bounds.min * 0.5 + bounds.max * 0.5)
        .unwrap_or_default();
    let offset = center[axis] as f32;
    if offset.is_finite() { offset } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Harness {
        options: RenderOptions,
        xray_supported: bool,
        axis: Option<usize>,
        menu: egui::Rect,
    }

    impl Default for Harness {
        fn default() -> Self {
            Self {
                options: RenderOptions::default(),
                xray_supported: true,
                axis: None,
                menu: egui::Rect::NOTHING,
            }
        }
    }

    impl Harness {
        fn frame(&mut self, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 600.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    self.menu = display_menu(ui, |ui| {
                        ui.set_width(268.0);
                        display_modes(ui, &mut self.options);
                        xray(ui, &mut self.options, self.xray_supported);
                        ui.separator();
                        supersampling(ui, &mut self.options);
                        section_axis_ui(ui, &mut self.axis);
                    })
                    .response
                    .rect;
                },
            );
            output.textures_delta.clear();
            output
        }

        fn click(&mut self, ctx: &egui::Context, position: egui::Pos2) -> egui::FullOutput {
            self.frame(
                ctx,
                vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            self.frame(
                ctx,
                vec![egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            )
        }
    }

    fn text_position(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap_or_else(|| {
                panic!(
                    "missing visible text {label:?}; labels {:?}",
                    output
                        .shapes
                        .iter()
                        .filter_map(|shape| match &shape.shape {
                            egui::Shape::Text(text) => Some(text.galley.text()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                )
            })
    }

    #[test]
    fn display_choices_select_canonical_modes_and_preserve_pending_settings() {
        for (mode, label) in [
            (DisplayMode::Shaded, "Shaded"),
            (DisplayMode::ShadedEdges, "Shaded with edges"),
            (DisplayMode::Wireframe, "Wireframe"),
            (DisplayMode::Xray, "X-ray"),
        ] {
            let ctx = egui::Context::default();
            let mut host = Harness {
                options: RenderOptions {
                    display_mode: if mode == DisplayMode::Shaded {
                        DisplayMode::Wireframe
                    } else {
                        DisplayMode::Shaded
                    },
                    ssaa: 2.0,
                    section: [0.0, 0.0, 1.0, 5.0],
                    ..RenderOptions::default()
                },
                ..Harness::default()
            };
            let expected = RenderOptions {
                display_mode: mode,
                ..host.options
            };
            host.frame(&ctx, vec![]);
            host.click(&ctx, host.menu.center());
            let output = host.frame(&ctx, vec![]);
            host.click(&ctx, text_position(&output, label));
            host.frame(&ctx, vec![]);
            assert_eq!(host.options, expected);
        }
    }

    #[test]
    fn unsupported_xray_is_visible_but_cannot_change_options() {
        let ctx = egui::Context::default();
        let mut host = Harness {
            xray_supported: false,
            ..Harness::default()
        };
        let expected = host.options;
        host.frame(&ctx, vec![]);
        host.click(&ctx, host.menu.center());
        let output = host.frame(&ctx, vec![]);
        host.click(&ctx, text_position(&output, "X-ray"));
        host.frame(&ctx, vec![]);
        assert_eq!(host.options, expected);
    }

    #[test]
    fn nested_section_choice_remains_open_and_selects_plane() {
        let ctx = egui::Context::default();
        let mut host = Harness::default();
        host.frame(&ctx, vec![]);
        host.click(&ctx, host.menu.center());
        let output = host.frame(&ctx, vec![]);
        let combo = text_position(&output, "Section Off");
        host.click(&ctx, combo);
        let output = host.frame(&ctx, vec![]);
        let plane = text_position(&output, "XY");
        host.click(&ctx, plane);
        host.frame(&ctx, vec![]);
        assert_eq!(host.axis, Some(2));
    }

    #[test]
    fn nested_supersampling_choice_remains_open_and_selects_factor() {
        let ctx = egui::Context::default();
        let mut host = Harness::default();
        host.frame(&ctx, vec![]);
        host.click(&ctx, host.menu.center());
        let output = host.frame(&ctx, vec![]);
        let combo = text_position(&output, "SSAA 1×");
        host.click(&ctx, combo);
        let output = host.frame(&ctx, vec![]);
        let factor = text_position(&output, "2×");
        host.click(&ctx, factor);
        host.frame(&ctx, vec![]);
        assert_eq!(host.options.ssaa, 2.0);
    }
}
