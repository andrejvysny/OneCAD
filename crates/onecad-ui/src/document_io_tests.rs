//! Native typed file routing with the production worker; no physical dialog acceptance.
use super::*;
use crate::{
    bridge::{BackendEvent, EventMailbox, Shared},
    document_flow::ExportFormat,
};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct Events {
    mailbox: Arc<EventMailbox>,
    publications: AtomicUsize,
}
impl EventSink for Events {
    fn emit(&self, event: AppEvent) -> Result<(), String> {
        match event {
            AppEvent::ProjectionUpdated(projection) => {
                self.publications.fetch_add(1, Ordering::Relaxed);
                self.mailbox
                    .observe_projection(&projection.runtime_session, projection.revision);
            }
            AppEvent::RegenFinished(report) => self.mailbox.emit(BackendEvent::Regen(report))?,
            AppEvent::WorkerStatus(status) if status.state == "failed" => {
                self.mailbox.emit(BackendEvent::WorkerFailed)?
            }
            _ => {}
        }
        Ok(())
    }
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn typed_real_worker_import_export_roundtrip_preserves_document() {
    assert_eq!(std::env::var("ONECAD_REQUIRE_WORKER").as_deref(), Ok("1"));
    assert!(
        Path::new(
            &std::env::var_os("ONECAD_WORKER_PATH").expect("explicit production worker required")
        )
        .is_file()
    );
    let area = tempfile::tempdir().unwrap();
    let (mut backend, events) = backend(area.path()).await;
    let original = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../onecad-app/tests/fixtures/colored_boxes.step");
    let source_bytes = fs::read(&original).unwrap();
    let source = backend.paths.root.join("documents/source.step");
    fs::copy(&original, &source).unwrap();
    choose(&mut backend, Some(&source));
    assert!(matches!(
        intent(&mut backend, Request::Import(None)).await.unwrap(),
        Outcome::Replaced { warning: None }
    ));
    let expected = solid_properties(&backend).await;
    let volumes: Vec<_> = expected.iter().map(|value| value.0).collect();
    assert_eq!(volumes.len(), 2);
    near(volumes[0], 1000.0);
    near(volumes[1], 24000.0);
    cancelled_failed_imports(&mut backend, &events).await;
    let exported = export_all(&mut backend, &events).await;
    choose(&mut backend, Some(&exported));
    let previous = api::get_projection(&backend.state).await.unwrap();
    assert!(matches!(
        intent(&mut backend, Request::Import(None)).await.unwrap(),
        Outcome::Replaced { warning: None }
    ));
    assert_ne!(
        api::get_projection(&backend.state)
            .await
            .unwrap()
            .document_id,
        previous.document_id
    );
    let actual = solid_properties(&backend).await;
    for (expected, actual) in expected.iter().zip(&actual) {
        near(expected.0, actual.0);
        for (expected, actual) in expected.1.into_iter().zip(actual.1) {
            near(expected, actual);
        }
    }
    assert_eq!(expected.len(), actual.len());
    assert_eq!(fs::read(&source).unwrap(), source_bytes);
    assert_eq!(fs::read(original).unwrap(), source_bytes);
    api::close_document(&backend.state, backend.host.clone())
        .await
        .unwrap();
    backend.state.retire_all();
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn insert_step_routes_into_the_active_document_without_replacing_it() {
    assert_eq!(std::env::var("ONECAD_REQUIRE_WORKER").as_deref(), Ok("1"));
    let area = tempfile::tempdir().unwrap();
    let (mut backend, events) = backend(area.path()).await;
    let before = api::get_projection(&backend.state).await.unwrap();
    let count = events.publications.load(Ordering::Relaxed);
    choose(&mut backend, None);
    assert!(matches!(
        intent(&mut backend, Request::InsertStep).await.unwrap(),
        Outcome::Cancelled
    ));
    assert_eq!(api::get_projection(&backend.state).await.unwrap(), before);
    assert_eq!(events.publications.load(Ordering::Relaxed), count);
    let source = backend.paths.root.join("documents/inserted.step");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../onecad-app/tests/fixtures/colored_boxes.step"),
        &source,
    )
    .unwrap();
    choose(&mut backend, Some(&source));
    let outcome = intent(&mut backend, Request::InsertStep).await.unwrap();
    assert!(
        matches!(outcome, Outcome::Inserted { warning: None }),
        "{outcome:?}"
    );
    let after = api::get_projection(&backend.state).await.unwrap();
    assert_eq!(after.runtime_session, before.runtime_session);
    assert_eq!(after.document_id, before.document_id);
    assert_eq!(after.undo_depth, before.undo_depth + 1);
    assert_eq!(after.features.len(), before.features.len() + 1);
    assert_eq!(after.features.last().unwrap().op_type, "ImportStep");
    assert_eq!(after.bodies.len(), 2);
    assert!(after.dirty);
    api::close_document(&backend.state, backend.host.clone())
        .await
        .unwrap();
    backend.state.retire_all();
}

async fn backend(root: &Path) -> (Backend, Arc<Events>) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = instance.request_adapter(&Default::default()).await.unwrap();
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            required_features: onecad_render::required_device_features(adapter.features()),
            ..Default::default()
        })
        .await
        .unwrap();
    let renderer =
        onecad_render::Renderer::new(&device, &queue, [0, 0], Default::default()).unwrap();
    let paths = crate::qa::QaPaths::from_args(
        ["--qa-root".into(), root.join("qa").into_os_string()].into_iter(),
    )
    .unwrap();
    let events = Arc::new(Events::default());
    let host = HostContext {
        paths: HostPaths {
            app_data: Some(paths.data.clone()),
            app_config: Some(paths.config.clone()),
        },
        events: events.clone(),
        dialogs: Arc::new(Host {
            picks: Mutex::new(Vec::new()),
        }),
    };
    let state = Arc::new(AppState::default());
    state.start(host.clone()).unwrap();
    api::new_document(&state, host.clone()).await.unwrap();
    assert!(
        state
            .readiness()
            .wait_ready(std::time::Duration::from_secs(15))
            .await
    );
    let shared = Arc::new(Shared::new(
        egui::Context::default(),
        renderer.preparation_budget(),
    ));
    shared.recovery.lock().unwrap().checked = true;
    (
        Backend {
            state,
            host,
            paths,
            events: events.mailbox.clone(),
            shared,
            startup_opened: true,
            scene_revision: 0,
            previous_scene: None,
            preparation_lease: None,
        },
        events,
    )
}

fn choose(backend: &mut Backend, path: Option<&Path>) {
    backend.host.dialogs = Arc::new(Host {
        picks: Mutex::new(
            path.map(|path| path.to_string_lossy().into_owned())
                .into_iter()
                .collect(),
        ),
    });
}

async fn intent(backend: &mut Backend, request: Request) -> Result<Outcome, String> {
    let result = tokio::time::timeout(std::time::Duration::from_secs(30), run(backend, request))
        .await
        .expect("typed file operation must complete");
    backend.events.finish();
    result
}

async fn cancelled_failed_imports(backend: &mut Backend, events: &Events) {
    let before = api::get_projection(&backend.state).await.unwrap();
    let count = events.publications.load(Ordering::Relaxed);
    choose(backend, None);
    assert!(matches!(
        intent(backend, Request::Import(None)).await.unwrap(),
        Outcome::Cancelled
    ));
    let worker = backend.state.live_worker().unwrap();
    for extension in ["step", "onecad", "obj"] {
        let corrupt = backend
            .paths
            .root
            .join(format!("documents/corrupt.{extension}"));
        fs::write(&corrupt, b"invalid import data").unwrap();
        choose(backend, Some(&corrupt));
        assert!(intent(backend, Request::Import(None)).await.is_err());
        assert!(!worker.is_retired());
        assert!(backend.state.live_worker().unwrap().is_same(&worker));
    }
    // A system-provided file (iPadOS "Open in") is imported without a dialog.
    let corrupt = backend.paths.root.join("documents/provided.stp");
    fs::write(&corrupt, b"ISO-10303-21;\nnot a step body\n").unwrap();
    choose(backend, None);
    assert!(
        intent(backend, Request::Import(Some(corrupt)))
            .await
            .is_err()
    );
    assert_eq!(api::get_projection(&backend.state).await.unwrap(), before);
    assert_eq!(
        events.publications.load(Ordering::Relaxed),
        count,
        "failed/cancelled import must not publish a replacement"
    );
    assert!(
        backend
            .state
            .readiness()
            .wait_ready(std::time::Duration::from_secs(15))
            .await
    );
}

async fn export_all(backend: &mut Backend, events: &Events) -> PathBuf {
    let before = api::get_projection(&backend.state).await.unwrap();
    let count = events.publications.load(Ordering::Relaxed);
    for (format, extension) in [
        (ExportFormat::Step, "step"),
        (ExportFormat::Stl, "stl"),
        (ExportFormat::Obj, "obj"),
        (ExportFormat::ThreeMf, "3mf"),
    ] {
        let output = backend
            .paths
            .root
            .join(format!("documents/exported.{extension}"));
        choose(backend, Some(&output));
        assert!(
            matches!(intent(backend, Request::Export(format)).await.unwrap(), Outcome::Exported(path) if path == output)
        );
        match format {
            ExportFormat::Step => {
                let text = fs::read_to_string(&output).unwrap();
                assert!(text.starts_with("ISO-10303-21;"));
                assert!(text.contains("MANIFOLD_SOLID_BREP"));
                assert!(text.trim_end().ends_with("END-ISO-10303-21;"));
            }
            ExportFormat::Stl => assert_stl(&output),
            ExportFormat::Obj => assert_obj(&output),
            ExportFormat::ThreeMf => assert_three_mf(&output),
        }
        assert_eq!(
            api::get_projection(&backend.state).await.unwrap(),
            before,
            "export must preserve document/history/dirty state"
        );
        assert_eq!(events.publications.load(Ordering::Relaxed), count);
    }
    backend.paths.root.join("documents/exported.step")
}

type Solid = (f64, [f64; 3]);
async fn solid_properties(backend: &Backend) -> Vec<Solid> {
    let projection = api::get_projection(&backend.state).await.unwrap();
    assert_eq!(projection.features.len(), 1);
    assert_eq!(projection.features[0].op_type, "ImportStep");
    assert_eq!(projection.applied_ops, 1);
    let mut values = Vec::new();
    for body in projection.bodies.keys() {
        let wire = onecad_app::worker::wire::body_id_wire(body.parse().unwrap());
        let topology = api::query_body_topology(&backend.state, wire.clone())
            .await
            .unwrap();
        assert_eq!((topology.solid_count, topology.face_count), (1, 6));
        let mass = api::query_mass_properties(&backend.state, wire, None)
            .await
            .unwrap();
        let blob = api::get_mesh(
            &backend.state,
            body.clone(),
            "fine".into(),
            None,
            Some(projection.runtime_session.clone()),
        )
        .await
        .unwrap();
        let mesh = onecad_protocol::mesh::MeshView::decode(&blob, Default::default()).unwrap();
        assert_eq!(mesh.header().face_count, 6);
        assert!(mesh.header().triangle_count >= 12);
        values.push((mass.volume, mass.centroid));
    }
    values.sort_by(|first, second| first.0.total_cmp(&second.0));
    values
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
}

fn assert_stl(path: &Path) {
    let bytes = fs::read(path).unwrap();
    assert!(bytes.len() >= 84);
    let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
    assert!(count >= 24);
    assert_eq!(bytes.len(), 84 + count * 50);
    for triangle in bytes[84..].as_chunks::<50>().0 {
        let points: Vec<_> = triangle[12..48]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|bytes| f32::from_le_bytes(*bytes))
            .collect();
        assert!(points.iter().all(|value| value.is_finite()));
        assert_ne!(&points[0..3], &points[3..6]);
        assert_ne!(&points[3..6], &points[6..9]);
    }
}

fn assert_obj(path: &Path) {
    let text = fs::read_to_string(path).unwrap();
    let vertices: Vec<_> = text
        .lines()
        .filter_map(|line| line.strip_prefix("v "))
        .collect();
    assert!(vertices.len() >= 16);
    for vertex in &vertices {
        let coordinates: Vec<f64> = vertex
            .split_whitespace()
            .map(|value| value.parse().unwrap())
            .collect();
        assert_eq!(coordinates.len(), 3);
        assert!(coordinates.iter().all(|value| value.is_finite()));
    }
    let faces: Vec<_> = text
        .lines()
        .filter_map(|line| line.strip_prefix("f "))
        .collect();
    assert!(faces.len() >= 24);
    for face in faces {
        let indices: Vec<usize> = face
            .split_whitespace()
            .map(|part| part.split('/').next().unwrap().parse().unwrap())
            .collect();
        assert!(indices.len() >= 3);
        assert!(
            indices
                .iter()
                .all(|index| *index > 0 && *index <= vertices.len())
        );
    }
}

fn assert_three_mf(path: &Path) {
    assert!(fs::read(path).unwrap().starts_with(b"PK\x03\x04"));
    let check = std::process::Command::new("/usr/bin/unzip")
        .arg("-t")
        .arg(path)
        .output()
        .expect("macOS archive verifier required");
    assert!(check.status.success(), "{:?}", check.stderr);
    assert!(
        archive_text(path, "[Content_Types].xml")
            .contains("application/vnd.ms-package.3dmanufacturing-3dmodel+xml")
    );
    assert!(archive_text(path, "_rels/.rels").contains("3D/3dmodel.model"));
    let xml = archive_text(path, "3D/3dmodel.model");
    assert!(xml.contains("unit=\"millimeter\""));
    assert!(xml.contains("http://schemas.microsoft.com/3dmanufacturing/core/2015/02"));
    assert!(xml.ends_with("</model>"));
    assert_eq!(xml.matches("<object ").count(), 2);
    assert_eq!(xml.matches("<item ").count(), 2);
    assert!(xml.matches("<vertex ").count() >= 16);
    assert!(xml.matches("<triangle ").count() >= 24);
    for object in xml.split("<object ").skip(1) {
        assert_model_object(object);
    }
}

fn archive_text(path: &Path, name: &str) -> String {
    // unzip treats [] as glob syntax; escaping preserves the standard OPC part name.
    let member = name.replace('[', "[[]");
    let part = std::process::Command::new("/usr/bin/unzip")
        .arg("-p")
        .arg(path)
        .arg(member)
        .output()
        .unwrap();
    assert!(part.status.success(), "missing 3MF archive part {name}");
    String::from_utf8(part.stdout).unwrap()
}

fn assert_model_object(object: &str) {
    let vertices = object.matches("<vertex ").count();
    for vertex in object.split("<vertex ").skip(1) {
        let attributes = vertex.split_once("/>").unwrap().0;
        for key in ["x", "y", "z"] {
            assert!(
                attribute(attributes, key)
                    .parse::<f64>()
                    .unwrap()
                    .is_finite()
            );
        }
    }
    for triangle in object.split("<triangle ").skip(1) {
        let attributes = triangle.split_once("/>").unwrap().0;
        let indices =
            ["v1", "v2", "v3"].map(|key| attribute(attributes, key).parse::<usize>().unwrap());
        assert!(indices.iter().all(|index| *index < vertices));
        assert_ne!(indices[0], indices[1]);
        assert_ne!(indices[1], indices[2]);
        assert_ne!(indices[0], indices[2]);
    }
}

fn attribute<'a>(attributes: &'a str, key: &str) -> &'a str {
    attributes
        .split_once(&format!("{key}=\""))
        .unwrap()
        .1
        .split_once('"')
        .unwrap()
        .0
}

#[path = "document_import_lifecycle_tests.rs"]
mod document_import_lifecycle_tests;
