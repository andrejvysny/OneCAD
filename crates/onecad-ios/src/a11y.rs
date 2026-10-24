//! egui's AccessKit tree, flattened for `UIAccessibilityElement`s.
//!
//! Identifier rule: the AccessKit `author_id` when a widget sets one, otherwise
//! `<role>.<label>` with the label lowercased and every run of other characters
//! collapsed to `-`; repeats in z-order get `#2`, `#3`, … Labels are the editor's
//! English UI strings, never localized, so identifiers are stable per build.
use crate::ffi::{self, A11yNode};
use egui::accesskit::{self, Action, Affine, Node, NodeId, Role, TreeUpdate};
use std::collections::{HashMap, HashSet};
use std::ffi::CString;

#[derive(Clone, Debug, PartialEq)]
pub struct Flat {
    pub id: u64,
    pub role: u32,
    pub flags: u32,
    pub rect: [f32; 4],
    pub identifier: String,
    pub label: String,
    pub value: String,
    pub clickable: bool,
}

pub enum Activation {
    Click(NodeId),
    Tap(egui::Pos2),
}

#[derive(Default)]
pub struct Tree {
    nodes: HashMap<NodeId, Node>,
    root: Option<NodeId>,
    focus: Option<NodeId>,
    flat: Vec<Flat>,
    generation: u64,
    snapshot: Vec<A11yNode>,
    strings: Vec<CString>,
    snapshot_generation: Option<u64>,
    #[cfg(test)]
    snapshot_rebuilds: usize,
}

fn role(role: Role) -> u32 {
    match role {
        Role::Button | Role::DefaultButton => ffi::ROLE_BUTTON,
        Role::TextInput
        | Role::MultilineTextInput
        | Role::SearchInput
        | Role::NumberInput
        | Role::PasswordInput => ffi::ROLE_TEXT_FIELD,
        Role::Label | Role::Paragraph => ffi::ROLE_STATIC_TEXT,
        Role::Tab => ffi::ROLE_TAB,
        Role::CheckBox | Role::Switch | Role::RadioButton => ffi::ROLE_CHECKBOX,
        Role::Image => ffi::ROLE_IMAGE,
        Role::Canvas => ffi::ROLE_CANVAS,
        Role::MenuItem | Role::MenuItemCheckBox | Role::MenuItemRadio => ffi::ROLE_MENU_ITEM,
        Role::Slider | Role::SpinButton | Role::ProgressIndicator => ffi::ROLE_SLIDER,
        Role::Group | Role::TabList | Role::Toolbar | Role::Menu | Role::List => ffi::ROLE_GROUP,
        _ => ffi::ROLE_OTHER,
    }
}

fn role_name(role: u32) -> &'static str {
    const NAMES: [&str; 11] = [
        "other",
        "button",
        "text_field",
        "static_text",
        "tab",
        "checkbox",
        "image",
        "canvas",
        "menu_item",
        "slider",
        "group",
    ];
    NAMES.get(role as usize).copied().unwrap_or("other")
}

pub fn normalize(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    for c in label.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let trimmed = out.trim_end_matches('-');
    if trimmed.is_empty() {
        "unlabeled".into()
    } else {
        trimmed.to_owned()
    }
}

fn flags(node: &Node, focused: bool) -> u32 {
    let mut flags = 0;
    if node.is_disabled() {
        flags |= ffi::A11Y_DISABLED;
    }
    if node.is_selected() == Some(true) || node.toggled() == Some(accesskit::Toggled::True) {
        flags |= ffi::A11Y_SELECTED;
    }
    if focused {
        flags |= ffi::A11Y_FOCUSED;
    }
    flags
}

impl Tree {
    /// Applies one update; returns whether the flattened list changed.
    pub fn update(&mut self, update: TreeUpdate) -> bool {
        if let Some(tree) = &update.tree {
            self.root = Some(tree.root);
        }
        self.focus = Some(update.focus);
        self.nodes.extend(update.nodes);
        let flat = self.flatten();
        if flat == self.flat {
            return false;
        }
        self.flat = flat;
        self.generation += 1;
        true
    }

    fn flatten(&mut self) -> Vec<Flat> {
        let mut out = Vec::new();
        let mut reachable = HashSet::new();
        if let Some(root) = self.root {
            // The root's own transform maps points to pixels; the ABI wants points.
            self.visit(root, Affine::IDENTITY, true, &mut out, &mut reachable);
        }
        self.nodes.retain(|id, _| reachable.contains(id));
        let mut seen: HashMap<String, u32> = HashMap::new();
        for flat in &mut out {
            let count = seen.entry(flat.identifier.clone()).or_insert(0);
            *count += 1;
            if *count > 1 {
                flat.identifier = format!("{}#{count}", flat.identifier);
            }
        }
        out
    }

    fn visit(
        &self,
        id: NodeId,
        parent: Affine,
        root: bool,
        out: &mut Vec<Flat>,
        reachable: &mut HashSet<NodeId>,
    ) {
        let Some(node) = self.nodes.get(&id) else {
            return;
        };
        if !reachable.insert(id) || node.is_hidden() {
            return;
        }
        let transform = match (root, node.transform()) {
            (false, Some(own)) => parent * *own,
            _ => parent,
        };
        if !root && let Some(flat) = self.describe(id, node, transform) {
            out.push(flat);
        }
        for child in node.children() {
            self.visit(*child, transform, false, out, reachable);
        }
    }

    fn describe(&self, id: NodeId, node: &Node, transform: Affine) -> Option<Flat> {
        // Text runs repeat their label's text for selection geometry.
        if node.role() == Role::TextRun {
            return None;
        }
        let bounds = transform.transform_rect_bbox(node.bounds()?);
        let role = role(node.role());
        let value = node.value().unwrap_or_default().to_owned();
        let mut label = node.label().unwrap_or_default().to_owned();
        // egui puts a label widget's text in its value; that text is its name.
        if label.is_empty() && role == ffi::ROLE_STATIC_TEXT {
            label.clone_from(&value);
        }
        let clickable = node.supports_action(Action::Click);
        let actionable = clickable || node.supports_action(Action::Focus);
        let empty = bounds.width() <= 0.0 || bounds.height() <= 0.0;
        let named = !label.is_empty() || !value.is_empty();
        // Unnamed generic regions (drag areas, containers) are not elements.
        if empty || !named && (role == ffi::ROLE_OTHER || !actionable) {
            return None;
        }
        let identifier = match node.author_id() {
            Some(author) => author.to_owned(),
            None => format!("{}.{}", role_name(role), normalize(&label)),
        };
        Some(Flat {
            id: id.0,
            role,
            flags: flags(node, self.focus == Some(id)),
            rect: [
                bounds.x0 as f32,
                bounds.y0 as f32,
                bounds.width() as f32,
                bounds.height() as f32,
            ],
            identifier,
            label,
            value,
            clickable,
        })
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Rebuilds the C array; valid until the next call or drop.
    pub fn snapshot(&mut self) -> (*const A11yNode, u32) {
        let cached = self.snapshot_generation == Some(self.generation);
        onecad_ui::performance::record(
            onecad_ui::performance::Stage::SnapshotCache,
            self.generation,
            [u64::from(cached), self.flat.len() as u64, 0],
        );
        if cached {
            return (
                self.snapshot.as_ptr(),
                u32::try_from(self.snapshot.len()).unwrap_or(u32::MAX),
            );
        }
        #[cfg(test)]
        {
            self.snapshot_rebuilds += 1;
        }
        self.strings.clear();
        self.snapshot.clear();
        for flat in &self.flat {
            let mut text = |value: &str| {
                let value = CString::new(value.replace('\0', "")).unwrap_or_default();
                let ptr = value.as_ptr();
                self.strings.push(value);
                ptr
            };
            let node = A11yNode {
                id: flat.id,
                role: flat.role,
                flags: flat.flags,
                x: flat.rect[0],
                y: flat.rect[1],
                w: flat.rect[2],
                h: flat.rect[3],
                identifier: text(&flat.identifier),
                label: text(&flat.label),
                value: text(&flat.value),
            };
            self.snapshot.push(node);
        }
        let count = u32::try_from(self.snapshot.len()).unwrap_or(u32::MAX);
        self.snapshot_generation = Some(self.generation);
        (self.snapshot.as_ptr(), count)
    }

    pub fn activation(&self, id: u64) -> Option<Activation> {
        let flat = self.flat.iter().find(|flat| flat.id == id)?;
        Some(if flat.clickable {
            Activation::Click(NodeId(id))
        } else {
            let [x, y, w, h] = flat.rect;
            Activation::Tap(egui::pos2(x + w / 2.0, y + h / 2.0))
        })
    }

    /// The enabled clickable element under `at` (points) that was flattened
    /// last, i.e. the innermost and topmost: where a control tap that could not
    /// take egui's pointer is delivered as an AccessKit click.
    pub fn click_target(&self, at: egui::Pos2) -> Option<NodeId> {
        self.flat
            .iter()
            .rev()
            .find(|flat| {
                let [x, y, w, h] = flat.rect;
                flat.clickable
                    && flat.flags & ffi::A11Y_DISABLED == 0
                    && (x..=x + w).contains(&at.x)
                    && (y..=y + h).contains(&at.y)
            })
            .map(|flat| NodeId(flat.id))
    }

    #[cfg(test)]
    pub fn flat(&self) -> &[Flat] {
        &self.flat
    }
}

/// egui's AccessKit click on `node`: the widget clicks without the pointer.
pub fn click(node: NodeId) -> egui::Event {
    egui::Event::AccessKitActionRequest(accesskit::ActionRequest {
        action: Action::Click,
        target_tree: accesskit::TreeId::ROOT,
        target_node: node,
        data: None,
    })
}

#[cfg(test)]
#[path = "a11y_tests.rs"]
mod tests;
