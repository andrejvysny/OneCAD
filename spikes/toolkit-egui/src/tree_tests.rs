use super::*;
use crate::mesh1::Mesh1;
use egui::accesskit::{Action, Node, NodeId, TreeUpdate};
use egui::{Context, Event, Key, Modifiers, RawInput};

struct Harness {
    ctx: Context,
    bodies: Vec<Body>,
    selected: Option<usize>,
    time: f64,
    other_text: String,
}

impl Harness {
    fn new() -> Self {
        let ctx = Context::default();
        ctx.enable_accesskit();
        Self {
            ctx,
            bodies: (1..=3).map(body).collect(),
            selected: None,
            time: 0.0,
            other_text: String::new(),
        }
    }

    fn frame(&mut self, events: Vec<Event>) -> TreeUpdate {
        self.time += 0.1;
        let output = self.ctx.run_ui(
            RawInput {
                events,
                time: Some(self.time),
                ..Default::default()
            },
            |ui| {
                ui.set_width(220.0);
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                show(
                    ui,
                    &crate::theme::dark(),
                    &mut self.bodies,
                    &mut self.selected,
                );
                let other = ui.text_edit_singleline(&mut self.other_text);
                ui.ctx()
                    .accesskit_node_builder(other.id, |node| node.set_label("Other input"));
            },
        );
        let update = output.platform_output.accesskit_update.clone().unwrap();
        output.drop_without_applying_deltas();
        update
    }
}

fn body(index: usize) -> Body {
    Body {
        label: format!("Body {index}"),
        file: format!("body-{index}.mesh1"),
        visible: true,
        mesh: Mesh1 {
            vertex_count: 0,
            triangle_count: 0,
            edge_count: 0,
            bbox_min: [0.0; 3],
            bbox_max: [0.0; 3],
            positions: Vec::new(),
            normals: Vec::new(),
            indices: Vec::new(),
            edge_segments: Vec::new(),
        },
    }
}

fn named<'a>(update: &'a TreeUpdate, label: &str) -> (NodeId, &'a Node) {
    update
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some(label))
        .map(|(id, node)| (*id, node))
        .unwrap()
}

#[test]
fn body_selection_is_selected_not_checked() {
    let mut h = Harness::new();
    h.selected = Some(1);
    let update = h.frame(Vec::new());
    let (_, node) = named(&update, "Body 2");
    assert_eq!(node.role(), egui::Role::TreeItem);
    assert_eq!(node.is_selected(), Some(true));
    assert_eq!(node.toggled(), None);
    assert!(node.supports_action(Action::Focus));
    assert!(node.supports_action(Action::Click));
}

fn key(key: Key) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}

fn action(action: Action, target_node: NodeId) -> Event {
    Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
        action,
        target_node,
        target_tree: egui::accesskit::TreeId::ROOT,
        data: None,
    })
}

fn focus(h: &mut Harness, label: &str) -> NodeId {
    let update = h.frame(Vec::new());
    let (id, _) = named(&update, label);
    let update = h.frame(vec![action(Action::Focus, id)]);
    assert_eq!(update.focus, id);
    h.frame(Vec::new());
    id
}

#[test]
fn tree_owns_named_rows_and_visibility_controls() {
    let mut h = Harness::new();
    let update = h.frame(Vec::new());
    let (_, tree) = named(&update, "Bodies");
    assert_eq!(tree.role(), egui::Role::Tree);
    assert_eq!(tree.size_of_set(), Some(3));
    for index in 1..=3 {
        let (id, body) = named(&update, &format!("Body {index}"));
        assert!(tree.children().contains(&id));
        assert_eq!(body.is_selected(), Some(false));
        assert_eq!(body.level(), Some(0));
        assert_eq!(body.position_in_set(), Some(index - 1));
        let (eye_id, eye) = named(&update, &format!("Show Body {index}"));
        assert!(body.children().contains(&eye_id));
        assert_eq!(eye.role(), egui::Role::CheckBox);
        assert_eq!(eye.toggled(), Some(egui::accesskit::Toggled::True));
    }
}

#[test]
fn keyboard_navigation_selects_rows_and_stops_at_ends() {
    let mut h = Harness::new();
    focus(&mut h, "Body 1");
    for (key, selected) in [
        (Key::ArrowUp, 0),
        (Key::ArrowDown, 1),
        (Key::End, 2),
        (Key::ArrowDown, 2),
        (Key::ArrowUp, 1),
        (Key::Home, 0),
    ] {
        let update = h.frame(vec![self::key(key)]);
        assert_eq!(h.selected, Some(selected));
        let (id, node) = named(&update, &format!("Body {}", selected + 1));
        assert_eq!(update.focus, id);
        assert_eq!(node.is_selected(), Some(true));
    }
}

#[test]
fn enter_space_and_accessibility_click_preserve_toggle_selection() {
    let mut h = Harness::new();
    let id = focus(&mut h, "Body 2");
    h.frame(vec![key(Key::Enter)]);
    assert_eq!(h.selected, Some(1));
    h.frame(vec![key(Key::Space)]);
    assert_eq!(h.selected, None);
    h.frame(vec![action(Action::Click, id)]);
    assert_eq!(h.selected, Some(1));
    h.frame(vec![action(Action::Click, id)]);
    assert_eq!(h.selected, None);
}

#[test]
fn visibility_actions_change_visibility_without_selecting_body() {
    let mut h = Harness::new();
    h.selected = Some(2);
    let id = focus(&mut h, "Show Body 1");
    h.frame(vec![key(Key::Space)]);
    assert!(!h.bodies[0].visible);
    assert_eq!(h.selected, Some(2));
    h.frame(vec![action(Action::Click, id)]);
    assert!(h.bodies[0].visible);
    assert_eq!(h.selected, Some(2));
}

#[test]
fn tab_moves_focus_between_row_visibility_and_next_row() {
    let mut h = Harness::new();
    focus(&mut h, "Body 1");
    let update = h.frame(vec![key(Key::Tab)]);
    assert_eq!(update.focus, named(&update, "Show Body 1").0);
    let update = h.frame(vec![key(Key::Tab)]);
    assert_eq!(update.focus, named(&update, "Body 2").0);
    assert_eq!(h.selected, None);
}

#[test]
fn empty_tree_has_no_keyboard_target() {
    let mut h = Harness::new();
    h.bodies.clear();
    let update = h.frame(vec![key(Key::Home), key(Key::End), key(Key::ArrowDown)]);
    let (_, tree) = named(&update, "Bodies");
    assert_eq!(tree.size_of_set(), Some(0));
    assert!(tree.children().is_empty());
    assert_eq!(h.selected, None);
}

fn click(h: &mut Harness, label: &str) {
    let update = h.frame(Vec::new());
    let bounds = named(&update, label).1.bounds().unwrap();
    let position = egui::pos2(
        ((bounds.x0 + bounds.x1) / 2.0) as f32,
        ((bounds.y0 + bounds.y1) / 2.0) as f32,
    );
    for pressed in [true, false] {
        h.frame(vec![
            Event::PointerMoved(position),
            Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            },
        ]);
    }
}

#[test]
fn pointer_rows_toggle_selection_eye_clicks_do_not() {
    let mut h = Harness::new();
    click(&mut h, "Body 2");
    assert_eq!(h.selected, Some(1));
    click(&mut h, "Body 2");
    assert_eq!(h.selected, None);
    click(&mut h, "Show Body 2");
    assert!(!h.bodies[1].visible);
    assert_eq!(h.selected, None);
}

#[test]
fn unrelated_text_input_owns_navigation_and_activation() {
    let mut h = Harness::new();
    h.selected = Some(1);
    focus(&mut h, "Other input");
    let update = h.frame(vec![
        Event::Text("abc".into()),
        key(Key::Home),
        key(Key::Space),
    ]);
    assert_eq!(h.selected, Some(1));
    assert_eq!(update.focus, named(&update, "Other input").0);
    assert_eq!(h.other_text, "abc");
}

#[test]
fn row_ids_survive_selection_visibility_and_name_changes() {
    let mut h = Harness::new();
    let before = h.frame(Vec::new());
    h.selected = Some(1);
    h.bodies[1].visible = false;
    h.bodies[1].label = "Motor housing".into();
    let after = h.frame(Vec::new());
    assert_eq!(named(&before, "Body 2").0, named(&after, "Motor housing").0);
    assert_eq!(
        named(&before, "Show Body 2").0,
        named(&after, "Show Motor housing").0
    );
}

#[test]
fn selecting_later_row_clears_previous_accessible_selection_in_same_frame() {
    let mut h = Harness::new();
    h.selected = Some(0);
    let initial = h.frame(Vec::new());
    let id = named(&initial, "Body 3").0;
    let update = h.frame(vec![action(Action::Click, id)]);
    for index in 1..=3 {
        assert_eq!(
            named(&update, &format!("Body {index}")).1.is_selected(),
            Some(index == 3)
        );
    }
}

#[test]
fn leaf_arrows_do_not_move_focus_or_change_selection() {
    let mut h = Harness::new();
    let id = focus(&mut h, "Body 2");
    for key in [Key::ArrowLeft, Key::ArrowRight] {
        let update = h.frame(vec![self::key(key)]);
        assert_eq!(update.focus, id);
        assert_eq!(h.selected, None);
    }
}

#[test]
fn tab_leaves_tree_for_next_input() {
    let mut h = Harness::new();
    focus(&mut h, "Show Body 3");
    let update = h.frame(vec![key(Key::Tab)]);
    assert_eq!(update.focus, named(&update, "Other input").0);
}
