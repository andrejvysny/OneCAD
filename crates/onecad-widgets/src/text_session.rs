//! Optional native text adapter. Widgets own drafts; application services own accepted geometry.
mod model;
use egui::{
    Context, Id, TextEdit, Ui,
    text::{CCursor, CCursorRange},
    widgets::text_edit::TextEditOutput,
};
use model::Active;
pub use model::{Edit, MAX_UTF16, Operation, Range, Snapshot, byte_offset};

#[derive(Clone, Default)]
struct Session {
    enabled: bool,
    scope: String,
    generation: u64,
    active: Option<Active>,
    cancelled: Vec<Cancelled>,
    seen: bool,
    tab_origin: Option<Id>,
}

#[derive(Clone)]
struct Cancelled {
    scope: String,
    field: Id,
    expected: String,
    original: String,
    selection: Range,
}

impl Session {
    fn retire(&mut self) {
        if let Some(a) = self.active.take()
            && let Some((original, selection)) = a.composition
        {
            self.cancelled.push(Cancelled {
                scope: self.scope.clone(),
                field: a.snapshot.field,
                expected: a.base,
                original,
                selection,
            });
        }
    }

    fn restore(&mut self, ui: &Ui, id: Id, text: &mut String) -> bool {
        let Some(index) = self
            .cancelled
            .iter()
            .position(|c| c.scope == self.scope && c.field == id)
        else {
            return false;
        };
        let cancelled = self.cancelled.remove(index);
        if *text != cancelled.expected {
            return false;
        }
        let changed = *text != cancelled.original;
        *text = cancelled.original;
        let mut state = TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(char_range(text, cancelled.selection)));
        state.store(ui.ctx(), id);
        changed
    }
}

fn key() -> Id {
    Id::unique("onecad.native-text-session.v1")
}
fn read(ctx: &Context) -> Session {
    ctx.data_mut(|d| d.get_temp(key()).unwrap_or_default())
}
fn write(ctx: &Context, session: Session) {
    ctx.data_mut(|d| d.insert_temp(key(), session));
}

pub fn enabled(ctx: &Context) -> bool {
    read(ctx).enabled
}

pub fn enable(ctx: &Context) {
    let mut s = read(ctx);
    s.enabled = true;
    write(ctx, s);
}

pub fn invalidate(ctx: &Context) {
    let mut s = read(ctx);
    s.retire();
    write(ctx, s);
}

pub fn begin_frame(ctx: &Context, scope: String) {
    let mut s = read(ctx);
    if !s.enabled {
        return;
    }
    if s.scope != scope {
        s.retire();
        s.scope = scope;
    }
    s.seen = false;
    s.tab_origin = None;
    write(ctx, s);
}

/// Refresh after input routing or chrome switches the owner within the current pass.
pub fn refresh_scope(ctx: &Context, scope: String) {
    let mut s = read(ctx);
    if s.enabled && s.scope != scope {
        s.retire();
        s.scope = scope;
        write(ctx, s);
    }
}

pub fn end_frame(ctx: &Context) {
    let mut s = read(ctx);
    if !s.seen
        || s.active
            .as_ref()
            .is_some_and(|a| !ctx.memory(|m| m.has_focus(a.snapshot.field)))
    {
        // An absent owner restores only its matching draft on its next visible pass.
        s.retire();
    }
    write(ctx, s);
}

pub fn snapshot(ctx: &Context) -> Option<Snapshot> {
    read(ctx).active.map(|a| a.snapshot)
}

/// Acknowledges synchronously, before UIKit's next callback; egui applies the pending draft next pass.
pub fn apply(ctx: &Context, operation: Operation) -> bool {
    let mut s = read(ctx);
    let full = s.cancelled.len() >= 16
        && matches!(&operation.edit, Edit::Mark(_, _))
        && s.active.as_ref().is_none_or(|a| a.composition.is_none());
    let accepted = !full && s.active.as_mut().is_some_and(|a| a.apply(operation));
    write(ctx, s);
    if accepted {
        ctx.request_repaint();
    }
    accepted
}

/// Custom field groups consume this frame's Tab from its original owner, before egui traversed.
pub fn native_tab_origin(ctx: &Context) -> Option<Id> {
    read(ctx).tab_origin
}

pub fn composing(ctx: &Context) -> bool {
    snapshot(ctx).is_some_and(|s| s.marked.is_some())
}

/// Applies only to the exact draft last published by this field, preserving external undo/changes.
pub fn before(ui: &Ui, id: Id, text: &mut String, enabled: bool) -> bool {
    let mut s = read(ui.ctx());
    let restored = s.restore(ui, id, text);
    let focused = enabled && ui.is_enabled() && ui.memory(|m| m.has_focus(id));
    let Some(a) = s.active.as_mut().filter(|a| a.snapshot.field == id) else {
        write(ui.ctx(), s);
        return restored;
    };
    if !focused || *text != a.base {
        s.retire();
        let cancelled = s.restore(ui, id, text);
        write(ui.ctx(), s);
        return restored || cancelled;
    }
    if !a.pending {
        write(ui.ctx(), s);
        return restored;
    }
    let mut state = TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
    let mut undoer = state.undoer();
    let old = state.cursor.char_range().unwrap_or_default();
    undoer.add_undo(&(old, text.clone()));
    let changed = *text != a.snapshot.text;
    *text = a.snapshot.text.clone();
    let selection = char_range(text, a.snapshot.selection);
    state.cursor.set_char_range(Some(selection));
    state.set_undoer(undoer);
    state.store(ui.ctx(), id);
    if let Some(key) = a.key.take() {
        if key == egui::Key::Tab {
            s.tab_origin = Some(id);
            ui.memory_mut(|m| m.move_focus(egui::FocusDirection::Next));
        }
        ui.ctx().input_mut(|i| {
            i.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            })
        });
    }
    a.base = text.clone();
    a.pending = false;
    write(ui.ctx(), s);
    changed || restored
}

fn char_range(text: &str, range: Range) -> CCursorRange {
    let at = |offset| byte_offset(text, offset).map_or(0, |b| text[..b].chars().count());
    CCursorRange::two(CCursor::new(at(range.start)), CCursor::new(at(range.end)))
}

pub fn after(ui: &Ui, text: &str, output: &TextEditOutput, label: &str, max_chars: usize) {
    let mut s = read(ui.ctx());
    if !s.enabled || !output.response.has_focus() || !ui.is_enabled() {
        return;
    }
    let id = output.response.id;
    let chars = output
        .cursor_range
        .unwrap_or_else(|| CCursorRange::one(CCursor::new(text.chars().count())));
    let [start, end] = chars.sorted_cursors();
    let utf16 = |at: usize| text.chars().take(at).map(char::len_utf16).sum::<usize>() as u32;
    let selection = Range {
        start: utf16(start.index.0),
        end: utf16(end.index.0),
    };
    if text.encode_utf16().count() > MAX_UTF16 {
        s.active = None;
        write(ui.ctx(), s);
        return;
    }
    if s.active.as_ref().is_none_or(|a| a.snapshot.field != id) {
        s.retire();
        let Some(generation) = s.generation.checked_add(1) else {
            return;
        };
        s.generation = generation;
        s.active = Some(Active {
            snapshot: Snapshot {
                generation,
                revision: 1,
                sequence: 0,
                field: id,
                text: text.to_owned(),
                selection,
                marked: None,
                label: label.to_owned(),
                rect: output.response.rect,
                caret: output.response.rect,
                enabled: true,
            },
            base: text.to_owned(),
            pending: false,
            max_chars,
            composition: None,
            key: None,
        });
    }
    if let Some(a) = s.active.as_mut() {
        if a.snapshot.text != text || a.snapshot.selection != selection {
            a.snapshot.revision = a.snapshot.revision.saturating_add(1);
            // Ordinary egui edits supersede any previous composition state.
            a.snapshot.marked = None;
            a.composition = None;
        }
        a.snapshot.text = text.to_owned();
        a.snapshot.selection = selection;
        a.snapshot.rect = output.response.rect;
        let caret = output.galley.pos_from_cursor(chars.primary);
        a.snapshot.caret = caret.translate(output.galley_pos.to_vec2());
        a.snapshot.label = label.chars().take(256).collect();
        a.base = text.to_owned();
        a.max_chars = max_chars;
    }
    s.seen = true;
    write(ui.ctx(), s);
}

/// Shared builder wrapper for raw editor fields; native integration remains dormant on desktop.
pub fn show(
    ui: &mut Ui,
    id: Id,
    text: &mut String,
    label: &str,
    max_chars: usize,
    build: impl FnOnce(&mut String) -> TextEdit<'_>,
) -> egui::Response {
    let changed = before(ui, id, text, ui.is_enabled());
    let mut output = build(text).id(id).show(ui);
    if changed {
        output.response.mark_changed();
    }
    after(ui, text, &output, label, max_chars);
    output.response.response
}

#[cfg(test)]
mod tests;
