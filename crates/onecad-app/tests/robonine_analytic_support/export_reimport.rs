//! Same-worker STEP reimport; independent source reader validation is separate.
use super::*;
use onecad_app::dto::{DocumentProjection, FeatureStatus};
use onecad_protocol::mesh::{MeshLimits, MeshView};
use std::sync::Mutex;

#[derive(Default)]
struct Events(Mutex<Vec<AppEvent>>);
impl EventSink for Events {
    fn emit(&self, event: AppEvent) -> Result<(), String> {
        self.0.lock().unwrap().push(event);
        Ok(())
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
                    .all(|f| f.status == FeatureStatus::Ok));
                return projection;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("STEP reimport worker regeneration timed out")
}

fn bounds(bytes: &[u8]) -> [[f64; 3]; 2] {
    let mesh = MeshView::decode(bytes, MeshLimits::default()).unwrap();
    let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for i in 0..mesh.header().triangle_count as usize {
        for vertex in mesh.triangle(i).unwrap() {
            let p = mesh.position(vertex as usize).unwrap();
            for axis in 0..3 {
                bounds[0][axis] = bounds[0][axis].min(p[axis] as f64);
                bounds[1][axis] = bounds[1][axis].max(p[axis] as f64);
            }
        }
    }
    bounds
}

fn matches(actual: &[[f64; 3]; 2], expected: &[[f64; 3]; 2]) -> bool {
    (0..2).all(|side| (0..3).all(|axis| (actual[side][axis] - expected[side][axis]).abs() <= 0.15))
}

pub(super) async fn verify(path: &Path, area: &Path) {
    // Bounds measured from exact source BReps after the authored poses; the
    // renderer's pre-existing tessellation envelope is 0.15 mm.
    let expected = [
        [[-64.0, -24.0, -8.0], [64.0, 35.5, 16.0]],
        [[41.0, -23.0, -1.5], [57.0, 23.0, 78.5]],
        [[-4.0, -13.35, -1.0], [57.0, -4.0, 3.5]],
        [[-10.0, -10.0, -3.8], [10.0, 10.0, 4.2011176]],
        [[-56.0, -23.0, -1.5], [-40.0, 23.0, 78.5]],
        [[-56.0, 4.0, -1.0], [5.0, 13.35, 3.5]],
    ];
    let events = Arc::new(Events::default());
    let host = HostContext {
        paths: HostPaths {
            app_data: Some(area.join("reimport-data")),
            app_config: Some(area.join("reimport-config")),
        },
        events: events.clone(),
        dialogs: Arc::new(Quiet),
    };
    let state = AppState::default();
    state.start(host.clone()).unwrap();
    api::import_step(&state, host.clone(), path.to_string_lossy().into_owned())
        .await
        .unwrap();
    let projection = settled(&state, &events).await;
    assert_eq!(projection.bodies.len(), 6);
    assert_eq!(projection.features.len(), 1);
    let mut seen = [false; 6];
    for body in projection.bodies.keys() {
        let bytes = api::get_mesh(&state, body.clone(), "fine".into(), None, None)
            .await
            .unwrap();
        let actual = bounds(&bytes);
        let candidates: Vec<_> = expected
            .iter()
            .enumerate()
            .filter(|(i, e)| !seen[*i] && matches(&actual, e))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(candidates.len(), 1, "STEP reimport body bounds {actual:?}");
        seen[candidates[0]] = true;
    }
    assert!(seen.into_iter().all(|value| value));
    api::shutdown_host(&state, host, None).await.unwrap();
}
