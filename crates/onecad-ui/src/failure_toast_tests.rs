use super::*;
use crate::diagnostics::{FileAction, Source, State};
use egui::{Event, PointerButton, Pos2, Rect, pos2};

fn frame(ctx: &Context, d: &Diagnostics, time: f64, events: Vec<Event>) -> Vec<String> {
    let mut output = ctx.run_ui(
        egui::RawInput {
            time: Some(time),
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 560.0))),
            events,
            ..Default::default()
        },
        |_| show(ctx, d),
    );
    output.textures_delta.clear();
    output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
            _ => None,
        })
        .collect()
}

fn has(labels: &[String], needle: &str) -> bool {
    labels.iter().any(|label| label.contains(needle))
}

fn failed() -> Diagnostics {
    let mut d = Diagnostics::default();
    d.record(Source::File(FileAction::Save), State::Failed, "Disk full.");
    d
}

#[test]
fn success_and_progress_never_show_a_toast() {
    let ctx = Context::default();
    let mut d = Diagnostics::default();
    d.record(Source::File(FileAction::Save), State::Succeeded, "Saved");
    d.record(Source::Model, State::Pending, "Submitting feature…");
    d.record(Source::Sketch, State::Rejected, "conflict");
    for t in [0.0, 0.1] {
        let labels = frame(&ctx, &d, t, vec![]);
        assert!(
            !has(&labels, "Saved") && !has(&labels, "Copy"),
            "{labels:?}"
        );
    }
}

#[test]
fn failure_shows_then_expires_and_stays_gone() {
    let ctx = Context::default();
    let d = failed();
    frame(&ctx, &d, 0.0, vec![]);
    let labels = frame(&ctx, &d, 0.1, vec![]);
    assert!(has(&labels, "Save: Disk full."), "{labels:?}");
    assert!(has(&labels, "Copy"));
    assert!(has(&frame(&ctx, &d, 5.9, vec![]), "Disk full"));
    assert!(!has(&frame(&ctx, &d, 6.1, vec![]), "Disk full"));
    assert!(!has(&frame(&ctx, &d, 7.0, vec![]), "Disk full"));
}

#[test]
fn newer_failure_replaces_the_toast() {
    let ctx = Context::default();
    let mut d = failed();
    frame(&ctx, &d, 0.0, vec![]);
    d.record(Source::Worker, State::Failed, "Worker unavailable.");
    let labels = frame(&ctx, &d, 1.0, vec![]);
    assert!(has(&labels, "Worker unavailable") && !has(&labels, "Disk full"));
}

fn click(pos: Pos2) -> Vec<Event> {
    [true, false]
        .into_iter()
        .flat_map(|pressed| {
            [
                Event::PointerMoved(pos),
                Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        })
        .collect()
}

#[test]
fn clicking_the_toast_dismisses_it() {
    let ctx = Context::default();
    let d = failed();
    frame(&ctx, &d, 0.0, vec![]);
    frame(&ctx, &d, 0.1, vec![]);
    let area = ctx
        .memory(|m| m.area_rect(Id::unique("failure-toast")))
        .expect("toast area");
    let pos = pos2(area.left() + 6.0, area.top() + 6.0);
    frame(&ctx, &d, 0.2, click(pos));
    assert!(!has(&frame(&ctx, &d, 0.3, vec![]), "Disk full"));
}
