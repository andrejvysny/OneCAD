use super::*;
use egui::accesskit::{Rect, TreeId, TreeInfo};

fn node(role: Role, label: &str, bounds: Option<Rect>, click: bool) -> Node {
    let mut node = Node::new(role);
    if !label.is_empty() {
        node.set_label(label);
    }
    if let Some(bounds) = bounds {
        node.set_bounds(bounds);
    }
    if click {
        node.add_action(Action::Click);
    }
    node
}

fn rect(x: f64, y: f64) -> Option<Rect> {
    Some(Rect::new(x, y, x + 20.0, y + 10.0))
}

fn update(nodes: Vec<(NodeId, Node)>, focus: u64) -> TreeUpdate {
    TreeUpdate {
        nodes,
        tree: Some(TreeInfo::new(NodeId(1))),
        tree_id: TreeId::ROOT,
        focus: NodeId(focus),
    }
}

fn window(children: &[u64]) -> Node {
    let mut root = Node::new(Role::Window);
    root.set_transform(Affine::scale(2.0));
    root.set_children(children.iter().map(|id| NodeId(*id)).collect::<Vec<_>>());
    root
}

#[test]
fn flattening_keeps_points_identifiers_and_drops_hidden_or_unlabeled_nodes() {
    let mut tree = Tree::default();
    let mut hidden = node(Role::Button, "Ghost", rect(0.0, 0.0), true);
    hidden.set_hidden();
    let mut disabled = node(Role::MenuItem, "Save As…", rect(5.0, 40.0), true);
    disabled.set_disabled();
    let changed = tree.update(update(
        vec![
            (NodeId(1), window(&[2, 3, 4, 5, 6, 7])),
            (
                NodeId(2),
                node(Role::Button, "Undo", rect(10.0, 20.0), true),
            ),
            (
                NodeId(3),
                node(Role::Button, "Undo", rect(40.0, 20.0), true),
            ),
            (NodeId(4), hidden),
            (
                NodeId(5),
                node(Role::GenericContainer, "", rect(0.0, 0.0), false),
            ),
            (NodeId(6), disabled),
            (
                NodeId(7),
                node(Role::Label, "Ready", rect(0.0, 90.0), false),
            ),
        ],
        3,
    ));
    assert!(changed);
    let ids: Vec<_> = tree.flat().iter().map(|f| f.identifier.as_str()).collect();
    assert_eq!(
        ids,
        [
            "button.undo",
            "button.undo#2",
            "menu_item.save-as",
            "static_text.ready"
        ]
    );
    let first = &tree.flat()[0];
    assert_eq!(first.rect, [10.0, 20.0, 20.0, 10.0], "points, not pixels");
    assert_eq!(tree.flat()[1].flags, ffi::A11Y_FOCUSED);
    assert_eq!(tree.flat()[2].flags, ffi::A11Y_DISABLED);
    assert!(matches!(
        tree.activation(2),
        Some(Activation::Click(NodeId(2)))
    ));
    assert!(matches!(
        tree.activation(7),
        Some(Activation::Tap(pos)) if pos == egui::pos2(10.0, 95.0)
    ));
    assert!(tree.activation(4).is_none());
    let generation = tree.generation();
    let same = update(vec![(NodeId(1), window(&[2, 3, 4, 5, 6, 7]))], 3);
    assert!(!tree.update(same));
    assert_eq!(tree.generation(), generation);
}

#[test]
fn removed_widgets_leave_the_snapshot_and_strings_are_c_safe() {
    let mut tree = Tree::default();
    tree.update(update(
        vec![
            (NodeId(1), window(&[2])),
            (
                NodeId(2),
                node(Role::Button, "Fi\0le", rect(0.0, 0.0), true),
            ),
        ],
        1,
    ));
    let (nodes, count) = tree.snapshot();
    assert_eq!(count, 1);
    // SAFETY: `snapshot` returned `count` initialized nodes that `tree` keeps alive.
    let label = unsafe { std::ffi::CStr::from_ptr((*nodes).label) };
    assert_eq!(label.to_str().unwrap(), "File");
    assert!(tree.update(update(vec![(NodeId(1), window(&[]))], 1)));
    assert_eq!(tree.snapshot().1, 0);
    let mut text = node(Role::Label, "", rect(0.0, 0.0), false);
    text.set_value("Saved plate");
    text.set_children(vec![NodeId(4)]);
    let run = node(Role::TextRun, "Saved plate", rect(0.0, 0.0), false);
    let drag = node(Role::Unknown, "", rect(0.0, 0.0), true);
    let nodes = vec![
        (NodeId(1), window(&[3, 5])),
        (NodeId(3), text),
        (NodeId(4), run),
        (NodeId(5), drag),
    ];
    tree.update(update(nodes, 1));
    let ids: Vec<_> = tree.flat().iter().map(|f| f.identifier.as_str()).collect();
    assert_eq!(
        ids,
        ["static_text.saved-plate"],
        "text runs and unnamed regions are skipped"
    );
    assert_eq!(tree.flat()[0].label, "Saved plate");
    assert_eq!(normalize("⌘ Export STEP…"), "export-step");
    assert_eq!(normalize("…"), "unlabeled");
}
