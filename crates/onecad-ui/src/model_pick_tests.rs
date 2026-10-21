use super::*;
use crate::tool_registry::{SelectionKind as PillKind, model_kind, pill_tools};
use onecad_core::ids::{BodyId, RecordId};
use onecad_interaction::selection_set::{OperandRole, OperandStage, SelectionSet};
use onecad_scene::SceneIdentity;

const SCENE: SceneIdentity = SceneIdentity {
    session: 1,
    scene: 1,
};

fn press(pos: egui::Pos2, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers,
    }
}

fn face(body: BodyId, key: &str) -> SelectionItem {
    SelectionItem::Face(TopologyRef {
        scene: SCENE,
        body,
        id: topology_id(key),
    })
}

fn edge(body: BodyId, key: &str) -> SelectionItem {
    SelectionItem::Edge(TopologyRef {
        scene: SCENE,
        body,
        id: topology_id(key),
    })
}

#[test]
fn each_modifier_maps_to_one_selection_mode() {
    let shift = egui::Modifiers::SHIFT;
    let command = egui::Modifiers::COMMAND;
    assert_eq!(mode_for(egui::Modifiers::NONE), SelectionMode::Replace);
    assert_eq!(mode_for(shift), SelectionMode::Add);
    assert_eq!(mode_for(command), SelectionMode::Toggle);
    assert_eq!(mode_for(shift | command), SelectionMode::Subtract);
}

#[test]
fn batched_press_modifier_change_and_release_uses_the_press_modifiers() {
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100.0, 100.0));
    let at = egui::pos2(10.0, 10.0);
    let batch = [
        press(at, true, egui::Modifiers::SHIFT),
        egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        },
        press(at, false, egui::Modifiers::NONE),
    ];
    assert_eq!(press_mode(&batch, rect), Some(SelectionMode::Add));
    let outside = [press(
        egui::pos2(200.0, 10.0),
        true,
        egui::Modifiers::COMMAND,
    )];
    assert_eq!(press_mode(&outside, rect), None);
    let two = [
        press(at, true, egui::Modifiers::SHIFT),
        press(at, false, egui::Modifiers::SHIFT),
        press(at, true, egui::Modifiers::COMMAND),
    ];
    assert_eq!(press_mode(&two, rect), Some(SelectionMode::Toggle));
}

#[test]
fn mesh_labels_classify_into_snapshot_keys_and_element_ids() {
    assert!(matches!(topology_id("f:12"), TopologyId::Snapshot(_)));
    assert!(matches!(topology_id("el_abc"), TopologyId::Element(_)));
    assert_eq!(label(&topology_id("e:3")), "e:3");
    assert_eq!(label(&topology_id("el_abc")), "el_abc");
}

#[test]
fn modifier_gestures_build_multi_selection_and_mixed_kinds_offer_no_scoped_tools() {
    let body = BodyId::new();
    let mut set = SelectionSet::default();
    set.apply(SelectionMode::Replace, [face(body, "f:1")])
        .unwrap();
    set.apply(SelectionMode::Add, [face(body, "f:2")]).unwrap();
    set.apply(SelectionMode::Toggle, [face(body, "f:3")])
        .unwrap();
    assert_eq!(set.len(), 3);
    assert_eq!(model_kind(&set), PillKind::Face);
    set.apply(SelectionMode::Toggle, [face(body, "f:3")])
        .unwrap();
    set.apply(SelectionMode::Subtract, [face(body, "f:1")])
        .unwrap();
    assert_eq!(set.items(), [face(body, "f:2")]);
    set.apply(SelectionMode::Add, [edge(body, "e:1")]).unwrap();
    assert_eq!(model_kind(&set), PillKind::Mixed);
    assert!(std::ptr::eq(
        pill_tools(PillKind::Mixed),
        pill_tools(PillKind::None)
    ));
    set.apply(
        SelectionMode::Replace,
        [SelectionItem::Feature(RecordId::new())],
    )
    .unwrap();
    assert_eq!(model_kind(&set), PillKind::Feature);
    set.clear();
    assert_eq!(model_kind(&set), PillKind::None);
}

#[test]
fn replace_refused_by_the_stage_keeps_staged_operands() {
    let body = BodyId::new();
    let stage = OperandStage::new(OperandRole::Edges, [SelectionKind::Edge], 1, None);
    let mut operands = StagedOperands::new(SCENE, vec![stage]).unwrap();
    operands.offer(edge(body, "e:1")).unwrap();
    operands.offer(edge(body, "e:2")).unwrap();
    assert!(matches!(
        replace_active(&mut operands, face(body, "f:1")),
        Err(OperandRefusal::WrongKind { .. })
    ));
    assert_eq!(operands.active().items.len(), 2);
    replace_active(&mut operands, edge(body, "e:3")).unwrap();
    assert_eq!(operands.active().items, [edge(body, "e:3")]);
}
