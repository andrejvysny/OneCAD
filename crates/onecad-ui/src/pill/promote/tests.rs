use super::*;
use crate::tool_registry::{PLANNED_MODEL, SelectionKind, pill_tools};

fn model(id: ToolId) -> ToolDef {
    *PLANNED_MODEL.iter().find(|d| d.id == id).unwrap()
}

fn ids(groups: &[Vec<ToolDef>]) -> Vec<Vec<ToolId>> {
    groups
        .iter()
        .map(|g| g.iter().map(|d| d.id).collect())
        .collect()
}

#[test]
fn promotes_enabled_tools_before_utilities_and_skips_drawn() {
    let base = pill_tools(SelectionKind::Region);
    let enabled = [
        model(ToolId::Extrude),
        model(ToolId::Hole),
        model(ToolId::Shell),
    ];
    let out = compose(base, &enabled, 10_000.0, false);
    assert_eq!(
        ids(&out),
        [
            vec![ToolId::Extrude],
            vec![ToolId::Hole, ToolId::Shell],
            vec![ToolId::Import, ToolId::More]
        ]
    );
}

#[test]
fn no_enabled_tools_leaves_registry_groups() {
    let base = pill_tools(SelectionKind::Edge);
    let out = compose(base, &[], 10_000.0, false);
    assert_eq!(out.len(), base.len());
}

#[test]
fn datum_plane_sits_next_to_sketch() {
    for (kind, sketch_group) in [(SelectionKind::None, 1), (SelectionKind::Face, 0)] {
        let enabled = [model(ToolId::Hole), model(ToolId::DatumPlane)];
        let out = ids(&compose(pill_tools(kind), &enabled, 10_000.0, false));
        assert_eq!(out[sketch_group][..2], [ToolId::Sketch, ToolId::DatumPlane]);
        let promoted = &out[out.len() - 2];
        assert!(promoted.contains(&ToolId::Hole));
        assert!(!promoted.contains(&ToolId::DatumPlane));
    }
}

#[test]
fn width_budget_drops_the_tail_only() {
    let base = pill_tools(SelectionKind::Region);
    let enabled = [
        model(ToolId::Hole),
        model(ToolId::Shell),
        model(ToolId::Draft),
    ];
    // Extrude, Import, More and 3 promoted tools in 3 groups.
    let full = required_width(6, 3, false);
    let ids_at = |room: f32| ids(&compose(base, &enabled, room, false));
    assert_eq!(ids_at(full)[1].len(), 3);
    assert_eq!(ids_at(full - 1.0)[1], [ToolId::Hole, ToolId::Shell]);
    assert_eq!(
        ids_at(0.0),
        [vec![ToolId::Extrude], vec![ToolId::Import, ToolId::More]]
    );
}

#[test]
fn dropped_tools_stay_in_overflow_with_disabled_ones() {
    let base = pill_tools(SelectionKind::Region);
    let out = compose(base, &[model(ToolId::Hole)], 0.0, false);
    let drawn: Vec<ToolId> = out.iter().flatten().map(|d| d.id).collect();
    let refs: Vec<&[ToolDef]> = out.iter().map(Vec::as_slice).collect();
    let rows = crate::pill::catalog::overflow(&refs, PLANNED_MODEL, &drawn);
    assert!(rows.iter().any(|d| d.id == ToolId::Hole));
    assert!(rows.iter().any(|d| d.id == ToolId::Sweep));
}

#[test]
fn desktop_has_no_multi_select_switch() {
    assert_eq!(
        crate::pill::additive_visible(false, true),
        cfg!(target_os = "ios")
    );
    assert!(!crate::pill::additive_visible(true, true));
}
