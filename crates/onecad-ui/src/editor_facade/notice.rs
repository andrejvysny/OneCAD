//! Extension failure notice (Extensions 21b): bottom-right card while Topology Check is down.
use crate::{app::App, layout::Chrome};
use egui::{Align2, Id, Margin, Order, Stroke, Ui, pos2, vec2};
use onecad_widgets::{
    components::button::{Button, ButtonKind},
    fonts, frame,
    icon::{self, IconStyle},
    metrics::Elevation,
    theme::look,
};

const CARD_W: f32 = 330.0;
const BODY: &str = "Its 2 tools and 1 layer are off until it restarts. Your document and other tools aren't affected.";

pub fn show(app: &mut App, ctx: &egui::Context, chrome: &Chrome) {
    if !app.ui.editor_facade.topology_failed {
        return;
    }
    let right = if app.ui.inspector_open {
        chrome.inspector.left() - 10.0
    } else {
        chrome.viewport.right() - 10.0
    };
    let at = pos2(right, chrome.viewport.bottom() - 10.0);
    let l = look(ctx);
    let mut dismiss = false;
    let f = &mut app.ui.editor_facade;
    frame::floating(Id::unique("ext-failure"), at, Order::Foreground)
        .pivot(Align2::RIGHT_BOTTOM)
        .show(ctx, |ui| {
            frame::surface(&l.theme, l.dark, 12, Elevation::Card)
                .stroke(Stroke::new(1.0, l.theme.danger_border))
                .inner_margin(Margin::same(14))
                .show(ui, |ui| {
                    ui.set_width(CARD_W - 28.0);
                    ui.spacing_mut().item_spacing.y = 8.0;
                    dismiss = header(ui, &l.theme, l.dark);
                    ui.add(
                        egui::Label::new(egui::RichText::new(BODY).size(12.0).color(l.theme.ink3))
                            .wrap(),
                    );
                    dismiss |= actions(ui, f.log_copied, &mut f.log_copied);
                });
        });
    if dismiss {
        app.ui.editor_facade.dismiss_failure();
    }
}

/// Title row; true when the close button was pressed.
fn header(ui: &mut Ui, t: &onecad_widgets::theme::Theme, dark: bool) -> bool {
    let mut close = false;
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), egui::Sense::hover());
        let style = IconStyle {
            tint: t.danger_strong,
            bg: t.floating,
            grey: false,
        };
        icon::paint(ui.painter(), r, "failed", dark, style, 1.0);
        ui.label(
            egui::RichText::new("Topology Check stopped")
                .size(13.0)
                .family(fonts::semibold())
                .color(t.ink),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            close = ui
                .add(
                    Button::new("")
                        .kind(ButtonKind::Ghost)
                        .icon(Some("close"))
                        .min_width(30.0),
                )
                .clicked();
        });
    });
    close
}

/// Footer row; true when Restart or Disable was pressed.
fn actions(ui: &mut Ui, copied: bool, copied_flag: &mut bool) -> bool {
    let mut done = false;
    ui.horizontal(|ui| {
        let label = if copied { "Log copied" } else { "Copy log" };
        let t = look(ui.ctx()).theme;
        let link = egui::Label::new(egui::RichText::new(label).size(12.0).color(t.accent))
            .sense(egui::Sense::click());
        if ui.add(link).clicked() {
            *copied_flag = true;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            done |= ui
                .add(Button::new("Restart").kind(ButtonKind::Primary))
                .clicked();
            done |= ui.add(Button::new("Disable")).clicked();
        });
    });
    done
}
