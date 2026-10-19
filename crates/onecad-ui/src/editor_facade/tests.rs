use super::*;
use egui::{Event, Key, Modifiers};

fn press(key: Key, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

#[test]
fn escape_and_enter_end_placement_and_are_consumed() {
    for key in [Key::Escape, Key::Enter] {
        let mut f = EditorFacade::default();
        f.begin_placement("Mounting bracket");
        let mut events = vec![press(key, Modifiers::NONE)];
        assert!(!process_keys(&mut f, &mut events));
        assert!(f.placing.is_none());
        assert!(events.is_empty());
    }
}

#[test]
fn escape_passes_through_when_not_placing() {
    let mut f = EditorFacade::default();
    let mut events = vec![press(Key::Escape, Modifiers::NONE)];
    process_keys(&mut f, &mut events);
    assert_eq!(events.len(), 1);
}

#[test]
fn escape_closes_popover_first() {
    let mut f = EditorFacade {
        popover_open: true,
        ..EditorFacade::default()
    };
    f.begin_placement("x");
    let mut events = vec![press(Key::Escape, Modifiers::NONE)];
    process_keys(&mut f, &mut events);
    assert!(!f.popover_open && f.placing.is_some());
}

#[test]
fn shift_l_requests_library_page() {
    let mut f = EditorFacade::default();
    let mut events = vec![press(Key::L, Modifiers::SHIFT)];
    assert!(process_keys(&mut f, &mut events));
    assert!(events.is_empty());
    let mut plain = vec![press(Key::L, Modifiers::NONE)];
    assert!(!process_keys(&mut f, &mut plain));
}

#[test]
fn extension_tools_toggle_and_check_geometry_fails() {
    let mut f = EditorFacade::default();
    f.press_extension_tool(ToolId::Flange);
    assert_eq!(f.active_tool, Some(ToolId::Flange));
    f.press_extension_tool(ToolId::Flange);
    assert_eq!(f.active_tool, None);
    f.press_extension_tool(ToolId::CheckGeometry);
    assert!(f.topology_failed);
    f.dismiss_failure();
    assert!(!f.topology_failed);
}
