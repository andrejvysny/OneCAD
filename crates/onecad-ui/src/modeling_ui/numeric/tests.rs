use super::*;

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    onecad_widgets::theme::install(&ctx, onecad_widgets::theme::load(false).unwrap(), false);
    text_session::enable(&ctx);
    ctx
}

fn frame(
    ctx: &egui::Context,
    owner: &str,
    value: &mut f64,
    range: Option<std::ops::RangeInclusive<u32>>,
    focus: bool,
) -> Fields {
    text_session::begin_frame(ctx, owner.into());
    let mut fields = Fields::new(owner);
    ctx.run_ui(egui::RawInput::default(), |ui| {
        let salt = (fields.owner, "value");
        let id = ui.make_persistent_id(egui::IdSalt::new(salt));
        ui.memory_mut(|m| {
            if focus {
                m.request_focus(id);
            } else {
                m.surrender_focus(id);
            }
        });
        if let Some(range) = range.clone() {
            let mut count = *value as u32;
            fields.count(ui, "value", "Option value", &mut count, range);
            *value = f64::from(count);
        } else {
            fields.value(ui, "value", "Option value", value);
        }
    })
    .textures_delta
    .clear();
    text_session::end_frame(ctx);
    fields
}

fn replace(ctx: &egui::Context, text: &str) {
    let snapshot = text_session::snapshot(ctx).expect("focused numeric option publishes a session");
    assert!(text_session::apply(
        ctx,
        text_session::Operation {
            generation: snapshot.generation,
            revision: snapshot.revision,
            sequence: snapshot.sequence + 1,
            edit: text_session::Edit::Replace(
                text_session::Range {
                    start: 0,
                    end: snapshot.text.encode_utf16().count() as u32,
                },
                text.into()
            ),
        }
    ));
}

fn edit(ctx: &egui::Context, edit: text_session::Edit) {
    let snapshot = text_session::snapshot(ctx).unwrap();
    assert!(text_session::apply(
        ctx,
        text_session::Operation {
            generation: snapshot.generation,
            revision: snapshot.revision,
            sequence: snapshot.sequence + 1,
            edit,
        }
    ));
}

#[test]
fn native_count_vector_and_gear_fields_publish_exact_drafts() {
    for (value, range) in [
        (3.0, Some(2..=128)),
        (20.0, Some(3..=400)),
        (50.00019918212638, None),
    ] {
        let ctx = context();
        let mut bound = value;
        let fields = frame(&ctx, "runtime-a/gesture-1", &mut bound, range, true);
        assert!(!fields.changed);
        assert_eq!(bound, value);
        let snapshot = text_session::snapshot(&ctx).expect("native keyboard needs a focused draft");
        assert_eq!(snapshot.text, value.to_string());
        assert_eq!(snapshot.label, "Option value");
    }
}

#[test]
fn invalid_count_text_survives_frames_and_same_value_correction_clears_error() {
    for invalid in ["", "-", "2.5", "1", "129", "NaN", "inf", "1e999", "=3+2"] {
        let ctx = context();
        let mut value = 3.0;
        frame(&ctx, "owner", &mut value, Some(2..=128), true);
        replace(&ctx, invalid);
        let edited = frame(&ctx, "owner", &mut value, Some(2..=128), true);
        assert!(edited.changed, "{invalid}");
        assert!(edited.error.is_some(), "{invalid}");
        assert_eq!(value, 3.0);
        let idle = frame(&ctx, "owner", &mut value, Some(2..=128), true);
        assert!(!idle.changed);
        assert!(idle.error.is_some());
        assert_eq!(text_session::snapshot(&ctx).unwrap().text, invalid);
        replace(&ctx, "3");
        let corrected = frame(&ctx, "owner", &mut value, Some(2..=128), true);
        assert!(corrected.changed);
        assert!(corrected.error.is_none());
        assert_eq!(value, 3.0);
    }
}

#[test]
fn vector_precision_survives_native_edit_focus_and_blur() {
    let ctx = context();
    let mut value = 0.0;
    frame(&ctx, "owner", &mut value, None, true);
    replace(&ctx, "50.00019918212638");
    assert!(frame(&ctx, "owner", &mut value, None, true).changed);
    let exact = value;
    assert_eq!(value, 50.00019918212638);
    frame(&ctx, "owner", &mut value, None, false);
    assert!(text_session::snapshot(&ctx).is_none());
    assert!(!frame(&ctx, "owner", &mut value, None, true).changed);
    assert_eq!(value, exact);
    assert_eq!(
        text_session::snapshot(&ctx).unwrap().text,
        "50.00019918212638"
    );
}

#[test]
fn invalid_count_draft_survives_blur_and_refocus() {
    let ctx = context();
    let mut value = 3.0;
    frame(&ctx, "owner", &mut value, Some(2..=128), true);
    replace(&ctx, "2.5");
    frame(&ctx, "owner", &mut value, Some(2..=128), true);
    let blurred = frame(&ctx, "owner", &mut value, Some(2..=128), false);
    assert!(blurred.error.is_some());
    assert_eq!(value, 3.0);
    let refocused = frame(&ctx, "owner", &mut value, Some(2..=128), true);
    assert!(refocused.error.is_some());
    assert_eq!(text_session::snapshot(&ctx).unwrap().text, "2.5");
}

#[test]
fn external_value_and_new_owner_supersede_pending_native_edits() {
    let ctx = context();
    let mut value = 3.0;
    frame(&ctx, "owner-a", &mut value, None, true);
    replace(&ctx, "99");
    value = 7.0;
    assert!(frame(&ctx, "owner-a", &mut value, None, true).changed);
    assert_eq!(value, 7.0);
    assert_eq!(text_session::snapshot(&ctx).unwrap().text, "7");
    replace(&ctx, "99");
    frame(&ctx, "owner-b", &mut value, None, true);
    assert_eq!(value, 7.0);
    assert_eq!(text_session::snapshot(&ctx).unwrap().text, "7");
}

#[test]
fn valid_range_endpoints_and_vector_finite_validation() {
    for value in ["2", "128"] {
        assert!(validation(value, Some(&(2..=128))).is_ok());
    }
    for value in ["3", "400"] {
        assert!(validation(value, Some(&(3..=400))).is_ok());
    }
    for invalid in ["NaN", "inf", "-inf", "1e999", "-"] {
        assert!(validation(invalid, None).is_err());
    }
    assert_eq!(
        validation("−0.123456789012345", None).unwrap(),
        -0.123456789012345
    );
}

#[test]
fn native_count_composition_cancel_and_owner_return_restore_the_original_draft() {
    for retire_owner in [false, true] {
        let ctx = context();
        let mut value = 3.0;
        frame(&ctx, "owner", &mut value, Some(2..=128), true);
        edit(
            &ctx,
            text_session::Edit::Select(text_session::Range { start: 0, end: 1 }),
        );
        edit(
            &ctx,
            text_session::Edit::Mark("12".into(), text_session::Range::caret(2)),
        );
        assert!(frame(&ctx, "owner", &mut value, Some(2..=128), true).changed);
        assert_eq!(value, 12.0);
        edit(
            &ctx,
            text_session::Edit::Mark("-".into(), text_session::Range::caret(1)),
        );
        assert!(
            frame(&ctx, "owner", &mut value, Some(2..=128), true)
                .error
                .is_some()
        );
        if retire_owner {
            text_session::begin_frame(&ctx, "other-owner".into());
            text_session::end_frame(&ctx);
        } else {
            edit(&ctx, text_session::Edit::Cancel);
        }
        let restored = frame(&ctx, "owner", &mut value, Some(2..=128), true);
        assert!(restored.changed);
        assert!(restored.error.is_none());
        assert_eq!(value, 3.0);
        assert_eq!(text_session::snapshot(&ctx).unwrap().text, "3");
    }
}

#[test]
fn invalid_sibling_remains_authoritative_when_a_valid_option_changes() {
    let ctx = context();
    let mut values = [3.0, 0.123456789012345];
    let mut draw = |focus: usize| {
        text_session::begin_frame(&ctx, "options".into());
        let mut fields = Fields::new("group");
        ctx.run_ui(egui::RawInput::default(), |ui| {
            let salt = (fields.owner, if focus == 0 { "count" } else { "vector" });
            let id = ui.make_persistent_id(egui::IdSalt::new(salt));
            ui.memory_mut(|m| m.request_focus(id));
            let mut count = values[0] as u32;
            fields.count(ui, "count", "Instances", &mut count, 2..=128);
            values[0] = f64::from(count);
            fields.value(ui, "vector", "Direction X", &mut values[1]);
        })
        .textures_delta
        .clear();
        text_session::end_frame(&ctx);
        fields
    };
    draw(0);
    replace(&ctx, "2.5");
    assert!(draw(0).error.is_some());
    assert!(draw(1).error.is_some());
    replace(&ctx, "0.987654321098765");
    let changed = draw(1);
    assert!(changed.changed);
    assert!(changed.error.is_some());
    assert!(draw(0).error.is_some());
    assert_eq!(text_session::snapshot(&ctx).unwrap().text, "2.5");
    replace(&ctx, "3");
    let corrected = draw(0);
    assert!(corrected.changed);
    assert!(corrected.error.is_none());
    assert_eq!(values, [3.0, 0.987654321098765]);
}
