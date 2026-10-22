//! Pill tool buttons and the multi-select switch carry stable automation ids.
use super::*;
use crate::automation::testing::{author_ids, find};
use egui::accesskit::{Role, Toggled};

fn enabled(groups: Groups<'_>) -> Vec<Vec<View>> {
    groups
        .iter()
        .map(|group| group.iter().map(|_| gate(true, "")).collect())
        .collect()
}

#[test]
fn selection_groups_name_every_tool_by_registry_id() {
    for (kind, expected) in [
        (
            SelectionKind::Edge,
            &["fillet", "chamfer", "import", "more"][..],
        ),
        (
            SelectionKind::Face,
            &["sketch", "offsetFace", "import", "more"][..],
        ),
        (SelectionKind::Body, &["moveRotate", "import", "more"][..]),
        (SelectionKind::Region, &["extrude", "import", "more"][..]),
        (
            SelectionKind::SketchIdle,
            &["select", "line", "rectangle", "circle", "dimension"][..],
        ),
    ] {
        let groups = pill_tools(kind);
        let views = enabled(groups);
        let named = author_ids(|ui| {
            ui.horizontal(|ui| {
                draw_groups(ui, groups, &views, None);
            });
        });
        for tool in expected {
            assert_eq!(find(&named, &format!("onecad.tool.{tool}")).1, Role::Button);
        }
    }
}

/// A narrow lane draws only [preferred, More]; the rest must stay addressable
/// by the same ids as rows of the More catalog, each exactly once.
#[test]
fn compact_pill_and_more_catalog_name_each_selection_tool_once() {
    for (kind, moved, expected) in [
        (
            SelectionKind::Edge,
            ToolId::Chamfer,
            &["fillet", "chamfer", "import", "more", "offsetFace"][..],
        ),
        (
            SelectionKind::Face,
            ToolId::OffsetFace,
            &["sketch", "offsetFace", "import", "more", "fillet"][..],
        ),
    ] {
        let groups = pill_tools(kind);
        // 120 pt is the narrowest lane `toolbar_bounds` yields.
        let compact = layout::compact_tools(groups, None, 120.0, true).expect("compact lane");
        let drawn: Vec<ToolId> = compact.iter().map(|def| def.id).collect();
        assert!(!drawn.contains(&moved), "{drawn:?}");
        let pill = [&compact[..]];
        let views = enabled(&pill);
        let named = author_ids(|ui| {
            ui.horizontal(|ui| {
                draw_groups(ui, &pill, &views, None);
            });
            let rows = catalog::overflow(groups, crate::tool_registry::PLANNED_MODEL, &drawn);
            for def in rows {
                catalog::row(ui, def, true);
            }
        });
        for tool in expected {
            assert_eq!(find(&named, &format!("onecad.tool.{tool}")).1, Role::Button);
        }
    }
}

#[test]
fn multi_select_switch_reports_its_state() {
    for (on, state) in [(false, Toggled::False), (true, Toggled::True)] {
        let named = author_ids(|ui| {
            multi_select::button(ui, on);
        });
        let (_, role, toggled) = find(&named, crate::automation::ADDITIVE_SELECT);
        assert_eq!((*role, *toggled), (Role::Button, Some(state)));
    }
}
