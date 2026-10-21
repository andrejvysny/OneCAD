use egui::epaint::Shape;
use egui::{Event, Pos2, Rect, vec2};

use super::{TabSpec, TabStrip, TitleBar, TitleBarOutput};
use crate::theme;

fn spec(id: u64, title: &str, active: bool, dirty: bool) -> TabSpec<'_> {
    TabSpec {
        id,
        title,
        icon: "body",
        dirty,
        active,
        busy: false,
    }
}

fn ctx() -> egui::Context {
    let ctx = egui::Context::default();
    theme::install(&ctx, theme::load(false).unwrap(), false);
    ctx
}

/// One frame of a `width`-wide bar; returns the output and the shapes it painted.
fn bar(
    ctx: &egui::Context,
    width: f32,
    events: Vec<Event>,
    tabs: &[TabSpec<'_>],
    home_active: bool,
) -> (TitleBarOutput, Vec<Shape>) {
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 200.0))),
        events,
        ..Default::default()
    };
    let mut out = None;
    let mut frame = ctx.run_ui(input, |ui| {
        let strip = TabStrip {
            tabs,
            home_active,
            can_add: true,
        };
        out = Some(TitleBar::new("x").tabs(Some(strip)).show(ui));
    });
    frame.textures_delta.clear();
    let shapes = frame.shapes.into_iter().map(|s| s.shape).collect();
    (out.unwrap(), shapes)
}

/// Move, press, release at `target`; returns (activate, close) seen over the frames.
fn click(ctx: &egui::Context, target: Pos2, tabs: &[TabSpec<'_>]) -> (Option<u64>, Option<u64>) {
    let button = |pressed| Event::PointerButton {
        pos: target,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let steps = [
        vec![Event::PointerMoved(target)],
        vec![button(true)],
        vec![button(false)],
    ];
    let (mut act, mut close) = (None, None);
    for events in steps {
        let (o, _) = bar(ctx, 1000.0, events, tabs, false);
        act = act.or(o.activate);
        close = close.or(o.close);
    }
    (act, close)
}

fn three() -> [TabSpec<'static>; 3] {
    [
        spec(1, "Bracket", false, false),
        spec(2, "Gearbox", true, false),
        spec(3, "Housing", false, true),
    ]
}

#[test]
fn tab_rects_one_per_tab_inside_bar() {
    let ctx = ctx();
    let tabs = three();
    let (o, _) = bar(&ctx, 1000.0, vec![], &tabs, false);
    let bar_rect = Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 44.0));
    assert_eq!(o.tab_rects.len(), 3);
    for ((id, r), t) in o.tab_rects.iter().zip(&tabs) {
        assert_eq!(*id, t.id);
        assert!(bar_rect.contains_rect(*r), "{r:?}");
        assert_eq!(r.height(), 28.0);
    }
}

#[test]
fn clicking_tab_centre_activates_and_close_slot_closes() {
    let ctx = ctx();
    let tabs = three();
    let (o, _) = bar(&ctx, 1000.0, vec![], &tabs, false);
    let first = o.tab_rects[0].1;
    assert_eq!(click(&ctx, first.center(), &tabs), (Some(1), None));
    // The active tab always shows its ×, in the 16 px slot right of the title.
    let active = o.tab_rects[1].1;
    let slot = Pos2::new(active.right() - 10.0 - 8.0, active.center().y);
    assert_eq!(click(&ctx, slot, &tabs), (None, Some(2)));
}

#[test]
fn no_tab_is_highlighted_when_home_is_active() {
    let ctx = ctx();
    let tabs = three();
    let surface = theme::load(false).unwrap().surface;
    let raised = |home_active| {
        let (o, shapes) = bar(&ctx, 1000.0, vec![], &tabs, home_active);
        let active = o.tab_rects[1].1;
        shapes
            .iter()
            .any(|s| matches!(s, Shape::Rect(r) if r.fill == surface && r.rect == active))
    };
    assert!(raised(false));
    assert!(!raised(true));
}

#[test]
fn many_long_titles_keep_minimum_width() {
    let ctx = ctx();
    let title = "A very long document title that cannot possibly fit";
    let tabs: Vec<TabSpec> = (0..8).map(|i| spec(i, title, i == 0, false)).collect();
    let (o, _) = bar(&ctx, 1000.0, vec![], &tabs, false);
    assert_eq!(o.tab_rects.len(), 8);
    for (_, r) in &o.tab_rects {
        assert!(r.width() >= 96.0, "{r:?}");
    }
}
