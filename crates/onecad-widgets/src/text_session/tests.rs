use super::*;

fn active(text: &str) -> Active {
    Active {
        snapshot: Snapshot {
            generation: 4,
            revision: 1,
            sequence: 0,
            field: Id::unique("field"),
            text: text.into(),
            selection: Range::caret(text.encode_utf16().count() as u32),
            marked: None,
            label: "Length mm".into(),
            rect: egui::Rect::NOTHING,
            caret: egui::Rect::NOTHING,
            enabled: true,
        },
        base: text.into(),
        pending: false,
        max_chars: 512,
        composition: None,
        key: None,
    }
}
fn apply_edit(a: &mut Active, edit: Edit) -> bool {
    a.apply(Operation {
        generation: a.snapshot.generation,
        revision: a.snapshot.revision,
        sequence: a.snapshot.sequence + 1,
        edit,
    })
}

#[test]
fn utf16_middle_expression_replacement_preserves_authored_units_unicode() {
    let mut a = active("=幅😀 + 2mm + 90°");
    assert!(apply_edit(
        &mut a,
        Edit::Replace(Range { start: 7, end: 10 }, "3in".into())
    ));
    assert_eq!(a.snapshot.text, "=幅😀 + 3in + 90°");
    let old = a.snapshot.clone();
    assert!(!apply_edit(
        &mut a,
        Edit::Replace(Range { start: 3, end: 4 }, "x".into())
    ));
    assert_eq!(a.snapshot, old, "surrogate interior must not mutate draft");
    assert!(!apply_edit(
        &mut a,
        Edit::Select(Range { start: 90, end: 91 })
    ));
}

#[test]
fn callbacks_acknowledge_revisions_and_reject_duplicate_stale_focus_and_limits() {
    let mut a = active("12mm");
    let first = Operation {
        generation: 4,
        revision: 1,
        sequence: 1,
        edit: Edit::Replace(Range { start: 0, end: 2 }, "30".into()),
    };
    assert!(a.apply(first.clone()));
    assert!(!a.apply(first));
    assert!(apply_edit(
        &mut a,
        Edit::Replace(Range { start: 2, end: 4 }, "°".into())
    ));
    assert_eq!(a.snapshot.text, "30°");
    assert!(!a.apply(Operation {
        generation: 3,
        revision: 3,
        sequence: 3,
        edit: Edit::Unmark
    }));
    let snapshot = a.snapshot.clone();
    assert!(!apply_edit(
        &mut a,
        Edit::Replace(Range::caret(0), "x".repeat(513))
    ));
    assert_eq!(a.snapshot, snapshot);
}

#[test]
fn composition_replaces_mark_once_unmark_is_idempotent_cancel_restores_base() {
    let mut a = active("a + 2mm");
    assert!(apply_edit(&mut a, Edit::Select(Range { start: 0, end: 1 })));
    assert!(apply_edit(
        &mut a,
        Edit::Mark("幅😀".into(), Range::caret(3))
    ));
    assert_eq!(a.snapshot.text, "幅😀 + 2mm");
    assert!(apply_edit(
        &mut a,
        Edit::Mark("幅度".into(), Range { start: 1, end: 2 })
    ));
    assert_eq!(a.snapshot.text, "幅度 + 2mm");
    assert!(apply_edit(&mut a, Edit::Cancel));
    assert_eq!(a.snapshot.text, "a + 2mm");
    assert_eq!(a.snapshot.selection, Range { start: 0, end: 1 });
    assert!(apply_edit(&mut a, Edit::Mark("長".into(), Range::caret(1))));
    assert!(apply_edit(&mut a, Edit::Unmark));
    assert!(apply_edit(&mut a, Edit::Unmark));
    let selection = a.snapshot.selection;
    assert!(apply_edit(&mut a, Edit::Replace(selection, "長".into())));
    assert_eq!(
        a.snapshot.text, "長長 + 2mm",
        "insertion after unmark is genuine input"
    );
}

fn field_frame(ctx: &Context, text: &mut String, events: Vec<egui::Event>, visible: bool) -> bool {
    begin_frame(ctx, "tab:1/runtime:a/gesture:1".into());
    let mut changed = false;
    let mut out = ctx.run_ui(
        egui::RawInput {
            focused: true,
            events,
            ..Default::default()
        },
        |ui| {
            if visible {
                let id = Id::unique("live-expression");
                ui.memory_mut(|m| m.request_focus(id));
                let response = show(ui, id, text, "Length mm", 512, |text| {
                    TextEdit::singleline(text).return_key(None)
                });
                changed = response.changed();
            }
        },
    );
    out.textures_delta.clear();
    end_frame(ctx);
    changed
}

#[test]
fn actual_egui_field_applies_middle_edit_undo_and_confirm_once() {
    let ctx = Context::default();
    enable(&ctx);
    let mut text = "=width * 2mm".to_owned();
    field_frame(&ctx, &mut text, vec![], true);
    let original = snapshot(&ctx).unwrap();
    let operation = Operation {
        generation: original.generation,
        revision: original.revision,
        sequence: 1,
        edit: Edit::Replace(Range { start: 9, end: 10 }, "3".into()),
    };
    assert!(apply(&ctx, operation));
    assert!(field_frame(&ctx, &mut text, vec![], true));
    assert_eq!(text, "=width * 3mm");
    let undo = egui::Event::Key {
        key: egui::Key::Z,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers {
            command: true,
            mac_cmd: true,
            ..Default::default()
        },
    };
    field_frame(&ctx, &mut text, vec![undo], true);
    assert_eq!(text, "=width * 2mm");
    let s = snapshot(&ctx).unwrap();
    let confirm = Operation {
        generation: s.generation,
        revision: s.revision,
        sequence: s.sequence + 1,
        edit: Edit::Confirm(false),
    };
    assert!(apply(&ctx, confirm.clone()));
    assert!(!apply(&ctx, confirm));
    field_frame(&ctx, &mut text, vec![], true);
    assert!(ctx.input(|i| i.key_pressed(egui::Key::Enter)));
    field_frame(&ctx, &mut text, vec![], true);
    assert!(!ctx.input(|i| i.key_pressed(egui::Key::Enter)));
}

#[test]
fn actual_field_external_change_missing_field_scope_reject_pending_native_draft() {
    let ctx = Context::default();
    enable(&ctx);
    let mut text = "20mm".into();
    field_frame(&ctx, &mut text, vec![], true);
    let s = snapshot(&ctx).unwrap();
    assert!(apply(
        &ctx,
        Operation {
            generation: s.generation,
            revision: s.revision,
            sequence: 1,
            edit: Edit::Replace(Range { start: 0, end: 2 }, "99".into())
        }
    ));
    text = "10mm".into(); // external undo supersedes unpublished UIKit edit
    field_frame(&ctx, &mut text, vec![], true);
    assert_eq!(text, "10mm");
    assert_ne!(snapshot(&ctx).unwrap().generation, s.generation);
    field_frame(&ctx, &mut text, vec![], false);
    assert!(snapshot(&ctx).is_none());
    field_frame(&ctx, &mut text, vec![], true);
    begin_frame(&ctx, "tab:2/runtime:b".into());
    assert!(snapshot(&ctx).is_none());
}

#[test]
fn removed_or_switched_owner_cancels_composition_when_its_draft_returns() {
    let ctx = Context::default();
    enable(&ctx);
    let mut text = "2mm".into();
    field_frame(&ctx, &mut text, vec![], true);
    let s = snapshot(&ctx).unwrap();
    assert!(apply(
        &ctx,
        Operation {
            generation: s.generation,
            revision: s.revision,
            sequence: 1,
            edit: Edit::Mark("長".into(), Range::caret(1))
        }
    ));
    field_frame(&ctx, &mut text, vec![], true);
    assert_eq!(text, "2mm長");
    field_frame(&ctx, &mut text, vec![], false);
    assert!(snapshot(&ctx).is_none());
    assert!(field_frame(&ctx, &mut text, vec![], true));
    assert_eq!(text, "2mm");
    let s = snapshot(&ctx).unwrap();
    assert!(apply(
        &ctx,
        Operation {
            generation: s.generation,
            revision: s.revision,
            sequence: s.sequence + 1,
            edit: Edit::Mark("幅".into(), Range::caret(1))
        }
    ));
    field_frame(&ctx, &mut text, vec![], true);
    begin_frame(&ctx, "other-tab".into());
    end_frame(&ctx);
    field_frame(&ctx, &mut text, vec![], true);
    assert_eq!(text, "2mm");
}

#[test]
fn number_field_snapshot_keeps_full_precision_and_actual_native_replacement() {
    use crate::components::inputs::NumberField;
    let ctx = Context::default();
    crate::theme::install(&ctx, crate::theme::load(false).unwrap(), false);
    enable(&ctx);
    let mut text = "0.123456789".to_owned();
    let mut draw = |request_focus: bool| {
        begin_frame(&ctx, "document1/operation1".into());
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let id = ui.make_persistent_id(egui::IdSalt::new("precise-distance"));
            if request_focus {
                ui.memory_mut(|m| m.request_focus(id));
            }
            ui.add(NumberField::new("precise-distance", &mut text).decimals(2));
        });
        output.textures_delta.clear();
        end_frame(&ctx);
    };
    draw(true);
    let s = snapshot(&ctx).unwrap();
    assert_eq!(s.text, "0.123456789");
    assert!(apply(
        &ctx,
        Operation {
            generation: s.generation,
            revision: s.revision,
            sequence: 1,
            edit: Edit::Replace(Range { start: 2, end: 5 }, "999".into())
        }
    ));
    draw(false);
    assert_eq!(text, "0.999456789");
}

#[test]
fn native_tab_commits_composition_and_traverses_real_fields_once() {
    let ctx = Context::default();
    enable(&ctx);
    let a = Id::unique("tab-first");
    let b = Id::unique("tab-second");
    let mut first = "20mm".to_owned();
    let mut second = "90°".to_owned();
    let mut render = |focus| {
        begin_frame(&ctx, "same-owner".into());
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            if focus {
                ui.memory_mut(|m| m.request_focus(a));
            }
            show(ui, a, &mut first, "Length", 512, |text| {
                TextEdit::singleline(text)
            });
            show(ui, b, &mut second, "Angle", 512, |text| {
                TextEdit::singleline(text)
            });
        });
        output.textures_delta.clear();
        end_frame(&ctx);
    };
    render(true);
    let s = snapshot(&ctx).unwrap();
    assert!(apply(
        &ctx,
        Operation {
            generation: s.generation,
            revision: s.revision,
            sequence: 1,
            edit: Edit::Confirm(true)
        }
    ));
    render(false);
    assert_eq!(ctx.memory(|m| m.focused()), Some(b));
    assert_eq!(native_tab_origin(&ctx), Some(a));
    render(false);
    assert_eq!(ctx.memory(|m| m.focused()), Some(b));
    assert_eq!(native_tab_origin(&ctx), None);
}
