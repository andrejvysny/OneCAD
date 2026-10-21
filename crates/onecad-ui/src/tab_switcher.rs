//! ⌃Tab document switcher: hold Control, press Tab to walk the open documents in
//! most-recent order (Home last), release Control to switch.
//!
//! The keys are taken out of the raw input before egui sees them: Tab would
//! otherwise move widget focus, and the release of Control has no event of its
//! own — it is observed as the modifiers no longer holding it.
use crate::{app::App, documents::TabId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Tab(TabId),
    Home,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    pub order: Vec<Target>,
    pub selected: usize,
}

impl State {
    fn step(&mut self, back: bool) {
        let len = self.order.len();
        self.selected = if back {
            (self.selected + len - 1) % len
        } else {
            (self.selected + 1) % len
        };
    }
}

/// Handles one event at its position in the frame: ⌃Tab opens or steps, W closes
/// the highlighted document, Escape abandons, and Control's release switches —
/// only if the switcher is open at that point, so a later ⌃Tab in the same frame
/// opens it again. Returns whether the event was consumed.
pub(crate) fn event(app: &mut App, event: &egui::Event) -> bool {
    if let egui::Event::ModifiersChanged(modifiers) = event {
        if !modifiers.ctrl
            && let Some(state) = app.switcher.take()
        {
            go(app, state.order[state.selected]);
        }
        return false;
    }
    let egui::Event::Key {
        key,
        pressed: true,
        modifiers,
        ..
    } = event
    else {
        return false;
    };
    match key {
        egui::Key::Tab if modifiers.ctrl && !modifiers.alt => {
            match &mut app.switcher {
                Some(state) => state.step(modifiers.shift),
                None => app.switcher = open(app, modifiers.shift),
            }
            true
        }
        egui::Key::W if app.switcher.is_some() => {
            close_selected(app);
            true
        }
        egui::Key::Escape if app.switcher.is_some() => {
            app.switcher = None;
            true
        }
        _ => false,
    }
}

/// The switcher's events alone, in frame order.
#[cfg(test)]
pub fn route(app: &mut App, raw: &mut egui::RawInput) {
    crate::input::ordered::route(app, raw, false, false, false);
}

/// Selecting a card with the pointer switches at once.
pub fn choose(app: &mut App, index: usize) {
    if let Some(state) = app.switcher.take()
        && let Some(target) = state.order.get(index)
    {
        go(app, *target);
    }
}

fn open(app: &App, back: bool) -> Option<State> {
    if app.busy() {
        return None;
    }
    let mut order: Vec<Target> = app
        .recent_tab_order()
        .into_iter()
        .map(Target::Tab)
        .collect();
    order.push(Target::Home);
    // The first press moves off what is showing: the active document, or Home.
    let current = if app.ui.start_open {
        order.len() - 1
    } else {
        0
    };
    let mut state = State {
        order,
        selected: current,
    };
    state.step(back);
    Some(state)
}

fn go(app: &mut App, target: Target) {
    match target {
        Target::Tab(tab) => app.show_tab(tab),
        Target::Home => app.open_home(),
    }
}

/// W in the switcher closes the highlighted document. One with unsaved work is
/// shown first so its save prompt sits on its own tab.
fn close_selected(app: &mut App) {
    let Some(state) = app.switcher.take() else {
        return;
    };
    if let Target::Tab(tab) = state.order[state.selected] {
        app.show_tab(tab);
        if app.doc.tab == tab {
            app.request_close_tab();
        }
    }
}

/// Draws the switcher while Control is held.
pub fn show(app: &mut App, ctx: &egui::Context) {
    use onecad_widgets::components::overlays::{SwitcherItem, TabSwitcher};
    let Some(state) = app.switcher.clone() else {
        return;
    };
    let titles: Vec<(String, bool, Option<egui::TextureId>)> = state
        .order
        .iter()
        .map(|target| match target {
            Target::Tab(tab) => app.editor(*tab).map_or_else(
                || (String::new(), false, None),
                |doc| {
                    let dirty =
                        doc.authoring_dirty || doc.document_state.as_ref().is_some_and(|s| s.dirty);
                    let thumbnail = doc.thumbnail.as_ref().map(|t| t.texture);
                    (doc.editor_title(), dirty, thumbnail)
                },
            ),
            Target::Home => ("Home".into(), false, None),
        })
        .collect();
    let items: Vec<SwitcherItem> = state
        .order
        .iter()
        .zip(&titles)
        .map(|(target, (title, dirty, thumbnail))| SwitcherItem {
            title,
            icon: if *target == Target::Home {
                "home"
            } else {
                "body"
            },
            dirty: *dirty,
            thumbnail: *thumbnail,
            home: *target == Target::Home,
        })
        .collect();
    let out = TabSwitcher::new(&items, state.selected).show(ctx, egui::Id::unique("tab-switcher"));
    if let Some(index) = out.clicked {
        choose(app, index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stepping_wraps_both_ways() {
        let mut state = State {
            order: vec![Target::Tab(1), Target::Tab(2), Target::Home],
            selected: 0,
        };
        state.step(false);
        assert_eq!(state.selected, 1);
        state.step(false);
        state.step(false);
        assert_eq!(state.selected, 0);
        state.step(true);
        assert_eq!(state.order[state.selected], Target::Home);
    }
}
