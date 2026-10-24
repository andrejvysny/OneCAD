//! Production dispatch guards; this synthetic host test does not qualify physical input.
use super::*;
use crate::tool_registry::ToolId;
use onecad_core::document::record::{KnownOperation, Operation};
use onecad_interaction::sketch::{SketchPhase, SketchTool};

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic dispatch is not physical input acceptance"]
async fn restored_controls_use_real_context_and_do_not_enable_mock_actions() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context) = create_home_app(area.path()).await;
    let mut frame = eframe::Frame::_new_kittest();
    pump_until(&mut app, &context, &mut frame, |app| {
        app.doc.bridge.shared.recovery.lock().unwrap().checked
    })
    .await;
    crate::pill::activate(&mut app, &context, ToolId::Sketch);
    assert!(!app.doc.sketch_entry.open);
    assert!(app.doc.interaction.tool.is_none());
    app.document_request(Request::New);
    pump_until(&mut app, &context, &mut frame, |app| {
        app.doc.display.is_some()
    })
    .await;
    for tool in crate::tool_registry::PLANNED_MODEL {
        if matches!(tool.id, ToolId::Extrude | ToolId::MoveRotate) {
            continue;
        }
        crate::pill::activate(&mut app, &context, tool.id);
        cancel_supported_model_start(&mut app, tool.id);
        assert!(app.doc.interaction.tool.is_none(), "{:?}", tool.id);
        assert!(!app.doc.sketch_entry.open, "{:?}", tool.id);
        assert!(app.doc.sketch_entry.pending.is_none(), "{:?}", tool.id);
        assert!(!app.ui.editor_facade.library_mode, "{:?}", tool.id);
        assert!(app.ui.editor_facade.active_tool.is_none(), "{:?}", tool.id);
        assert!(!app.ui.editor_facade.topology_failed, "{:?}", tool.id);
    }
    crate::pill::activate(&mut app, &context, ToolId::Section);
    assert!(app.ui.display_menu.is_some());
    crate::pill::activate(&mut app, &context, ToolId::Section);
    assert!(app.ui.display_menu.is_none());
    app.new_sketch_on_plane(onecad_core::sketch::WorldPlane::XY);
    pump_until(&mut app, &context, &mut frame, |app| {
        app.doc
            .sketch
            .session
            .as_ref()
            .is_some_and(|session| session.phase == SketchPhase::Active)
    })
    .await;
    crate::pill::activate(&mut app, &context, ToolId::Line);
    assert_eq!(
        app.doc.sketch.session.as_ref().unwrap().tool,
        SketchTool::Line
    );
    crate::pill::activate(&mut app, &context, ToolId::Constraints);
    assert!(app.ui.constraints_open);
    crate::pill::activate(&mut app, &context, ToolId::Constraints);
    assert!(!app.ui.constraints_open);
    for tool in crate::tool_registry::PLANNED_SKETCH {
        crate::pill::activate(&mut app, &context, tool.id);
        assert_eq!(
            app.doc.sketch.session.as_ref().unwrap().tool,
            SketchTool::Line
        );
    }
    app.stop_discard();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

fn cancel_supported_model_start(app: &mut App, id: ToolId) {
    match id {
        ToolId::Generators => {
            assert!(
                app.doc.interaction.tool.as_ref().is_some_and(|tool| {
                    matches!(tool.record.op, Operation::Known(KnownOperation::Gear(_)))
                }),
                "Generators must start an actual Gear draft"
            );
            crate::pill::cancel_tool(app);
        }
        ToolId::DatumPlane | ToolId::OffsetPlane => {
            assert!(
                app.doc.sketch_entry.open,
                "{id:?} must open the offset-plane chooser"
            );
            assert!(app.doc.sketch_entry.pending.is_none());
            app.doc.sketch_entry.dismiss();
        }
        _ => {}
    }
}
