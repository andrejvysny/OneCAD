use super::*;
use egui::{PointerButton as B, vec2};

#[test]
fn defaults_match_three_button_preset() {
    let p = NavigationPrefs::default();
    let mut q = p.clone();
    q.apply_preset(NavPreset::ThreeButtonMouse);
    assert_eq!(p, q);
    assert!(p.zoom_toward_cursor && !p.invert_scroll_zoom);
    assert_eq!(p.validate(), Ok(()));
}

#[test]
fn presets_overwrite_mapping_but_not_zoom_options() {
    let mut p = NavigationPrefs {
        zoom_toward_cursor: false,
        invert_scroll_zoom: true,
        ..Default::default()
    };
    p.apply_preset(NavPreset::Trackpad);
    assert_eq!(
        (p.orbit, p.pan, p.pan_alt, p.fit_on_middle_double_click),
        (DragInput::RightDrag, DragInput::ShiftRightDrag, None, false)
    );
    assert!(!p.zoom_toward_cursor && p.invert_scroll_zoom);
    p.apply_preset(NavPreset::Custom);
    assert_eq!(p.preset, NavPreset::Custom);
    assert_eq!(p.pan, DragInput::ShiftRightDrag);
    p.reset_to_preset();
    assert_eq!(
        p,
        NavigationPrefs {
            zoom_toward_cursor: false,
            invert_scroll_zoom: true,
            ..Default::default()
        }
    );
    p.apply_preset(NavPreset::Trackpad);
    p.assign(MappingSlot::Pan, Some(DragInput::MiddleDrag));
    p.reset_to_preset();
    assert_eq!(p.preset, NavPreset::ThreeButtonMouse);
}

#[test]
fn assign_swaps_clears_and_refuses() {
    let mut p = NavigationPrefs::default();
    assert!(!p.assign(MappingSlot::Orbit, Some(DragInput::RightDrag)));
    assert_eq!(p.preset, NavPreset::ThreeButtonMouse);
    assert!(!p.assign(MappingSlot::Orbit, None));
    assert!(!p.assign(MappingSlot::Pan, None));
    // Swap with the other required slot.
    assert!(p.assign(MappingSlot::Orbit, Some(DragInput::MiddleDrag)));
    assert_eq!(
        (p.orbit, p.pan),
        (DragInput::MiddleDrag, DragInput::RightDrag)
    );
    assert_eq!(p.preset, NavPreset::Custom);
    // Taking PanAlt's input clears PanAlt.
    assert!(p.assign(MappingSlot::Pan, Some(DragInput::ShiftRightDrag)));
    assert_eq!(p.pan_alt, None);
    assert_eq!(p.validate(), Ok(()));
    // PanAlt cannot steal a required slot's input.
    let before = p.clone();
    assert!(!p.assign(MappingSlot::PanAlt, Some(DragInput::MiddleDrag)));
    assert_eq!(p, before);
    assert!(p.assign(MappingSlot::PanAlt, Some(DragInput::ShiftMiddleDrag)));
    assert!(p.assign(MappingSlot::PanAlt, None));
    assert_eq!(p.pan_alt, None);
    assert!(!p.assign(MappingSlot::PanAlt, None));
}

#[test]
fn fit_toggle_marks_custom_only_on_change() {
    let mut p = NavigationPrefs::default();
    p.set_fit_on_middle_double_click(true);
    assert_eq!(p.preset, NavPreset::ThreeButtonMouse);
    p.set_fit_on_middle_double_click(false);
    assert_eq!(p.preset, NavPreset::Custom);
}

#[test]
fn drag_action_matches_exact_then_falls_back_when_shift_unmapped() {
    let p = NavigationPrefs::default();
    assert_eq!(p.drag_action(B::Secondary, false), Some(DragAction::Orbit));
    assert_eq!(p.drag_action(B::Secondary, true), Some(DragAction::Pan));
    assert_eq!(p.drag_action(B::Middle, false), Some(DragAction::Pan));
    // ⇧ middle unmapped: falls back to plain middle.
    assert_eq!(p.drag_action(B::Middle, true), Some(DragAction::Pan));
    assert_eq!(p.drag_action(B::Primary, false), None);
    assert_eq!(p.drag_action(B::Primary, true), None);

    let mut t = NavigationPrefs::default();
    t.apply_preset(NavPreset::Trackpad);
    assert_eq!(t.drag_action(B::Middle, false), None);
    assert_eq!(t.drag_action(B::Middle, true), None);
    assert_eq!(t.drag_action(B::Secondary, true), Some(DragAction::Pan));

    // Exact shifted match wins over the unshifted fallback.
    let mut c = NavigationPrefs::default();
    c.assign(MappingSlot::Pan, Some(DragInput::ShiftMiddleDrag));
    assert_eq!(c.drag_action(B::Middle, true), Some(DragAction::Pan));
    assert_eq!(c.drag_action(B::Middle, false), None);
}

#[test]
fn validate_rejects_collisions() {
    let mut p = NavigationPrefs::default();
    p.pan = p.orbit;
    assert!(p.validate().is_err());
    let mut p = NavigationPrefs::default();
    p.pan_alt = Some(p.orbit);
    assert!(p.validate().is_err());
    p.pan_alt = Some(p.pan);
    assert!(p.validate().is_err());
}

#[test]
fn serde_roundtrip_and_strictness() {
    let mut p = NavigationPrefs::default();
    p.assign(MappingSlot::Orbit, Some(DragInput::ShiftMiddleDrag));
    let json = serde_json::to_string(&p).unwrap();
    assert!(json.contains("\"shiftMiddleDrag\"") && json.contains("\"panAlt\""));
    assert_eq!(serde_json::from_str::<NavigationPrefs>(&json).unwrap(), p);
    assert_eq!(
        serde_json::from_str::<NavigationPrefs>("{}").unwrap(),
        NavigationPrefs::default()
    );
    assert!(serde_json::from_str::<NavigationPrefs>("{\"bogus\":1}").is_err());
}

#[test]
fn drag_motion_follows_mapping() {
    let p = NavigationPrefs::default();
    let d = vec2(3.0, 4.0);
    assert_eq!(
        drag_motion(&p, true, false, false, d),
        Some(Motion::Orbit(d))
    );
    assert_eq!(drag_motion(&p, true, false, true, d), Some(Motion::Pan(d)));
    assert_eq!(drag_motion(&p, false, true, false, d), Some(Motion::Pan(d)));
    assert_eq!(drag_motion(&p, true, true, false, d), Some(Motion::Pan(d)));
    assert_eq!(drag_motion(&p, false, false, false, d), None);
    let mut t = p.clone();
    t.apply_preset(NavPreset::Trackpad);
    assert_eq!(drag_motion(&t, false, true, false, d), None);
}

#[test]
fn invert_flips_wheel_zoom_but_not_pinch_or_other_motion() {
    let mut p = NavigationPrefs::default();
    assert_eq!(wheel_motion(&p, true, Motion::Zoom(2.0)), Motion::Zoom(2.0));
    p.invert_scroll_zoom = true;
    assert_eq!(wheel_motion(&p, true, Motion::Zoom(2.0)), Motion::Zoom(0.5));
    assert_eq!(
        wheel_motion(&p, false, Motion::Zoom(2.0)),
        Motion::Zoom(2.0)
    );
    let pan = || Motion::Pan(vec2(1.0, 1.0));
    assert_eq!(wheel_motion(&p, true, pan()), pan());
}

#[test]
fn zoom_anchor_is_dropped_when_disabled() {
    let mut p = NavigationPrefs::default();
    assert_eq!(zoom_anchor(&p, Some(5)), Some(5));
    p.zoom_toward_cursor = false;
    assert_eq!(zoom_anchor(&p, Some(5)), None);
}
