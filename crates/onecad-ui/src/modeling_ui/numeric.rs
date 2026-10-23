//! Native option fields keep their text until validation, independent of preview sequences.
use onecad_widgets::{
    components::inputs::{NumberField, ParsedValue, parse_value},
    text_session,
};

#[derive(Clone)]
struct Draft {
    accepted: f64,
    text: String,
}

pub(super) struct Fields {
    owner: egui::Id,
    pub changed: bool,
    pub error: Option<String>,
}

impl Fields {
    pub fn new(owner: impl std::hash::Hash + std::fmt::Debug) -> Self {
        Self {
            owner: egui::Id::unique(owner),
            changed: false,
            error: None,
        }
    }

    pub fn count(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        label: &str,
        value: &mut u32,
        range: std::ops::RangeInclusive<u32>,
    ) -> egui::Response {
        if !text_session::enabled(ui.ctx()) {
            return ui.add(egui::DragValue::new(value).range(range));
        }
        let mut numeric = f64::from(*value);
        let response = self.native(ui, id, label, &mut numeric, Some(range));
        *value = numeric as u32;
        response
    }

    pub fn value(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        label: &str,
        value: &mut f64,
    ) -> egui::Response {
        if text_session::enabled(ui.ctx()) {
            return self.native(ui, id, label, value, None);
        }
        let field_id = ui.next_auto_id();
        let ctx = ui.ctx().clone();
        ui.add(
            egui::DragValue::new(value)
                .speed(0.1)
                .custom_formatter(move |value, _| {
                    if ctx.memory(|m| m.has_focus(field_id)) {
                        value.to_string()
                    } else {
                        format!("{value:.2}")
                    }
                }),
        )
    }

    fn native(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        label: &str,
        value: &mut f64,
        range: Option<std::ops::RangeInclusive<u32>>,
    ) -> egui::Response {
        let salt = (self.owner, id);
        let draft_id = ui.make_persistent_id((salt, "draft"));
        let mut draft: Draft = ui
            .ctx()
            .data_mut(|data| data.get_temp(draft_id))
            .unwrap_or(Draft {
                accepted: *value,
                text: value.to_string(),
            });
        // External placement/undo supersedes a draft, but preview sequence changes do not.
        if draft.accepted != *value {
            draft = Draft {
                accepted: *value,
                text: value.to_string(),
            };
            self.changed = true;
        }
        let previous = draft.text.clone();
        let error = validation(&draft.text, range.as_ref()).err();
        let response = ui.add(
            NumberField::new(salt, &mut draft.text)
                .label(label)
                .decimals(if range.is_some() { 0 } else { 2 })
                .step(if range.is_some() { 1.0 } else { 0.1 })
                .step_keys(!text_session::composing(ui.ctx()))
                .error(error.as_deref())
                .width(ui.available_width()),
        );
        self.changed |= previous != draft.text;
        match validation(&draft.text, range.as_ref()) {
            Ok(parsed) => {
                *value = parsed;
                draft.accepted = parsed;
            }
            Err(message) => {
                self.error
                    .get_or_insert_with(|| format!("{label}: {message}"));
            }
        }
        response
            .ctx
            .accesskit_node_builder(response.id, |node| node.set_label(label));
        ui.ctx().data_mut(|data| data.insert_temp(draft_id, draft));
        response
    }
}

fn validation(text: &str, range: Option<&std::ops::RangeInclusive<u32>>) -> Result<f64, String> {
    let ParsedValue::Number(value) = parse_value(text) else {
        return Err("Enter a finite number".into());
    };
    if let Some(range) = range
        && (value.fract() != 0.0
            || value < f64::from(*range.start())
            || value > f64::from(*range.end()))
    {
        return Err(format!(
            "Enter a whole number from {} to {}",
            range.start(),
            range.end()
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod tests;
