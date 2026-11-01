use super::*;
use crate::tool_registry::{shortcut_label, shortcut_tool};
use egui::Key;

const M: KeyContext = KeyContext::Modeling;
const S: KeyContext = KeyContext::Sketch;

#[test]
fn assign_remaps_and_unbinds_the_conflicting_tool() {
    let mut o = Vec::new();
    assign(&mut o, M, ToolId::Extrude, Key::Q);
    assert_eq!(effective_key(&o, ToolId::Extrude, M), Some(Key::Q));
    assert_eq!(shortcut_tool(&o, Key::Q, false), Some(ToolId::Extrude));
    assert_eq!(shortcut_tool(&o, Key::E, false), None);
    assert_eq!(shortcut_label(&o, ToolId::Extrude, false), Some("Q"));
    // Taking Fillet's F unbinds Fillet.
    assert_eq!(
        conflict(&o, M, ToolId::Revolve, Key::F),
        Some(ToolId::Fillet)
    );
    assign(&mut o, M, ToolId::Revolve, Key::F);
    assert_eq!(effective_key(&o, ToolId::Fillet, M), None);
    assert_eq!(shortcut_label(&o, ToolId::Fillet, false), None);
    assert_eq!(shortcut_tool(&o, Key::F, false), Some(ToolId::Revolve));
    assert_eq!(validate(&o), Ok(()));
}

#[test]
fn reset_restores_default_and_unbinds_a_tool_that_took_it() {
    let mut o = Vec::new();
    assign(&mut o, M, ToolId::Extrude, Key::F); // Fillet loses F
    reset(&mut o, M, ToolId::Fillet);
    assert_eq!(effective_key(&o, ToolId::Fillet, M), Some(Key::F));
    assert_eq!(effective_key(&o, ToolId::Extrude, M), None);
    assert_eq!(validate(&o), Ok(()));
    reset(&mut o, M, ToolId::Extrude);
    assert!(o.is_empty());
    assign(&mut o, M, ToolId::Extrude, Key::Q);
    assign(&mut o, S, ToolId::Line, Key::Z);
    reset_all(&mut o);
    assert!(o.is_empty());
}

#[test]
fn entries_equal_to_default_are_removed() {
    let mut o = Vec::new();
    assign(&mut o, M, ToolId::Extrude, Key::Q);
    assert_eq!(changed_count(&o), 1);
    assign(&mut o, M, ToolId::Extrude, Key::E);
    assert!(o.is_empty());
    assert_eq!(changed_count(&o), 0);
    assign(&mut o, M, ToolId::Extrude, Key::F);
    assert_eq!(o.len(), 2);
    assert_eq!(changed_count(&o), 2);
}

#[test]
fn modeling_and_sketch_are_independent() {
    let mut o = Vec::new();
    assign(&mut o, S, ToolId::Line, Key::R); // Sketch Rectangle loses R
    assert_eq!(effective_key(&o, ToolId::Rectangle, S), None);
    assert_eq!(effective_key(&o, ToolId::Revolve, M), Some(Key::R));
    assert_eq!(shortcut_tool(&o, Key::R, false), Some(ToolId::Revolve));
    assert_eq!(shortcut_tool(&o, Key::R, true), Some(ToolId::Line));
    assert_eq!(validate(&o), Ok(()));
}

#[test]
fn assign_ignores_unrecordable_keys_and_tools_without_defaults() {
    let mut o = Vec::new();
    assign(&mut o, M, ToolId::Extrude, Key::Escape);
    assign(&mut o, M, ToolId::Select, Key::Q);
    assign(&mut o, M, ToolId::Line, Key::Q);
    assert!(o.is_empty());
    assert!(recordable(Key::A) && recordable(Key::Z) && recordable(Key::Num0));
    assert!(recordable(Key::Num9));
    assert!(!recordable(Key::Escape) && !recordable(Key::F1) && !recordable(Key::Minus));
}

fn entry(context: KeyContext, tool: ToolId, key: Option<&str>) -> ShortcutOverride {
    ShortcutOverride {
        context,
        tool,
        key: key.map(str::to_owned),
    }
}

#[test]
fn validate_rejects_bad_overrides() {
    assert_eq!(validate(&[entry(M, ToolId::Extrude, Some("Q"))]), Ok(()));
    assert_eq!(validate(&[entry(M, ToolId::Extrude, None)]), Ok(()));
    let bad = [
        vec![
            entry(M, ToolId::Extrude, Some("Q")),
            entry(M, ToolId::Extrude, Some("W")),
        ],
        vec![entry(M, ToolId::Select, Some("Q"))],
        vec![entry(M, ToolId::Line, Some("Q"))],
        vec![entry(M, ToolId::Extrude, Some("Escape"))],
        vec![entry(M, ToolId::Extrude, Some("Nonsense"))],
        vec![entry(M, ToolId::Extrude, Some("F1"))],
        // Fillet still owns F by default.
        vec![entry(M, ToolId::Extrude, Some("F"))],
        vec![
            entry(M, ToolId::Extrude, Some("Q")),
            entry(M, ToolId::Fillet, Some("Q")),
        ],
        (0..65)
            .map(|_| entry(M, ToolId::Extrude, Some("Q")))
            .collect(),
    ];
    for overrides in bad {
        assert!(validate(&overrides).is_err(), "{overrides:?}");
    }
}

#[test]
fn serde_roundtrip_is_camel_case_and_strict() {
    let mut o = Vec::new();
    assign(&mut o, S, ToolId::Line, Key::R);
    let json = serde_json::to_string(&o).unwrap();
    assert!(
        json.contains("\"sketch\"") && json.contains("\"line\""),
        "{json}"
    );
    assert_eq!(
        serde_json::from_str::<Vec<ShortcutOverride>>(&json).unwrap(),
        o
    );
    assert!(
        serde_json::from_str::<ShortcutOverride>(
            r#"{"context":"sketch","tool":"line","key":"R","x":1}"#
        )
        .is_err()
    );
}

#[test]
fn key_names_round_trip_for_every_recordable_key() {
    for key in Key::ALL.iter().copied().filter(|k| recordable(*k)) {
        assert_eq!(Key::from_name(key.name()), Some(key), "{key:?}");
    }
}
