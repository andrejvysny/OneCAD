use super::*;

pub(super) async fn check(host: &mut Harness) {
    let record = host
        .app
        .doc
        .display
        .as_ref()
        .unwrap()
        .records
        .iter()
        .find(|record| matches!(record.op, Operation::Known(KnownOperation::Extrude(_))))
        .unwrap()
        .record_id;
    crate::pill::select_feature(&mut host.app, record);
    crate::inspector_history::edit_selected_feature(&mut host.app);
    assert_eq!(
        host.app.ui.inspector_tab,
        crate::ui_state::InspectorTab::History
    );
    host.wait(|app| {
        app.doc.interaction.tool.as_ref().is_some_and(|tool| {
            matches!(tool.draft_status, DraftStatus::Valid(_))
                && tool.preview == PreviewStatus::UnavailableForEdit
        })
    })
    .await;
    assert_editor(host);
    host.app
        .dispatch(Action::DraftChanged("missing_variable".into()));
    host.wait(|app| {
        app.doc
            .interaction
            .tool
            .as_ref()
            .is_some_and(|tool| matches!(tool.draft_status, DraftStatus::Invalid(_)))
    })
    .await;
    assert_editor(host);
    assert_eq!(
        host.app.doc.interaction.tool.as_ref().unwrap().draft,
        "missing_variable"
    );
    crate::pill::cancel_tool(&mut host.app);
    assert!(host.app.doc.interaction.tool.is_none());
    assert_mesh_bounds(&host.app, 90.0);
}

fn assert_editor(host: &mut Harness) {
    for size in [egui::vec2(900.0, 560.0), egui::vec2(1280.0, 820.0)] {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        let inspector = crate::layout::chrome(screen, &host.app.ui).inspector;
        for frame in 0..3 {
            let mut output = host.context.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ui| eframe::App::ui(&mut host.app, ui, &mut host.frame),
            );
            output.textures_delta.clear();
            let mut apply = false;
            for shape in output.shapes {
                if let egui::Shape::Text(text) = shape.shape
                    && text.galley.job.text == "Apply ↵"
                {
                    apply = true;
                    let bounds = egui::Rect::from_min_size(text.pos, text.galley.size());
                    assert!(inspector.contains_rect(bounds));
                    assert!(shape.clip_rect.contains_rect(bounds));
                }
            }
            if frame > 0 {
                assert!(apply, "inline Apply must stay visible at {size:?}");
            }
        }
    }
}
