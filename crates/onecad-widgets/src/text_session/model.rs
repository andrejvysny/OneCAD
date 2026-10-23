//! Bounded native draft protocol. All offsets are UTF-16 scalar boundaries.
use egui::{Id, Rect};

pub const MAX_UTF16: usize = 16_384;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Range {
    pub start: u32,
    pub end: u32,
}

impl Range {
    pub fn caret(at: u32) -> Self {
        Self { start: at, end: at }
    }
    pub fn bytes(self, text: &str) -> Option<std::ops::Range<usize>> {
        (self.start <= self.end).then_some(())?;
        Some(byte_offset(text, self.start)?..byte_offset(text, self.end)?)
    }
}

pub fn byte_offset(text: &str, offset: u32) -> Option<usize> {
    let mut utf16 = 0;
    for (byte, ch) in text.char_indices() {
        if utf16 == offset {
            return Some(byte);
        }
        utf16 += ch.len_utf16() as u32;
    }
    (utf16 == offset).then_some(text.len())
}

#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub generation: u64,
    pub revision: u64,
    pub sequence: u64,
    pub field: Id,
    pub text: String,
    pub selection: Range,
    pub marked: Option<Range>,
    pub label: String,
    pub rect: Rect,
    pub caret: Rect,
    pub enabled: bool,
}

#[derive(Clone, Debug)]
pub enum Edit {
    Replace(Range, String),
    Select(Range),
    Mark(String, Range),
    Unmark,
    Cancel,
    Confirm(bool), // true: Tab, false: Return
    Blur,
}

#[derive(Clone, Debug)]
pub struct Operation {
    pub generation: u64,
    pub revision: u64,
    pub sequence: u64,
    pub edit: Edit,
}

#[derive(Clone)]
pub(super) struct Active {
    pub snapshot: Snapshot,
    pub base: String,
    pub pending: bool,
    pub max_chars: usize,
    pub composition: Option<(String, Range)>,
    pub key: Option<egui::Key>,
}

impl Active {
    pub fn apply(&mut self, op: Operation) -> bool {
        let s = &self.snapshot;
        if op.generation != s.generation
            || op.revision != s.revision
            || op.sequence <= s.sequence
            || self.key.is_some()
        {
            return false;
        }
        let mut next = self.clone();
        if !next.edit(op.edit) {
            return false;
        }
        let Some(revision) = s.revision.checked_add(1) else {
            return false;
        };
        next.snapshot.revision = revision;
        next.snapshot.sequence = op.sequence;
        next.pending = true;
        *self = next;
        true
    }

    fn edit(&mut self, edit: Edit) -> bool {
        match edit {
            Edit::Replace(range, text) => {
                if !self.replace(range, &text) {
                    return false;
                }
                self.snapshot.marked = None;
                self.composition = None;
            }
            Edit::Select(range) => {
                if range.bytes(&self.snapshot.text).is_none() {
                    return false;
                }
                self.snapshot.selection = range;
            }
            Edit::Mark(text, selection) => {
                if selection.bytes(&text).is_none() {
                    return false;
                }
                if self.composition.is_none() {
                    self.composition = Some((self.snapshot.text.clone(), self.snapshot.selection));
                }
                let range = self.snapshot.marked.unwrap_or(self.snapshot.selection);
                if !self.replace(range, &text) {
                    return false;
                }
                self.snapshot.marked = Some(Range {
                    start: range.start,
                    end: range.start + text.encode_utf16().count() as u32,
                });
                self.snapshot.selection = Range {
                    start: range.start + selection.start,
                    end: range.start + selection.end,
                };
            }
            Edit::Unmark => {
                self.snapshot.marked = None;
                self.composition = None;
            }
            Edit::Cancel | Edit::Blur => self.cancel(),
            Edit::Confirm(tab) => {
                self.snapshot.marked = None;
                self.composition = None;
                self.key = Some(if tab {
                    egui::Key::Tab
                } else {
                    egui::Key::Enter
                });
            }
        }
        true
    }

    fn replace(&mut self, range: Range, text: &str) -> bool {
        let Some(bytes) = range.bytes(&self.snapshot.text) else {
            return false;
        };
        let mut draft = self.snapshot.text.clone();
        draft.replace_range(bytes, text);
        if draft.encode_utf16().count() > MAX_UTF16 || draft.chars().count() > self.max_chars {
            return false;
        }
        self.snapshot.text = draft;
        self.snapshot.selection = Range::caret(range.start + text.encode_utf16().count() as u32);
        true
    }

    pub fn cancel(&mut self) {
        if let Some((text, selection)) = self.composition.take() {
            self.snapshot.text = text;
            self.snapshot.selection = selection;
            self.pending = true;
        }
        self.snapshot.marked = None;
    }
}
