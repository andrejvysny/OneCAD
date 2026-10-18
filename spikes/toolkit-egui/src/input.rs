//! Dimension editing and native input routing.

use crate::{expr, menu, ui::font};
use egui::{Align2, FontFamily, Key, Ui, vec2};

/// The floating dimension field: a committed value with history, plus the text being edited.
pub(crate) struct DimInput {
    pub id: Option<egui::Id>,
    pub composing: bool,
    pub value_mm: f64,
    pub text: String,
    pub editing: bool,
    pub error: Option<String>,
    pub undo: Vec<f64>,
    pub redo: Vec<f64>,
    /// World anchor: midpoint of the measured edge, and its two ends.
    pub anchor: [glam::Vec3; 3],
    pub request_focus: bool,
}

impl DimInput {
    fn has_focus(&self, ctx: &egui::Context) -> bool {
        self.id.is_some_and(|id| ctx.memory(|m| m.has_focus(id)))
    }

    /// Run before egui processes Escape, which otherwise clears focus before TextEdit runs.
    pub fn prepare_input(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if !self.has_focus(ctx)
            || !raw.focused
            || raw
                .events
                .iter()
                .any(|e| matches!(e, egui::Event::WindowFocused(false)))
        {
            self.composing = false;
            return;
        }
        let mut protected = self.composing;
        for event in &raw.events {
            match event {
                egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => {
                    self.composing = !text.is_empty();
                    protected = true;
                }
                egui::Event::Ime(egui::ImeEvent::Commit(_)) => {
                    self.composing = false;
                    protected = true;
                }
                #[allow(deprecated)]
                egui::Event::Ime(egui::ImeEvent::Disabled) => {
                    protected |= self.composing;
                    self.composing = false;
                }
                _ => {}
            }
        }
        if protected {
            raw.events.retain(|event| {
                !matches!(
                    event,
                    egui::Event::Key {
                        key: Key::Enter | Key::Escape,
                        pressed: true,
                        ..
                    }
                )
            });
        }
    }

    /// A focused field owns Undo/Redo even when its local history is empty.
    pub fn route_edit(
        &mut self,
        ctx: &egui::Context,
        action: menu::Action,
        status: &mut String,
        events: &mut Vec<egui::Event>,
    ) {
        if let Some(focused) = ctx.memory(|m| m.focused()) {
            menu::push_command(events, Key::Z, action == menu::Action::Redo);
            let target = if self.id == Some(focused) {
                "dimension text undoer"
            } else {
                "focused widget"
            };
            eprintln!("[route] {action:?} -> {target}");
            return;
        }
        let (from, to) = if action == menu::Action::Redo {
            (&mut self.redo, &mut self.undo)
        } else {
            (&mut self.undo, &mut self.redo)
        };
        if let Some(value) = from.pop() {
            to.push(self.value_mm);
            self.value_mm = value;
            self.text = format!("{value:.2}");
            *status = format!("{action:?}: width = {value:.2} mm");
            eprintln!("[route] {action:?} -> document history (width now {value:.2} mm)");
        } else {
            eprintln!("[route] {action:?} -> document history (nothing to {action:?})");
        }
    }

    pub fn route_clipboard(
        &self,
        ctx: &egui::Context,
        action: menu::Action,
        events: &mut Vec<egui::Event>,
    ) {
        if !self.has_focus(ctx) {
            eprintln!("[route] {action:?} -> no text focus, ignored");
            return;
        }
        match action {
            menu::Action::Cut => events.push(egui::Event::Cut),
            menu::Action::Copy => events.push(egui::Event::Copy),
            menu::Action::Paste => {
                #[cfg(target_os = "macos")]
                if let Ok(text) = arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                    events.push(egui::Event::Paste(text));
                }
            }
            menu::Action::SelectAll => menu::push_command(events, Key::A, false),
            _ => return,
        }
        eprintln!("[route] {action:?} -> text field");
    }
}

fn text_field(dim: &mut DimInput, ui: &mut Ui, t: &crate::theme::Theme) -> egui::Response {
    let edit = egui::TextEdit::singleline(&mut dim.text)
        .id_salt("dimension-input")
        .font(font(14.0, FontFamily::Monospace))
        .text_color(if dim.error.is_some() {
            t.traffic_close
        } else {
            t.ink
        })
        .frame(egui::Frame::NONE)
        .desired_width(if dim.editing { 120.0 } else { 64.0 })
        .align(egui::Align2::RIGHT_CENTER)
        .return_key(None);
    let response = ui.add(edit);
    dim.id = Some(response.id);
    response.widget_info(|| {
        let mut info = egui::WidgetInfo::text_edit(true, "", &dim.text, "Flange width");
        info.label = Some("Flange width (mm)".into());
        info
    });
    if response.changed() {
        eprintln!("[dim] text={:?}", dim.text);
    }
    response
}

fn commit(dim: &mut DimInput, status: &mut String, response: &egui::Response) {
    match expr::eval_mm(&dim.text) {
        Ok(value) => {
            dim.undo.push(dim.value_mm);
            dim.redo.clear();
            dim.value_mm = value;
            dim.text = format!("{value:.2}");
            dim.error = None;
            dim.editing = false;
            *status = format!("Width = {value:.2} mm");
            eprintln!("[dim] commit {value:.4} mm");
            response.surrender_focus();
        }
        Err(error) => {
            eprintln!("[dim] reject: {error}");
            dim.error = Some(error);
        }
    }
}

fn handle_response(dim: &mut DimInput, status: &mut String, ui: &Ui, response: &egui::Response) {
    if std::mem::take(&mut dim.request_focus) {
        response.request_focus();
    }
    if response.gained_focus() {
        // The app rewrites the text on commit/undo/redo; each edit session must start
        // with that value, or text Undo could restore a stale document value.
        if let Some(mut state) = egui::text_edit::TextEditState::load(ui.ctx(), response.id) {
            state.clear_undoer();
            state.store(ui.ctx(), response.id);
        }
        dim.editing = true;
        crate::metrics::mark("dimension_focus");
    }
    let (enter, escape) = ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
    if dim.editing && response.has_focus() && enter {
        commit(dim, status, response);
    } else if dim.editing && (escape || response.lost_focus() || !ui.input(|i| i.focused)) {
        dim.text = format!("{:.2}", dim.value_mm);
        dim.error = None;
        dim.editing = false;
        dim.composing = false;
        response.surrender_focus();
        eprintln!("[dim] cancel");
    }
}

pub(crate) fn dimension_field(
    dim: &mut DimInput,
    status: &mut String,
    ui: &mut Ui,
    t: &crate::theme::Theme,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 0.0);
        let response = text_field(dim, ui, t);
        handle_response(dim, status, ui, &response);
        crate::ui::label(ui, "mm", 12.0, t.ink5, FontFamily::Proportional);
        if dim.editing {
            let preview = match (&dim.error, expr::eval_mm(&dim.text)) {
                (Some(error), _) => format!("⚠ {error}"),
                (None, Ok(value)) => format!("= {value:.2} mm"),
                (None, Err(_)) => String::new(),
            };
            if !preview.is_empty() {
                let pos = ui.min_rect().left_bottom() + vec2(0.0, 8.0);
                ui.painter().text(
                    pos,
                    Align2::LEFT_TOP,
                    preview,
                    font(11.5, FontFamily::Monospace),
                    t.ink4,
                );
            }
        }
    });
}

#[cfg(test)]
#[path = "input_tests.rs"]
mod tests;
