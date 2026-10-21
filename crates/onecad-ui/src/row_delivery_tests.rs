//! Real widgets receive native-host synthetic events; physical input stays separate.
use super::point_center_delivery_tests::support::{Host, active};
use super::*;
use crate::ui_state::InspectorTab;
use onecad_interaction::sketch::SketchAction;

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic rows do not qualify physical input"]
async fn unselected_rows_single_select_double_click_and_explicit_edit_share_entry() {
    let root = tempfile::tempdir().unwrap();
    let mut host = Host::new(root.path()).await;
    host.control("Rectangle");
    host.xy(0.0, 0.0);
    host.xy(80.0, 50.0);
    host.count(8).await;
    finish(&mut host).await;
    let sketch = host
        .app
        .display
        .as_ref()
        .unwrap()
        .projection
        .sketches
        .values()
        .next()
        .unwrap()
        .clone();
    for (history, explicit) in [(false, false), (true, false), (false, true)] {
        select_and_edit(&mut host, &sketch.name, history, explicit);
        pump_until(&mut host.app, &host.context, &mut host.frame, active).await;
        assert_eq!(host.sketch().id.to_string(), sketch.id);
        assert_eq!(host.sketch().entities().len(), 8);
        finish(&mut host).await;
    }
    host.stop().await;
}

fn select_and_edit(host: &mut Host, name: &str, history: bool, explicit: bool) {
    host.app.ui.selected_sketch = None;
    host.app.selected_feature = None;
    host.app.ui.explorer_open = !history;
    host.app.ui.inspector_open = true;
    host.app.ui.inspector_tab = if history {
        InspectorTab::History
    } else {
        InspectorTab::Properties
    };
    host.redraw();
    // Separate this double-click from setup clicks in egui's simulated clock.
    for _ in 0..24 {
        host.ui(vec![]);
    }
    let position = row_position(host, name, history);
    host.click(position);
    assert!(!host.app.sketch_active(), "single click must select only");
    assert!(
        if history {
            host.app.selected_feature.is_some()
        } else {
            host.app.ui.selected_sketch.is_some()
        },
        "history={history} tab={:?} position={position:?} status={}",
        host.app.ui.inspector_tab,
        host.app.status
    );
    if explicit {
        host.control("Edit sketch");
    } else {
        host.click(position);
        assert!(host.context.input(|input| {
            input
                .pointer
                .button_double_clicked(egui::PointerButton::Primary)
        }));
    }
}

fn row_position(host: &mut Host, name: &str, history: bool) -> egui::Pos2 {
    for _ in 0..40 {
        host.ui(vec![]);
        let chrome = crate::layout::chrome(host.screen(), &host.app.ui);
        let bounds = if history {
            chrome.inspector
        } else {
            chrome.explorer
        };
        if let Some((_, rect)) = host
            .widgets
            .iter()
            .find(|(label, rect)| label == name && bounds.contains(rect.center()))
        {
            return rect.center();
        }
    }
    panic!("missing row {name}: history={history}");
}

async fn finish(host: &mut Host) {
    host.app.sketch_dispatch(SketchAction::Finish);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        !app.sketch_active()
    })
    .await;
    host.redraw();
}
