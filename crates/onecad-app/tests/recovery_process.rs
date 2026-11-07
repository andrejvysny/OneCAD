//! Controlled process exits exercise persisted saves and repeated crash recovery.
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use onecad_app::api;
use onecad_app::autosave;
use onecad_app::host::{AppEvent, DialogPort, EventSink, FileDialog, HostContext, HostPaths};
use onecad_app::state::AppState;
use onecad_app::worker::PendingBackend;
use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord,
};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::ids::RecordId;
use onecad_core::io::recovery::{marker_path, scan_recoverable, SessionMarker};
use uuid::Uuid;

struct Quiet;
impl EventSink for Quiet {
    fn emit(&self, _: AppEvent) -> Result<(), String> {
        Ok(())
    }
}
#[async_trait::async_trait]
impl DialogPort for Quiet {
    async fn pick(&self, _: FileDialog) -> Vec<String> {
        Vec::new()
    }
}

fn setup(root: &Path) -> (AppState, HostContext) {
    let state = AppState::new(Arc::new(|| {
        let b = Arc::new(PendingBackend);
        (
            b.clone(),
            b.clone(),
            b.clone(),
            b.clone(),
            b.clone(),
            b.clone(),
            b.clone(),
            b.clone(),
            b.clone(),
            b,
        )
    }));
    let host = HostContext {
        paths: HostPaths {
            app_data: Some(root.into()),
            app_config: Some(root.join("config")),
        },
        events: Arc::new(Quiet),
        dialogs: Arc::new(Quiet),
    };
    (state, host)
}

async fn add(state: &AppState, seed: u128, distance: f64) {
    let mut guard = state.runtime.lock().await;
    let rt = guard.as_mut().unwrap();
    rt.apply(EditCommand::AddOperation {
        record: OperationRecord::new(
            RecordId(Uuid::from_u128(seed)),
            0,
            "Extrude",
            Operation::Known(KnownOperation::Extrude(ExtrudeParams {
                profile: None,
                distance: Scalar::new(distance),
                draft_angle_deg: Scalar::new(0.0),
                mode: ExtrudeMode::Blind,
                boolean_mode: BooleanMode::NewBody,
                target_body: None,
                target_face: None,
                two_directions: false,
                mode2: ExtrudeMode::Blind,
                distance2: Scalar::new(0.0),
                target_face2: None,
                extra: Default::default(),
            })),
        ),
        at_cursor: true,
    })
    .unwrap();
    assert!(rt.is_dirty());
}

async fn assert_content(state: &AppState, expected: &[&str], dirty: bool, path: &Path) {
    let guard = state.runtime.lock().await;
    let rt = guard.as_ref().unwrap();
    let projection = rt.projection();
    let values: Vec<_> = projection
        .features
        .iter()
        .map(|f| f.value_text.as_str())
        .collect();
    assert_eq!(values, expected);
    assert_eq!(rt.is_dirty(), dirty);
    assert_eq!(rt.path(), Some(path));
    assert_eq!(rt.title(), "Bracket");
}

async fn child(root: &Path, phase: &str) {
    let (state, host) = setup(root);
    let saved = root.join("Bracket.onecad");
    match phase {
        "save" => {
            api::new_document(&state, host.clone()).await.unwrap();
            add(&state, 1, 10.0).await;
            assert!(
                api::save_document(
                    &state,
                    host,
                    Some(saved.to_string_lossy().into_owned()),
                    None
                )
                .await
                .unwrap()
                .clean
            );
            assert_content(&state, &["10.0 mm"], false, &saved).await;
        }
        "reopen" => {
            assert!(api::check_recovery(&state, host.clone())
                .await
                .unwrap()
                .is_empty());
            api::open_document(&state, host, saved.to_string_lossy().into_owned(), None)
                .await
                .unwrap();
            assert_content(&state, &["10.0 mm"], false, &saved).await;
            add(&state, 2, 42.0).await;
            assert!(
                autosave::autosave_current(&state.runtime, root, &state.persistence)
                    .await
                    .is_some()
            );
            assert_content(&state, &["10.0 mm", "42.0 mm"], true, &saved).await;
        }
        "restore" | "restore_again" => {
            let offers = api::check_recovery(&state, host.clone()).await.unwrap();
            assert_eq!(offers.len(), 1);
            assert_eq!(offers[0].title.as_deref(), Some("Bracket"));
            assert_eq!(
                offers[0].original_path.as_deref(),
                Some(saved.to_str().unwrap())
            );
            let id = offers[0].document_id.clone();
            assert!(
                api::recover_document(&state, host.clone(), id.clone(), true)
                    .await
                    .unwrap()
                    .is_some()
            );
            assert_content(&state, &["10.0 mm", "42.0 mm"], true, &saved).await;
            let document_id = state.runtime.lock().await.as_ref().unwrap().document_uuid();
            assert_eq!(document_id.to_string(), id);
            let marker: SessionMarker =
                serde_json::from_slice(&std::fs::read(marker_path(root, document_id)).unwrap())
                    .unwrap();
            assert_eq!(marker.pid, std::process::id());
            assert!(api::check_recovery(&state, host).await.unwrap().is_empty());
        }
        _ => panic!("unknown phase"),
    }
    // Exit before dropping the live app state or its runtime.
    std::process::exit(73);
}

#[test]
fn save_and_repeated_restore_survive_process_exit() {
    if let Ok(phase) = std::env::var("ONECAD_RECOVERY_PHASE") {
        let root = std::env::var_os("ONECAD_RECOVERY_ROOT").unwrap();
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(child(Path::new(&root), &phase));
        unreachable!();
    }
    let root = tempfile::tempdir().unwrap();
    let executable = std::env::current_exe().unwrap();
    let mut original = None;
    for phase in ["save", "reopen", "restore", "restore_again"] {
        let output = Command::new(&executable)
            .args([
                "--exact",
                "save_and_repeated_restore_survive_process_exit",
                "--nocapture",
            ])
            .env("ONECAD_RECOVERY_PHASE", phase)
            .env("ONECAD_RECOVERY_ROOT", root.path())
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(73),
            "phase {phase} failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let offers = scan_recoverable(
            root.path(),
            autosave::pid_alive,
            std::time::SystemTime::now(),
        )
        .unwrap();
        if phase == "save" {
            assert!(offers.is_empty());
            continue;
        }
        assert_eq!(offers.len(), 1);
        let offer = &offers[0];
        let bytes = std::fs::read(&offer.autosave_path).unwrap();
        if let Some(expected) = &original {
            assert_eq!(&bytes, expected, "restore must preserve recovery container");
        } else {
            original = Some(bytes);
        }
        let marker = offer.marker.as_ref().expect("session marker survives exit");
        assert_ne!(marker.pid, 0);
    }
}
