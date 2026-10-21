//! Native Finish/Discard routes, refusal repair and model history over production worker and Metal.
//! Synthetic host events; physical input and a separate OS process reopen remain unqualified.
use super::point_center_delivery_tests::support::*;
use super::sketch_selection_delivery_tests::{line, open, selection, wait_closed};
use super::*;
use onecad_core::ids::{EntityId, SketchId};
use onecad_interaction::sketch::{SketchStatus, SketchTarget};

#[path = "sketch_finish_delivery_tests/reopen.rs"]
mod reopen;

const OVERLAP: &str = "SKETCH_PROFILE_OVERLAPPING_CURVES";

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn refused_finish_reenters_for_chooser_repair_and_model_history_restores_region() {
    let area = tempfile::tempdir().unwrap();
    let mut host = Host::new(area.path()).await;
    rectangle(&mut host).await;
    let overlap = line(&mut host, [20.0, 0.0], [60.0, 0.0]).await;
    host.control("Select");
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        overlap_reported(app)
    })
    .await;
    let sketch = host.sketch().id;
    let created = depths(&host);
    host.control("Finish sketch");
    wait_closed(&mut host).await;
    assert!(overlap_reported(&host.app), "Finish reports the refusal");
    assert!(
        host.app
            .doc
            .status
            .starts_with("Sketch finished without a profile:"),
        "{}",
        host.app.doc.status
    );
    assert!(closed_refusal(&host.app, sketch), "closed sketch keeps it");
    assert!(regions(&host).is_empty());
    assert_eq!(sketches(&host), 1);
    let refused = depths(&host);
    assert_eq!(refused, created, "a refused Finish adds no entry");

    host.app.edit_sketch(sketch);
    open(&mut host).await;
    assert!(host.sketch().contains_entity(overlap));
    host.click(host.project(40.0, 0.0));
    assert!(selection(&host).is_empty(), "collinear pick must ask");
    host.control(&format!("Line · {overlap}"));
    assert_eq!(selection(&host), vec![SketchTarget::Entity(overlap)]);
    host.ui(vec![key(egui::Key::Delete, egui::Modifiers::NONE)]);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        active(app)
            && !app
                .doc
                .sketch
                .session
                .as_ref()
                .unwrap()
                .authoritative
                .contains_entity(overlap)
    })
    .await;
    host.control("Finish sketch");
    wait_closed(&mut host).await;
    let SketchStatus::Finished { result, .. } =
        &host.app.doc.sketch.session.as_ref().unwrap().status
    else {
        panic!("Finish result: {}", host.app.doc.status)
    };
    assert_eq!(result.regions.len(), 1);
    let repaired = regions(&host);
    assert_eq!(repaired.len(), 1);
    assert!(!overlap_reported(&host.app), "repair clears the refusal");
    assert!(!closed_refusal(&host.app, sketch));
    assert!(!stale_warning(&host.app), "no warning after a clean Finish");
    let finished = depths(&host);
    let projection = &host.app.doc.display.as_ref().unwrap().projection;
    assert_eq!(projection.undo_label.as_deref(), Some("Edit Sketch"));
    assert_eq!(
        sketch_records(&host),
        1,
        "visits never duplicate the record"
    );
    assert_eq!(
        finished,
        (refused.0 + 1, 0),
        "one entry for the repair visit"
    );

    model_history(&mut host, false).await;
    assert_eq!(depths(&host), (refused.0, 1));
    assert!(
        regions(&host).is_empty(),
        "Undo restores the refused sketch"
    );
    assert!(closed_refusal(&host.app, sketch), "and its refusal");
    model_history(&mut host, true).await;
    assert_eq!(depths(&host), finished);
    assert_eq!(regions(&host), repaired, "Redo restores the same region");
    assert!(!closed_refusal(&host.app, sketch));
    assert!(!stale_warning(&host.app), "Redo leaves no stale warning");
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn no_op_finish_discards_and_partial_undo_keep_model_history_and_region() {
    let area = tempfile::tempdir().unwrap();
    let mut host = Host::new(area.path()).await;
    rectangle(&mut host).await;
    let sketch = host.sketch().id;
    let entities = ids(&host);
    host.control("Finish sketch");
    wait_closed(&mut host).await;
    let baseline = (depths(&host), regions(&host));
    assert_eq!(baseline.1.len(), 1);

    reenter(&mut host, sketch).await;
    host.control("Finish sketch");
    wait_closed(&mut host).await;
    assert_eq!((depths(&host), regions(&host)), baseline, "no-op Finish");

    reenter(&mut host, sketch).await;
    line(&mut host, [10.0, 60.0], [30.0, 60.0]).await;
    line(&mut host, [10.0, 70.0], [30.0, 70.0]).await;
    host.control("Select");
    host.history(false, entities.len() + 3).await;
    host.control("Discard sketch edits");
    wait_closed(&mut host).await;
    assert_eq!(
        (depths(&host), regions(&host)),
        baseline,
        "partial Undo then Discard"
    );

    reenter(&mut host, sketch).await;
    line(&mut host, [10.0, 60.0], [30.0, 60.0]).await;
    host.control("Select");
    host.history(false, entities.len()).await;
    host.control("Finish sketch");
    wait_closed(&mut host).await;
    assert_eq!((depths(&host), regions(&host)), baseline, "net-empty visit");

    reenter(&mut host, sketch).await;
    line(&mut host, [10.0, 60.0], [30.0, 60.0]).await;
    let kept = ids(&host);
    line(&mut host, [10.0, 70.0], [30.0, 70.0]).await;
    host.control("Select");
    host.history(false, kept.len()).await;
    host.control("Finish sketch");
    wait_closed(&mut host).await;
    let partial = depths(&host);
    assert_eq!(partial, (baseline.0.0 + 1, 0), "partial Undo then Finish");
    assert_eq!(regions(&host), baseline.1, "open line keeps the region");

    host.app.new_sketch();
    open(&mut host).await;
    assert_eq!(sketches(&host), 2);
    host.control("Discard sketch edits");
    wait_closed(&mut host).await;
    assert_eq!(sketches(&host), 1, "Discard of a created visit");
    assert_eq!(depths(&host), partial);

    assert_eq!(sketch_records(&host), 1);
    reenter(&mut host, sketch).await;
    assert_eq!(ids(&host), kept, "identities survive every visit");
    host.stop().await;
}

async fn rectangle(host: &mut Host) {
    host.control("Rectangle");
    host.xy(0.0, 0.0);
    host.xy(80.0, 50.0);
    host.count(8).await;
}

async fn reenter(host: &mut Host, sketch: SketchId) {
    host.app.edit_sketch(sketch);
    open(host).await;
    assert_eq!(host.sketch().id, sketch);
}

async fn model_history(host: &mut Host, redo: bool) {
    let before = depths(host);
    let command = egui::Modifiers {
        command: true,
        mac_cmd: true,
        shift: redo,
        ..Default::default()
    };
    host.ui(vec![
        egui::Event::ModifiersChanged(command),
        key(egui::Key::Z, command),
    ]);
    host.ui(vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        !app.busy() && app_depths(app) != before
    })
    .await;
    host.redraw();
}

fn overlap_reported(app: &App) -> bool {
    app.doc
        .diagnostics
        .profile
        .as_ref()
        .is_some_and(|report| report.has_reason(OVERLAP))
}

fn closed_refusal(app: &App, sketch: SketchId) -> bool {
    use crate::diagnostics::{Source, State};
    app.doc
        .display
        .as_ref()
        .unwrap()
        .profile_failures
        .iter()
        .find(|failure| failure.sketch == sketch)
        .is_some_and(|failure| {
            failure
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.reason_code.as_deref() == Some(OVERLAP))
                && app
                    .doc
                    .diagnostics
                    .contains(Source::Sketch, State::Warning, "")
        })
}

fn stale_warning(app: &App) -> bool {
    use crate::diagnostics::{Source, State};
    app.doc
        .diagnostics
        .contains(Source::Sketch, State::Warning, "")
}

fn ids(host: &Host) -> Vec<EntityId> {
    host.sketch().entities().iter().map(|e| e.id()).collect()
}

fn depths(host: &Host) -> (usize, usize) {
    app_depths(&host.app)
}

fn app_depths(app: &App) -> (usize, usize) {
    let projection = &app.doc.display.as_ref().unwrap().projection;
    (projection.undo_depth, projection.redo_depth)
}

fn sketches(host: &Host) -> usize {
    host.app
        .doc
        .display
        .as_ref()
        .unwrap()
        .projection
        .sketches
        .len()
}

fn regions(host: &Host) -> Vec<String> {
    host.app
        .doc
        .display
        .as_ref()
        .unwrap()
        .profiles
        .iter()
        .map(|profile| profile.reference.region.to_string())
        .collect()
}

fn sketch_records(host: &Host) -> usize {
    use onecad_core::document::record::{KnownOperation, Operation};
    let display = host.app.doc.display.as_ref().unwrap();
    display
        .records
        .iter()
        .filter(|record| matches!(record.op, Operation::Known(KnownOperation::Sketch(_))))
        .count()
}
