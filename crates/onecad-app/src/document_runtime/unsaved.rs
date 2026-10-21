//! What changed since the last save, for a close prompt ("3 changes since last
//! save: Mate 4, Gear pair, Housing offset").
//!
//! The undo history is the only record of user-visible steps, so the saved point
//! is remembered as the undo labels at save time plus the eviction count. The
//! current history either extends that point (the new labels are the changes),
//! has been undone below it (only a count is honest), or has diverged from it.

use super::DocumentRuntime;

/// The undo history as of a save.
#[derive(Debug, Clone, Default)]
pub(crate) struct SavedHistory {
    labels: Vec<String>,
    evictions: u64,
}

/// Unsaved work, described for a person.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnsavedChanges {
    /// Steps since the save, oldest first; `None` when they cannot be named
    /// (undone past the save, or the save point is unknown).
    pub labels: Option<Vec<String>>,
    /// Number of steps that differ from the save; 0 when unknown.
    pub count: usize,
}

impl DocumentRuntime {
    pub(super) fn history_now(&self) -> SavedHistory {
        SavedHistory {
            labels: self.session.undo_labels().map(str::to_owned).collect(),
            evictions: self.session.evictions(),
        }
    }

    /// The steps that separate this document from its last save.
    #[must_use]
    pub fn unsaved_changes(&self) -> UnsavedChanges {
        if !self.dirty {
            return UnsavedChanges {
                labels: Some(Vec::new()),
                count: 0,
            };
        }
        let Some(saved) = &self.saved_history else {
            return UnsavedChanges::default();
        };
        diff(saved, &self.history_now())
    }
}

fn diff(saved: &SavedHistory, now: &SavedHistory) -> UnsavedChanges {
    // Steps evicted since the save left the bottom of both histories.
    let shifted = now.evictions.saturating_sub(saved.evictions);
    let Some(saved) = usize::try_from(shifted)
        .ok()
        .and_then(|shifted| saved.labels.get(shifted..))
    else {
        return UnsavedChanges::default();
    };
    if now.labels.starts_with(saved) {
        let labels = now.labels[saved.len()..].to_vec();
        return UnsavedChanges {
            count: labels.len(),
            labels: Some(labels),
        };
    }
    if saved.starts_with(&now.labels) {
        return UnsavedChanges {
            labels: None,
            count: saved.len() - now.labels.len(),
        };
    }
    UnsavedChanges::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history(labels: &[&str], evictions: u64) -> SavedHistory {
        SavedHistory {
            labels: labels.iter().map(|s| (*s).to_owned()).collect(),
            evictions,
        }
    }

    #[test]
    fn steps_after_the_save_are_named_in_order() {
        let saved = history(&["Sketch 1", "Extrude 1"], 0);
        let now = history(&["Sketch 1", "Extrude 1", "Mate 4", "Gear pair"], 0);
        assert_eq!(
            diff(&saved, &now),
            UnsavedChanges {
                labels: Some(vec!["Mate 4".into(), "Gear pair".into()]),
                count: 2
            }
        );
    }

    #[test]
    fn undoing_past_the_save_is_counted_not_named() {
        let saved = history(&["Sketch 1", "Extrude 1", "Fillet 1"], 0);
        let now = history(&["Sketch 1"], 0);
        assert_eq!(
            diff(&saved, &now),
            UnsavedChanges {
                labels: None,
                count: 2
            }
        );
    }

    #[test]
    fn a_diverged_history_is_unknown() {
        let saved = history(&["Sketch 1", "Extrude 1"], 0);
        let now = history(&["Sketch 1", "Hole 1"], 0);
        assert_eq!(diff(&saved, &now), UnsavedChanges::default());
    }

    #[test]
    fn evictions_shift_the_saved_point() {
        let saved = history(&["a", "b", "c"], 10);
        let now = history(&["b", "c", "d"], 11);
        assert_eq!(
            diff(&saved, &now),
            UnsavedChanges {
                labels: Some(vec!["d".into()]),
                count: 1
            }
        );
        assert_eq!(
            diff(&saved, &history(&["x"], 20)),
            UnsavedChanges::default()
        );
    }
}
