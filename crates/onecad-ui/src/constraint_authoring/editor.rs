//! The field belongs to a sketch visit and constraint, independently of camera captures.
use super::*;
use onecad_interaction::sketch::{SketchPhase, SketchStatus};

pub(super) enum Settlement {
    Waiting,
    Complete,
    Retry,
}

pub(super) fn settle(draft: &mut Draft, session: &SketchSession) -> Settlement {
    if draft.pending.as_ref() != Some(&session.request) {
        return Settlement::Complete;
    }
    if matches!(
        session.phase,
        SketchPhase::Submitting | SketchPhase::Accepted
    ) {
        return Settlement::Waiting;
    }
    match &session.status {
        SketchStatus::Refused(message) | SketchStatus::Failed { message, .. } => {
            draft.error = Some(message.clone());
            draft.pending = None;
            draft.document = session.document.clone();
            Settlement::Retry
        }
        _ => Settlement::Complete,
    }
}

pub(super) fn editor(
    ctx: &egui::Context,
    draft: &mut Draft,
    variables: &[VariableDto],
) -> (bool, bool) {
    let Some(relation) = Relation::from_constraint(&draft.constraint) else {
        return (false, true);
    };
    let safe = draft
        .viewport
        .unwrap_or_else(|| ctx.content_rect())
        .shrink(8.0);
    let size = egui::vec2(300.0_f32.min(safe.width()), 164.0_f32.min(safe.height()));
    let position = draft.position.unwrap_or_else(|| safe.center() - size * 0.5);
    let position = egui::pos2(
        position
            .x
            .clamp(safe.left(), (safe.right() - size.x).max(safe.left())),
        position
            .y
            .clamp(safe.top(), (safe.bottom() - size.y).max(safe.top())),
    );
    draft.position = Some(position);
    let mut result = (false, false);
    let id = editor_id().with((
        &draft.key.runtime_session,
        draft.key.sketch_id,
        draft.key.generation,
        draft.constraint.id(),
    ));
    egui::Area::new(id)
        .order(egui::Order::Foreground)
        .fixed_pos(position)
        .movable(false)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width((size.x - 16.0).max(1.0));
                result = field(ui, draft, relation, variables);
            });
        });
    result
}

fn field(
    ui: &mut egui::Ui,
    draft: &mut Draft,
    relation: Relation,
    variables: &[VariableDto],
) -> (bool, bool) {
    let pending = draft.pending.is_some();
    ui.strong(format!(
        "{} ({})",
        relation.label(),
        if relation == Relation::Angle {
            "°"
        } else {
            "mm"
        }
    ));
    let response = ui.add_enabled(
        !pending,
        egui::TextEdit::singleline(&mut draft.text)
            .desired_width(ui.available_width())
            .char_limit(onecad_core::expr::MAX_SOURCE_LEN),
    );
    if draft.focus && !pending {
        response.request_focus();
        draft.focus = false;
    }
    if response.changed() {
        draft.error = None;
    }
    let parsed = dimensions::scalar(&draft.text, relation, variables);
    if let Some(error) = draft.error.as_ref().or_else(|| parsed.as_ref().err()) {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
    if pending {
        ui.label("Applying dimension…");
    }
    let enter = !pending
        && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    let escape = ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    let mut apply = false;
    let mut cancel = escape;
    ui.horizontal(|ui| {
        apply = ui
            .add_enabled(!pending && parsed.is_ok(), egui::Button::new("Apply"))
            .clicked();
        cancel |= ui
            .button(if pending { "Dismiss" } else { "Cancel" })
            .clicked();
    });
    (parsed.is_ok() && (apply || enter), cancel)
}
