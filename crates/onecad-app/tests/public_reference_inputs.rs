//! Public source-authored oracle checks; imported geometry remains an ImportStep history.
use async_trait::async_trait;
use onecad_app::{
    api,
    dto::{DocumentProjection, GeometryReadFenceDto, MassPropertiesDto},
    host::{AppEvent, DialogPort, EventSink, FileDialog, HostContext, HostPaths},
    state::AppState,
};
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Default)]
struct Events(Mutex<Vec<AppEvent>>);
impl EventSink for Events {
    fn emit(&self, event: AppEvent) -> Result<(), String> {
        self.0.lock().unwrap().push(event);
        Ok(())
    }
}
struct Dialogs;
#[async_trait]
impl DialogPort for Dialogs {
    async fn pick(&self, _: FileDialog) -> Vec<String> {
        Vec::new()
    }
}

async fn settled(state: &AppState, events: &Events) -> DocumentProjection {
    let source = api::get_projection(state).await.unwrap();
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let terminal = events
                .0
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find_map(|event| match event {
                    AppEvent::RegenFinished(report)
                        if report.runtime_session == source.runtime_session
                            && report.source_revision >= source.revision
                            && report.outcome != "superseded" =>
                    {
                        Some(report.clone())
                    }
                    _ => None,
                });
            if let Some(report) = terminal {
                assert!(
                    matches!(report.outcome.as_str(), "published" | "noop"),
                    "{report:?}"
                );
                assert!(report.failed_steps.is_empty(), "{report:?}");
                let projection = api::get_projection(state).await.unwrap();
                assert!(projection
                    .features
                    .iter()
                    .all(|feature| feature.status == onecad_app::dto::FeatureStatus::Ok));
                return projection;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("production worker regeneration timed out")
}

async fn mass(state: &AppState, body: &str) -> MassPropertiesDto {
    let context = api::preview_context(state).await.unwrap();
    let body =
        onecad_app::worker::wire::body_id_wire(body.parse::<onecad_core::ids::BodyId>().unwrap());
    api::query_mass_properties(
        state,
        body,
        Some(GeometryReadFenceDto {
            document_id: context.document_id,
            runtime_session: context.runtime_session,
            snapshot_id: context.snapshot_id.0,
        }),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn nist_ftc09_import_matches_frozen_upstream_properties_and_reopens_offline() {
    assert!(
        onecad_app::worker::resolve_worker_path().is_some(),
        "production worker required"
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let oracle: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../fixtures/reference-oracles/nist-ftc09.json"
    ))
    .unwrap();
    let source =
        std::fs::read(root.join("fixtures/public-cad/nist/nist_ftc_09_asme1_rd.stp")).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&source)),
        oracle["sourceSha256"].as_str().unwrap()
    );
    let area = tempfile::tempdir().unwrap();
    let input = area.path().join("source.step");
    std::fs::write(&input, source).unwrap();
    let events = Arc::new(Events::default());
    let host = HostContext {
        paths: HostPaths {
            app_data: Some(area.path().join("data")),
            app_config: Some(area.path().join("config")),
        },
        events: events.clone(),
        dialogs: Arc::new(Dialogs),
    };
    let state = AppState::default();
    state.start(host.clone()).unwrap();
    api::import_step(&state, host.clone(), input.to_string_lossy().into_owned())
        .await
        .unwrap();
    let projection = settled(&state, &events).await;
    assert_eq!(
        projection.bodies.len(),
        1,
        "compare the sole solid, not supplementary curves"
    );
    assert_eq!(projection.features.len(), 1);
    let body = projection.bodies.keys().next().unwrap().clone();
    let properties = mass(&state, &body).await;
    let tolerance = oracle["trialRelativeTolerance"].as_f64().unwrap();
    for (actual, expected) in [
        (properties.volume, oracle["volumeMm3"].as_f64().unwrap()),
        (
            properties.surface_area,
            oracle["surfaceAreaMm2"].as_f64().unwrap(),
        ),
    ] {
        eprintln!("FTC09 upstream oracle actual={actual:.12} expected={expected:.12} relTolerance={tolerance}");
        assert!((actual - expected).abs() <= expected.abs() * tolerance);
    }
    let saved = area.path().join("imported.onecad");
    assert!(
        api::save_document(
            &state,
            host.clone(),
            Some(saved.to_string_lossy().into_owned()),
            None
        )
        .await
        .unwrap()
        .clean
    );
    api::close_document(&state, host.clone()).await.unwrap();
    std::fs::remove_file(input).unwrap();
    api::open_document(
        &state,
        host.clone(),
        saved.to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    let reopened = settled(&state, &events).await;
    assert!(reopened.bodies.contains_key(&body));
    let fresh = mass(&state, &body).await;
    assert_eq!(fresh.volume, properties.volume);
    assert_eq!(fresh.surface_area, properties.surface_area);
    api::shutdown_host(&state, host, None).await.unwrap();
}
