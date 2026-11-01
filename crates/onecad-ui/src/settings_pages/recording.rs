//! Keyboard-shortcut page state and the key-recording rules; no drawing here.
use crate::{
    shortcut_prefs::{self, KeyContext, ShortcutOverride},
    tool_registry::{ToolId, default_bindings, effective_key, recordable},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    Modeling,
    Sketch,
    View,
    Changed,
}

impl Filter {
    pub const ALL: [Filter; 5] = [
        Self::All,
        Self::Modeling,
        Self::Sketch,
        Self::View,
        Self::Changed,
    ];
}

/// Command, Control or Option held: a shortcut chord, not a key to record.
fn chord(modifiers: &egui::Modifiers) -> bool {
    modifiers.command || modifiers.mac_cmd || modifiers.ctrl || modifiers.alt
}

/// A pressed chord among raw events; the app consumes these before the page sees them.
pub fn chord_pressed(events: &[egui::Event]) -> bool {
    events.iter().any(|event| {
        matches!(event, egui::Event::Key { pressed: true, modifiers, .. } if chord(modifiers))
    })
}

/// Whether the recording row was drawn this frame inside the scroll area's clip.
pub fn row_on_screen(row: Option<(egui::Rect, egui::Rect)>) -> bool {
    row.is_some_and(|(row, clip)| clip.intersects(row))
}

/// A key that another tool in the same context already holds, awaiting confirmation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pending {
    pub context: KeyContext,
    pub tool: ToolId,
    pub key: egui::Key,
    pub loser: ToolId,
}

#[derive(Clone, Debug, Default)]
pub struct ShortcutsUi {
    /// (row, visible clip) of the row being recorded, refreshed each time it is drawn.
    pub row: std::cell::Cell<Option<(egui::Rect, egui::Rect)>>,
    pub search: String,
    pub filter: Filter,
    pub recording: Option<(KeyContext, ToolId)>,
    pub pending: Option<Pending>,
}

impl ShortcutsUi {
    /// True while the page owns key presses, so Esc must not leave the page.
    pub fn capturing(&self) -> bool {
        self.recording.is_some()
    }

    pub fn start(&mut self, context: KeyContext, tool: ToolId) {
        self.recording = Some((context, tool));
        self.pending = None;
    }

    pub fn cancel(&mut self) {
        self.recording = None;
        self.pending = None;
    }

    /// Applies the key presses of one frame to the recording, if any.
    pub fn keys(&mut self, overrides: &mut Vec<ShortcutOverride>, events: &[egui::Event]) {
        for event in events {
            let Some((context, tool)) = self.recording else {
                return;
            };
            let egui::Event::Key {
                key,
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } = event
            else {
                continue;
            };
            // Command, Control or Option chords are shortcuts, not keys to record: stop
            // and leave the event to run.
            if *key == egui::Key::Escape || chord(modifiers) {
                self.cancel();
            } else if modifiers.is_none() && recordable(*key) {
                self.record(overrides, context, tool, *key);
            }
        }
    }

    fn record(
        &mut self,
        overrides: &mut Vec<ShortcutOverride>,
        context: KeyContext,
        tool: ToolId,
        key: egui::Key,
    ) {
        match shortcut_prefs::conflict(overrides, context, tool, key) {
            None => {
                shortcut_prefs::assign(overrides, context, tool, key);
                self.cancel();
            }
            Some(loser) => {
                self.pending = Some(Pending {
                    context,
                    tool,
                    key,
                    loser,
                });
            }
        }
    }

    /// "Use for {Tool}": takes the key and unbinds the tool that held it.
    pub fn confirm(&mut self, overrides: &mut Vec<ShortcutOverride>) {
        if let Some(p) = self.pending {
            shortcut_prefs::assign(overrides, p.context, p.tool, p.key);
        }
        self.cancel();
    }
}

/// One tool row of the shortcut table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToolRow {
    pub context: KeyContext,
    pub tool: ToolId,
}

impl ToolRow {
    pub fn key(self, overrides: &[ShortcutOverride]) -> Option<egui::Key> {
        effective_key(overrides, self.tool, self.context)
    }

    pub fn default_key(self) -> Option<egui::Key> {
        default_bindings(self.context)
            .iter()
            .find(|(_, tool)| *tool == self.tool)
            .map(|(key, _)| *key)
    }

    pub fn changed(self, overrides: &[ShortcutOverride]) -> bool {
        self.key(overrides) != self.default_key()
    }
}

/// Every remappable tool: Modeling bindings, then Sketch, in registry order.
pub fn tool_rows() -> Vec<ToolRow> {
    [KeyContext::Modeling, KeyContext::Sketch]
        .into_iter()
        .flat_map(|context| {
            default_bindings(context)
                .iter()
                .map(move |(_, tool)| ToolRow {
                    context,
                    tool: *tool,
                })
        })
        .collect()
}
