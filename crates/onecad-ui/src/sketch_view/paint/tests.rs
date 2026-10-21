use super::*;
use crate::sketch_view::selection::tests::{add_point, at, session};
use onecad_core::ids::EntityId;

fn theme() -> Theme {
    onecad_widgets::theme::load(false).unwrap()
}

#[test]
fn hover_resolves_pick_for_select_tool_and_none_with_draft() {
    let (mut session, rect, view) = session();
    let id = add_point(&mut session, 0.0, 0.0);
    let position = at(&session, rect, &view, 0.0, 0.0);
    let target = hover_target(rect, &view, &session, Some(position));
    assert!(matches!(target, Some(SketchTarget::Point { entity, .. }) if entity == id));
    assert_eq!(hover_target(rect, &view, &session, None), None);
    session.tool = SketchTool::Line;
    assert_eq!(hover_target(rect, &view, &session, Some(position)), None);
    session.tool = SketchTool::Select;
    session.selection_dragging = true;
    assert_eq!(hover_target(rect, &view, &session, Some(position)), None);
}

#[test]
fn hover_is_none_while_draft_exists() {
    let (mut session, rect, view) = session();
    add_point(&mut session, 0.0, 0.0);
    let position = at(&session, rect, &view, 0.0, 0.0);
    let origin = Vec2::new(0.0, 0.0).unwrap();
    session.draft = Some(onecad_interaction::sketch::SketchDraft {
        first: origin,
        anchor: origin,
        first_id: None,
        first_target: None,
        anchor_id: None,
        cursor: origin,
        segments: 0,
        anchor_snap: None,
        secondary: None,
        preceding: None,
        tangent_arc: false,
    });
    assert_eq!(hover_target(rect, &view, &session, Some(position)), None);
}

#[test]
fn selected_and_state_colors_are_distinct_from_neutral() {
    let theme = theme();
    let entity = SketchEntity::point(EntityId::new(), Vec2::new(0.0, 0.0).unwrap(), false, false);
    let selected = entity_color(&theme, &entity, true, None);
    let neutral = entity_color(&theme, &entity, false, None);
    assert_eq!(selected, theme.accent);
    assert_eq!(neutral, theme.sketch_neutral);
    assert_ne!(selected, neutral);
    let states = [
        (EntityConstrainedState::FullyConstrained, theme.dof_ok),
        (
            EntityConstrainedState::UnderConstrained,
            theme.sketch_underconstrained,
        ),
        (EntityConstrainedState::Conflicting, theme.danger_strong),
    ];
    for (state, expected) in states {
        let color = entity_color(&theme, &entity, false, Some(state));
        assert_eq!(color, expected);
        assert_ne!(color, neutral);
        assert_ne!(color, selected);
    }
    assert_eq!(
        entity_color(
            &theme,
            &entity,
            true,
            Some(EntityConstrainedState::Conflicting)
        ),
        theme.accent
    );
}
