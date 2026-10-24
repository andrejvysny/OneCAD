//! Real egui clipping must not change the native editor's keyboard ownership.
use super::*;
use crate::ffi;
use onecad_widgets::{components::inputs::NumberField, text_session};

struct Editor {
    ctx: egui::Context,
    text: String,
    field: Option<egui::Id>,
    numeric: bool,
}

impl Editor {
    fn new(numeric: bool) -> Self {
        let ctx = egui::Context::default();
        onecad_widgets::theme::install(&ctx, onecad_widgets::theme::load(false).unwrap(), false);
        text_session::enable(&ctx);
        Self {
            ctx,
            text: "10".into(),
            field: None,
            numeric,
        }
    }

    fn frame(&mut self, offset: f32, show_field: bool) -> (egui::PlatformOutput, ffi::FrameResult) {
        text_session::begin_frame(&self.ctx, "same-live-editor".into());
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 300.0),
                )),
                focused: true,
                ..Default::default()
            },
            |ui| {
                egui::ScrollArea::vertical()
                    .max_height(96.0)
                    .vertical_scroll_offset(offset)
                    .show(ui, |ui| {
                        if show_field {
                            self.field = Some(
                                ui.add(
                                    NumberField::new("distance", &mut self.text)
                                        .label("Distance (mm)")
                                        .keep_focus_on_enter(true)
                                        .width(200.0),
                                )
                                .id,
                            );
                        }
                        ui.allocate_exact_size(egui::vec2(200.0, 1000.0), egui::Sense::hover());
                    });
            },
        );
        text_session::end_frame(&self.ctx);
        output.textures_delta.clear();
        let mut result = ffi::FrameResult::default();
        text_input_output(
            &mut result,
            &self.ctx,
            &output.platform_output,
            self.numeric,
        );
        (output.platform_output, result)
    }

    fn focus(&mut self) {
        self.frame(0.0, true);
        self.ctx
            .memory_mut(|memory| memory.request_focus(self.field.unwrap()));
    }
}

fn active(result: &ffi::FrameResult) -> bool {
    result.flags & ffi::FRAME_TEXT_INPUT_ACTIVE != 0
}

#[test]
fn clipped_focused_number_field_keeps_native_keyboard_owner_and_snapshot_caret() {
    for numeric in [false, true] {
        let mut editor = Editor::new(numeric);
        editor.focus();
        let (visible, shown) = editor.frame(0.0, true);
        assert!(visible.ime.is_some() && active(&shown));
        let owner = text_session::snapshot(&editor.ctx).unwrap();
        let (clipped, result) = editor.frame(300.0, true);
        let snapshot = text_session::snapshot(&editor.ctx).unwrap();
        assert!(
            clipped.ime.is_none(),
            "test must exercise egui's visible-only IME output"
        );
        assert_eq!(snapshot.generation, owner.generation);
        assert_eq!(snapshot.field, owner.field);
        assert_eq!(snapshot.text, owner.text);
        assert!(
            snapshot.rect.max.y < 0.0,
            "focused field must be completely offscreen"
        );
        assert!(
            active(&result),
            "clipping must not dismiss a live native text owner"
        );
        assert_eq!(result.text_cursor_x, snapshot.caret.min.x);
        assert_eq!(result.text_cursor_y, snapshot.caret.min.y);
        assert_eq!(result.text_cursor_h, snapshot.caret.height());
        assert_eq!(
            result.text_input_kind,
            if numeric {
                ffi::TEXT_NUMERIC_EXPRESSION
            } else {
                ffi::TEXT_GENERAL
            }
        );
    }
}

#[test]
fn focus_loss_or_removed_editor_ends_keyboard_ownership_instead_of_retaining_old_ime() {
    for removed in [false, true] {
        let mut editor = Editor::new(true);
        editor.focus();
        let (old_platform, _) = editor.frame(0.0, true);
        assert!(old_platform.ime.is_some());
        editor.frame(300.0, true);
        if !removed {
            editor
                .ctx
                .memory_mut(|memory| memory.surrender_focus(editor.field.unwrap()));
        }
        let (_, result) = editor.frame(300.0, !removed);
        assert!(text_session::snapshot(&editor.ctx).is_none());
        assert!(!active(&result));
        let mut stale = ffi::FrameResult::default();
        text_input_output(&mut stale, &editor.ctx, &old_platform, true);
        assert!(
            !active(&stale),
            "native ownership must supersede even an older visible IME output"
        );
    }
}

#[test]
fn unadapted_legacy_text_edit_keeps_platform_ime_fallback_and_general_kind() {
    let ctx = egui::Context::default();
    let mut text = "name".to_owned();
    let field = egui::Id::unique("legacy-unadapted-editor");
    let mut render = |focus| {
        let mut output = ctx.run_ui(Default::default(), |ui| {
            if focus {
                ui.memory_mut(|memory| memory.request_focus(field));
            }
            ui.add(egui::TextEdit::singleline(&mut text).id(field));
        });
        output.textures_delta.clear();
        output.platform_output
    };
    render(false);
    let platform = render(true);
    assert!(text_session::snapshot(&ctx).is_none());
    assert!(!text_session::enabled(&ctx));
    let ime = platform.ime.as_ref().unwrap();
    let mut result = ffi::FrameResult::default();
    text_input_output(&mut result, &ctx, &platform, false);
    assert!(active(&result));
    assert_eq!(result.text_input_kind, ffi::TEXT_GENERAL);
    assert_eq!(result.text_cursor_x, ime.cursor_rect.min.x);
    assert_eq!(result.text_cursor_y, ime.cursor_rect.min.y);
    assert_eq!(result.text_cursor_h, ime.cursor_rect.height());
}
