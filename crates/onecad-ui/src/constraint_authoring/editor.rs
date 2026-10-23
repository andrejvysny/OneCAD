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
    let size = egui::vec2(
        crate::dimension_chip::WIDTH.min(safe.width()),
        104.0_f32.min(safe.height()),
    );
    let anchor = draft.position.unwrap_or_else(|| safe.center());
    let position = crate::dimension_chip::place(anchor, size, safe);
    let mut result = (false, false);
    let id = editor_id().with((
        &draft.key.runtime_session,
        draft.key.sketch_id,
        draft.key.generation,
        draft.constraint.id(),
    ));
    let response = egui::Area::new(id)
        .order(egui::Order::Foreground)
        .fixed_pos(position)
        .default_size(size)
        .movable(false)
        .show(ctx, |ui| {
            crate::dimension_chip::frame(ui.style()).show(ui, |ui| {
                ui.set_width((size.x - 12.0).max(1.0));
                result = field(ui, draft, relation, variables);
            });
        });
    ctx.data_mut(|data| data.insert_temp(editor_id().with("bounds"), response.response.rect));
    crate::dimension_chip::witness(
        ctx,
        id.with("witness"),
        anchor,
        response.response.rect,
        safe,
    );
    result
}

fn field(
    ui: &mut egui::Ui,
    draft: &mut Draft,
    relation: Relation,
    variables: &[VariableDto],
) -> (bool, bool) {
    let pending = draft.pending.is_some();
    let unit = if relation == Relation::Angle {
        "°"
    } else {
        "mm"
    };
    let response = ui
        .horizontal(|ui| {
            ui.label(format!(
                "{} {unit}",
                match relation {
                    Relation::Diameter => "Ø",
                    Relation::Radius => "R",
                    Relation::HorizontalDistance => "H",
                    Relation::VerticalDistance => "V",
                    Relation::Angle => "Angle",
                    _ => "Distance",
                }
            ))
            .on_hover_text(relation.label());
            let id = ui.make_persistent_id("constraint-dimension-value");
            let width = ui.available_width().clamp(24.0, 110.0);
            ui.add_enabled_ui(!pending, |ui| {
                onecad_widgets::text_session::show(
                    ui,
                    id,
                    &mut draft.text,
                    relation.label(),
                    onecad_core::expr::MAX_SOURCE_LEN,
                    |text| {
                        egui::TextEdit::singleline(text)
                            .desired_width(width)
                            .char_limit(onecad_core::expr::MAX_SOURCE_LEN)
                    },
                )
            })
            .inner
        })
        .inner;
    // Dimension editing needs the Select tool and no draft, so a creation
    // chip never shows this id at the same time.
    crate::automation::name(ui.ctx(), response.id, crate::automation::DIMENSION_INPUT);
    if draft.focus && !pending {
        response.request_focus();
        draft.focus = false;
    }
    if response.changed() {
        draft.error = None;
        draft.edited = true;
    }
    let parsed = dimensions::scalar(&draft.text, relation, variables);
    crate::dimension_chip::error(ui, draft.error.as_ref().or_else(|| parsed.as_ref().err()));
    let backwards = ui.input_mut(|input| input.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab));
    let forwards = ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Tab));
    if (backwards || forwards) && !pending {
        response.request_focus();
    }
    let enter = !pending
        && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    let escape = ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    let mut apply = false;
    let mut cancel = escape;
    ui.horizontal(|ui| {
        apply = ui
            .add_enabled(
                !pending && parsed.is_ok(),
                egui::Button::new("Apply").small(),
            )
            .clicked();
        if pending {
            ui.spinner();
        }
        cancel |= ui
            .small_button(if pending { "Dismiss" } else { "Cancel" })
            .clicked();
    });
    (parsed.is_ok() && (apply || enter), cancel)
}
