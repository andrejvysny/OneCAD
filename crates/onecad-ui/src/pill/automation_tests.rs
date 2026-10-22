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

#[test]
fn multi_select_switch_reports_its_state() {
    for (on, state) in [(false, Toggled::False), (true, Toggled::True)] {
        let named = author_ids(|ui| {
            additive_button(ui, on);
        });
        let (_, role, toggled) = find(&named, crate::automation::ADDITIVE_SELECT);
        assert_eq!((*role, *toggled), (Role::Button, Some(state)));
    }
}
