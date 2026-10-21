use egui::{Event, Pos2, RawInput};

use super::*;
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

/// Runs the dialog for a few frames and ORs outcomes (sizing passes may eat events).
fn drive(dark: bool, events: Vec<Event>, build: impl Fn() -> Dialog<'static>) -> DialogOutcome {
    let ctx = Context::default();
    theme::install(&ctx, theme::load(dark).unwrap(), dark);
    let mut got = DialogOutcome::default();
    for n in 0..3 {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
            events: if n == 1 { events.clone() } else { Vec::new() },
            ..Default::default()
        };
        let mut frame = ctx.run_ui(input, |ui| {
            let rows = [("body", "Body 1", "body"), ("sketch", "Sketch 1", "sketch")];
            let o = build().show(ui.ctx(), Id::unique("d"), |ui| {
                affected_list(ui, &rows);
                undo_hint(ui, "Undo restores this deletion.");
            });
            got.primary |= o.primary;
            got.secondary |= o.secondary;
            got.tertiary |= o.tertiary;
            got.dismissed |= o.dismissed;
            got.rect = o.rect;
        });
        frame.textures_delta.clear();
    }
    got
}

fn decide() -> Dialog<'static> {
    Dialog::new(DialogIntent::Decide, "Save changes before continuing?")
        .icon("save")
        .body("Save the current document, discard its unsaved changes, or keep editing.")
        .primary("Save", true)
        .secondary("Cancel", true)
        .tertiary("Discard")
}

#[test]
fn enter_triggers_primary() {
    let o = drive(false, vec![key(Key::Enter)], decide);
    assert!(o.primary && !o.secondary && !o.dismissed);
}

#[test]
fn enter_with_focus_secondary_triggers_cancel() {
    let o = drive(false, vec![key(Key::Enter)], || {
        Dialog::new(DialogIntent::Destroy, "Delete selected item?")
            .icon("delete")
            .primary("Delete", true)
            .secondary("Cancel", true)
            .focus_secondary(true)
    });
    assert!(o.secondary && !o.primary);
}

#[test]
fn disabled_primary_ignores_enter() {
    let o = drive(false, vec![key(Key::Enter)], || {
        Dialog::new(DialogIntent::Decide, "t")
            .primary("Save", false)
            .secondary("Cancel", true)
    });
    assert!(!o.primary && !o.secondary);
}

#[test]
fn escape_dismisses() {
    let o = drive(false, vec![key(Key::Escape)], decide);
    assert!(o.dismissed && !o.primary);
}

#[test]
fn idle_reports_nothing() {
    let o = drive(true, Vec::new(), decide);
    assert!(!o.primary && !o.secondary && !o.tertiary && !o.dismissed);
}

#[test]
fn fits_width_in_both_themes() {
    for dark in [false, true] {
        for intent in [
            DialogIntent::Decide,
            DialogIntent::Destroy,
            DialogIntent::Failure,
            DialogIntent::Input,
        ] {
            let o = drive(dark, Vec::new(), move || {
                Dialog::new(intent, "A rather long title that should wrap instead of overflow")
                    .icon("failed")
                    .body("Body text that is long enough to need wrapping across several lines of the card.")
                    .primary("Primary", true)
                    .secondary("Cancel", true)
                    .tertiary("Copy details")
            });
            assert!(
                o.rect.width() <= 420.5,
                "{intent:?} dark={dark} {:?}",
                o.rect
            );
            assert!(
                o.rect.width() >= 400.0,
                "{intent:?} dark={dark} {:?}",
                o.rect
            );
            let screen = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
            assert!(screen.contains_rect(o.rect), "{:?}", o.rect);
        }
    }
}
