use super::{Action, Request, definition_edit, run};
use crate::bridge::Backend;
use onecad_app::api;
use onecad_assembly::occurrence::{ComponentDefId, OccurrenceId};
use onecad_core::edit::AssemblyEdit;

async fn edit(backend: &mut Backend, session: &str, edit: AssemblyEdit) {
    let generation = api::get_assembly_projection(&backend.state, session)
        .await
        .unwrap()
        .persisted_generation;
    let reply = run(
        backend,
        Request {
            sequence: 1,
            session: session.into(),
            generation,
            action: Action::Edit(edit),
            cancellation: None,
        },
    )
    .await
    .unwrap();
    assert!(
        reply.capture_error.is_none(),
        "accepted edit failed scene capture: {:?}",
        reply.capture_error
    );
}

async fn positions(backend: &mut Backend) -> Vec<f64> {
    let display = crate::capture::load_display(backend).await.unwrap();
    display
        .scene
        .bodies()
        .iter()
        .filter(|body| body.occurrence_path().is_some())
        .map(|body| body.bounds().min.x)
        .collect()
}

async fn history(backend: &mut Backend, redo: bool) {
    if redo {
        api::redo(&backend.state, backend.host.clone())
            .await
            .unwrap();
    } else {
        api::undo(&backend.state, backend.host.clone())
            .await
            .unwrap();
    }
    crate::services::load_ready_display(backend).await.unwrap();
}

pub(crate) async fn qualify(backend: &mut Backend, session: &str, definition: ComponentDefId) {
    let projection = api::get_assembly_projection(&backend.state, session)
        .await
        .unwrap();
    let source = projection
        .definitions
        .iter()
        .find(|item| item.id == definition.to_string())
        .unwrap();
    let mut unready = source.clone();
    unready.mesh_generation = None;
    assert!(definition_edit::translate(&unready, [1.0, 0.0, 0.0]).is_err());
    let baseline = positions(backend).await;
    assert_eq!(baseline.len(), 2);
    assert!(definition_edit::translate(source, [f64::NAN, 0.0, 0.0]).is_err());
    assert!(definition_edit::translate(source, [0.0; 3]).is_err());
    edit(
        backend,
        session,
        definition_edit::translate(source, [7.0, 0.0, 0.0]).unwrap(),
    )
    .await;
    let moved = positions(backend).await;
    for (before, after) in baseline.iter().zip(&moved) {
        assert!((after - before - 7.0).abs() < 1e-6);
    }
    history(backend, false).await;
    assert_eq!(positions(backend).await, baseline);
    history(backend, true).await;
    assert_eq!(positions(backend).await, moved);
    history(backend, false).await;
    qualify_independent(backend, session, definition, baseline).await;
}

async fn qualify_independent(
    backend: &mut Backend,
    session: &str,
    definition: ComponentDefId,
    baseline: Vec<f64>,
) {
    let independent = ComponentDefId::new();
    let occurrence = OccurrenceId(uuid::Uuid::from_u128(0xC3));
    edit(
        backend,
        session,
        AssemblyEdit::MakeIndependent {
            occurrence,
            new_definition: independent,
            name: "Independent part".into(),
        },
    )
    .await;
    let projection = api::get_assembly_projection(&backend.state, session)
        .await
        .unwrap();
    assert_eq!(
        projection
            .occurrences
            .iter()
            .find(|item| item.id == occurrence.to_string())
            .unwrap()
            .definition,
        independent.to_string()
    );
    let source = projection
        .definitions
        .iter()
        .find(|item| item.id == definition.to_string())
        .unwrap();
    edit(
        backend,
        session,
        definition_edit::translate(source, [11.0, 0.0, 0.0]).unwrap(),
    )
    .await;
    let moved = positions(backend).await;
    assert_eq!(moved.len(), 2);
    assert_eq!(
        moved
            .iter()
            .zip(&baseline)
            .filter(|(after, before)| (*after - *before - 11.0).abs() < 1e-6)
            .count(),
        1
    );
    assert_eq!(
        moved
            .iter()
            .zip(&baseline)
            .filter(|(after, before)| *after == *before)
            .count(),
        1
    );
    history(backend, false).await;
    history(backend, false).await;
    let restored = api::get_assembly_projection(&backend.state, session)
        .await
        .unwrap();
    assert!(
        restored
            .occurrences
            .iter()
            .all(|item| item.definition == definition.to_string())
    );
    assert!(
        !restored
            .definitions
            .iter()
            .any(|item| item.id == independent.to_string())
    );
    assert_eq!(positions(backend).await, baseline);
    history(backend, true).await;
    let redone = api::get_assembly_projection(&backend.state, session)
        .await
        .unwrap();
    assert!(
        redone
            .definitions
            .iter()
            .any(|item| item.id == independent.to_string())
    );
    history(backend, false).await;
}
