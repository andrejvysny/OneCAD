use egui::{Context, Event, Id, Key, Modifiers, Pos2, RawInput, Rect, vec2};

use super::{SwitcherItem, TabCloseOutcome, TabClosePopover, TabSwitcher};
use crate::theme;

fn key(k: Key) -> Event {
    Event::Key {
        key: k,
        physical_key: Some(k),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}

/// Runs the popover for a few frames with `events` on frame 1 and ORs the outcomes.
fn drive(events: Vec<Event>, saving: bool) -> TabCloseOutcome {
    let ctx = Context::default();
    theme::install(&ctx, theme::load(false).unwrap(), false);
    let anchor = Rect::from_min_size(Pos2::new(300.0, 10.0), vec2(150.0, 28.0));
    let mut got = TabCloseOutcome::default();
    for n in 0..3 {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
            events: if n == 1 { events.clone() } else { Vec::new() },
            ..Default::default()
        };
        let mut frame = ctx.run_ui(input, |ui| {
            let o = TabClosePopover::new("Bracket", anchor)
                .body(Some("3 changes since last save."))
                .saving(saving)
                .show(ui.ctx(), Id::unique("p"));
            got.save |= o.save;
            got.discard |= o.discard;
            got.cancel |= o.cancel;
        });
        frame.textures_delta.clear();
    }
    got
}

#[test]
fn escape_cancels_and_enter_saves() {
    let esc = drive(vec![key(Key::Escape)], false);
    assert!(esc.cancel && !esc.save && !esc.discard);
    let enter = drive(vec![key(Key::Enter)], false);
    assert!(enter.save && !enter.cancel && !enter.discard);
}

#[test]
fn save_is_disabled_while_saving() {
    assert!(!drive(vec![key(Key::Enter)], true).save);
    // Escape still backs out.
    assert!(drive(vec![key(Key::Escape)], true).cancel);
}

#[test]
fn switcher_renders_and_wraps_inside_the_screen() {
    let ctx = Context::default();
    theme::install(&ctx, theme::load(false).unwrap(), false);
    let items: Vec<SwitcherItem> = (0..5)
        .map(|i| SwitcherItem {
            title: "Document",
            icon: "body",
            dirty: i == 2,
            thumbnail: None,
            home: i == 4,
        })
        .collect();
    for width in [1200.0, 500.0] {
        let mut rect = Rect::NOTHING;
        for _ in 0..2 {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 700.0))),
                ..Default::default()
            };
            let mut frame = ctx.run_ui(input, |ui| {
                rect = TabSwitcher::new(&items, 1)
                    .show(ui.ctx(), Id::unique("s"))
                    .rect;
            });
            frame.textures_delta.clear();
        }
        assert!(rect.width() <= width * 0.9 + 1.0, "{rect:?}");
    }
}
