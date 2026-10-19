//! Layers popover above the nav pill (Extensions 19d). Eyes only flip facade booleans.
use crate::{app::App, layout::Chrome};
use egui::{Align2, Color32, Context, CornerRadius, FontId, Order, Pos2, Rect, Sense, Ui, vec2};
use onecad_widgets::{
    components::lists::{MenuSection, menu_frame},
    fonts, frame,
    icon::{self, IconStyle},
    theme::look,
};

/// Nav pill: 5 buttons of 32 + 4 gaps of 2 + 3px margins.
const PILL_W: f32 = 5.0 * 32.0 + 4.0 * 2.0 + 6.0;
const PILL_H: f32 = 38.0;
const GAP: f32 = 8.0;
const WIDTH: f32 = 232.0;

/// Rect of the nav pill, used to ignore its clicks when dismissing.
fn pill_rect(chrome: &Chrome) -> Rect {
    let p = chrome.nav_pill;
    Rect::from_min_max(Pos2::new(p.x, p.y - PILL_H), Pos2::new(p.x + PILL_W, p.y))
}

pub fn popover(app: &mut App, ctx: &Context, chrome: &Chrome) {
    if !app.ui.shell_facade.layers_open {
        return;
    }
    let pill = pill_rect(chrome);
    let pos = Pos2::new(pill.left(), pill.top() - GAP);
    let area = frame::floating(egui::Id::unique("layers-popover"), pos, Order::Foreground)
        .pivot(Align2::LEFT_BOTTOM)
        .show(ctx, |ui| {
            menu_frame(ctx).show(ui, |ui| contents(app, ui));
        });
    let outside = ctx.input(|i| {
        i.pointer.any_pressed()
            && i.pointer
                .interact_pos()
                .is_some_and(|p| !area.response.rect.contains(p) && !pill.contains(p))
    });
    if outside {
        app.ui.shell_facade.layers_open = false;
    }
}

fn contents(app: &mut App, ui: &mut Ui) {
    let f = &mut app.ui.shell_facade;
    ui.set_width(WIDTH);
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.add(MenuSection::new("Viewport layers"));
    for (i, name) in ["Grid", "Sketches", "Datums & origin"]
        .into_iter()
        .enumerate()
    {
        row(ui, name, None, Some(&mut f.layers_on[i]));
    }
    ui.add(MenuSection::new("From extensions"));
    row(
        ui,
        "Tool reach",
        Some("CNC Prep"),
        Some(&mut f.layers_on[3]),
    );
    if f.topology_on {
        row(
            ui,
            "Topology warnings",
            Some("Topology Check"),
            Some(&mut f.layers_on[4]),
        );
    } else if row(ui, "Topology warnings", Some("Topology Check · off"), None) {
        f.turn_on_topology();
    }
}

/// Layer row. With `eye` the whole row toggles it; otherwise a "Turn on" link is shown and
/// its click is returned.
fn row(ui: &mut Ui, label: &str, sub: Option<&str>, eye: Option<&mut bool>) -> bool {
    let l = look(ui.ctx());
    let t = &l.theme;
    let h = if sub.is_some() { 38.0 } else { 30.0 };
    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
    let hover = r.hovered() && eye.is_some();
    let p = ui.painter();
    if hover {
        p.rect_filled(rect, CornerRadius::same(6), t.well);
    }
    let g = p.layout_no_wrap(label.to_owned(), FontId::proportional(12.5), t.ink2);
    let s = sub.map(|s| p.layout_no_wrap(s.to_owned(), FontId::proportional(11.0), t.ink5));
    let block = g.size().y + s.as_ref().map_or(0.0, |s| 1.0 + s.size().y);
    let top = rect.center().y - block / 2.0;
    let gh = g.size().y;
    p.galley(Pos2::new(rect.left() + 8.0, top), g, t.ink2);
    if let Some(s) = s {
        p.galley(Pos2::new(rect.left() + 8.0, top + gh + 1.0), s, t.ink5);
    }
    let mut link = false;
    match eye {
        Some(on) => {
            if r.clicked() {
                *on = !*on;
            }
            let name = if *on { "eyeOn" } else { "eyeOff" };
            let tint = if *on { t.ink3 } else { t.ink6 };
            let at = Rect::from_center_size(
                Pos2::new(rect.right() - 8.0 - 8.0, rect.center().y),
                vec2(16.0, 16.0),
            );
            let style = IconStyle {
                tint,
                bg: t.floating,
                grey: false,
            };
            icon::paint(ui.painter(), at, name, l.dark, style, 1.0);
        }
        None => link = turn_on_link(ui, rect, t.accent),
    }
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    link
}

fn turn_on_link(ui: &mut Ui, row: Rect, colour: Color32) -> bool {
    let g = ui.painter().layout_no_wrap(
        "Turn on".to_owned(),
        FontId::new(12.0, fonts::medium()),
        colour,
    );
    let rect = Rect::from_center_size(
        Pos2::new(row.right() - 8.0 - g.size().x / 2.0, row.center().y),
        g.size() + vec2(8.0, 8.0),
    );
    let r = ui.interact(rect, ui.scope_id().with("turn-on"), Sense::click());
    ui.painter()
        .galley(rect.center() - g.size() / 2.0, g, colour);
    r.clicked()
}
