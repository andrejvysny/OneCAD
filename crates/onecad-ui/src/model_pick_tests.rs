use super::*;
use crate::item_selection::model_kind;
use crate::tool_registry::{SelectionKind as PillKind, pill_tools};
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
fn multi_select_switch_toggles_hits_but_a_miss_clears_and_a_double_tap_replaces() {
    use SelectionMode::*;
    assert_eq!(additive(true, Replace, true, false), Toggle);
    assert_eq!(
        additive(true, Replace, false, false),
        Replace,
        "a miss clears"
    );
    assert_eq!(
        additive(true, Replace, true, true),
        Replace,
        "a double tap selects the body alone"
    );
    for mode in [Add, Toggle, Subtract] {
        assert_eq!(additive(true, mode, true, false), mode, "modifiers win");
        assert_eq!(additive(true, mode, false, false), mode, "modifiers win");
    }
    assert_eq!(additive(false, Replace, true, false), Replace);
    // What a Replace double tap does to the face its first tap toggled in.
    let body = BodyId::new();
    let mut selection = SelectionSet::default();
    selection.apply(Toggle, vec![face(body, "f:1")]).unwrap();
    let mode = additive(true, Replace, true, true);
    selection
        .apply(mode, vec![SelectionItem::Body(body)])
        .unwrap();
    assert_eq!(selection.items(), [SelectionItem::Body(body)]);
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
    let vertex = SelectionItem::Vertex(TopologyRef {
        scene: SCENE,
        body,
        id: topology_id("v:1"),
    });
    set.apply(SelectionMode::Replace, [vertex]).unwrap();
    assert_eq!(model_kind(&set), PillKind::None, "no tool takes a vertex");
}

/// Runs one frame with a fixed-rect row and returns the row mode it reports.
fn row_frame(ctx: &egui::Context, events: Vec<egui::Event>) -> (bool, SelectionMode) {
    let mut out = (false, SelectionMode::Replace);
    let input = egui::RawInput {
        events,
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        let response = ui.add_sized([200.0, 40.0], egui::Button::new("row"));
        out = (response.clicked(), row_mode(ui, &response));
    });
    output.textures_delta.clear();
    out
}

#[test]
fn row_mode_comes_from_the_press_even_when_the_release_frame_differs() {
    let ctx = egui::Context::default();
    let _ = row_frame(&ctx, Vec::new());
    let at = egui::pos2(20.0, 20.0);
    let (clicked, _) = row_frame(
        &ctx,
        vec![
            egui::Event::PointerMoved(at),
            press(at, true, egui::Modifiers::COMMAND),
        ],
    );
    assert!(!clicked);
    let (clicked, mode) = row_frame(
        &ctx,
        vec![
            egui::Event::ModifiersChanged(egui::Modifiers::SHIFT),
            press(at, false, egui::Modifiers::NONE),
        ],
    );
    assert!(clicked, "the release completes the row click");
    assert_eq!(mode, SelectionMode::Toggle, "press modifiers decide");
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
