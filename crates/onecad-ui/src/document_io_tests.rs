//! Native typed file routing with the production worker; no physical dialog acceptance.
use super::*;
use crate::{
    bridge::{BackendEvent, EventMailbox, Shared},
    document_flow::ExportFormat,
};
use std::sync::atomic::{AtomicUsize, Ordering};

#[path = "desktop_document_io_tests.rs"]
mod desktop_tests;

#[path = "document_io_tests/deleted_source_repair.rs"]
mod deleted_source_repair;

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
    let paths = crate::qa::QaPaths::from_args(
        ["--qa-root".into(), root.join("qa").into_os_string()].into_iter(),
    )
    .unwrap();
    backend_at(paths).await
}

async fn backend_at(paths: crate::qa::QaPaths) -> (Backend, Arc<Events>) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = instance.request_adapter(&Default::default()).await.unwrap();
    #[cfg(target_os = "macos")]
    if std::env::var("ONECAD_REQUIRE_GPU").as_deref() == Ok("1") {
        assert_eq!(adapter.get_info().backend, wgpu::Backend::Metal);
        eprintln!("Native document IO adapter: {:?}", adapter.get_info());
    }
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            required_features: onecad_render::required_device_features(adapter.features()),
            ..Default::default()
        })
        .await
        .unwrap();
    let renderer =
        onecad_render::Renderer::new(&device, &queue, [0, 0], Default::default()).unwrap();
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
    let shared = Arc::new(Shared::new(
        egui::Context::default(),
        renderer.preparation_budget(),
    ));
    let state = Arc::new(AppState::default());
    state
        .install_archive_budget(onecad_app::resource_admission::ArchiveBudget::new(
            Arc::new(crate::budget::ArchiveBudgetAdapter(
                shared.host_budget.clone(),
            )),
        ))
        .unwrap();
    state.start(host.clone()).unwrap();
    api::new_document(&state, host.clone()).await.unwrap();
    assert!(
        state
            .readiness()
            .wait_ready(std::time::Duration::from_secs(15))
            .await
    );
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

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn native_assembly_capture_save_close_reopen_keeps_occurrences() {
    assert_eq!(std::env::var("ONECAD_REQUIRE_WORKER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("ONECAD_REQUIRE_GPU").as_deref(), Ok("1"));
    use onecad_core::document::{
        Document,
        component::{
            ComponentDef, ComponentDefId, Occurrence, OccurrenceId, Quat, RigidTransform, Vec3d,
        },
    };
    use onecad_core::ids::DocumentId;
    use onecad_core::io::container::{ContainerCaches, ContainerReader, ContainerWriter, SaveMeta};
    let area = tempfile::tempdir().unwrap();
    let (mut backend, _) = backend(area.path()).await;
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../onecad-app/tests/fixtures/qa_plate_hole.onecad");
    let source = ContainerReader::open(&fixture).unwrap();
    let source = source.document();
    let mut document = Document::new(DocumentId(uuid::Uuid::from_u128(0x102)));
    document.variables = source.variables.clone();
    document.sketches = source.sketches.clone();
    document.sketch_visibility = source.sketch_visibility.clone();
    document.datum_planes = source.datum_planes.clone();
    document.timeline = source.timeline.clone();
    document.bodies = source.bodies.clone();
    for d in 0..10u128 {
        let definition = ComponentDefId(uuid::Uuid::from_u128(0x1000 + d));
        let mut component = ComponentDef::new(definition, "Plate");
        component.variables = source.variables.clone();
        component.sketches = source.sketches.clone();
        component.sketch_visibility = source.sketch_visibility.clone();
        component.datum_planes = source.datum_planes.clone();
        component.timeline = source.timeline.clone();
        document.definitions.insert(definition, component);
        for n in 0..2u128 {
            document
                .assembly
                .occurrences
                .insert(Occurrence {
                    id: OccurrenceId(uuid::Uuid::from_u128(0x2000 + 2 * d + n)),
                    def: definition,
                    parent: None,
                    local: RigidTransform::new(
                        if d == 9 && n == 1 {
                            Quat::from_axis_angle(Vec3d::Z, std::f64::consts::PI).unwrap()
                        } else {
                            Quat::IDENTITY
                        },
                        Vec3d::new(200.0 + n as f64 * 50.0, d as f64 * 50.0, 0.0),
                    ),
                    grounded: false,
                    name: format!("Plate {n}"),
                })
                .unwrap();
        }
    }
    let path = backend.paths.root.join("documents/assembly.onecad");
    let meta = SaveMeta {
        app_version: "test".into(),
        occt_fingerprint: None,
        created: String::new(),
        modified: String::new(),
    };
    ContainerWriter::save(&path, &document, &ContainerCaches::none(), &meta).unwrap();
    assert!(matches!(
        intent(&mut backend, Request::Open(Some(path.clone())))
            .await
            .unwrap(),
        Outcome::Replaced { .. }
    ));
    let display = crate::capture::load_display(&mut backend).await.unwrap();
    assert_eq!(display.scene.bodies().len(), 21);
    eprintln!(
        "10-definition 20-occurrence capture: {:?}",
        backend.shared.host_budget.snapshot()
    );
    assert!(display.scene.bodies()[1].shares_geometry(&display.scene.bodies()[2]));
    assert_ne!(
        display.scene.bodies()[1].occurrence_path(),
        display.scene.bodies()[2].occurrence_path()
    );
    let expected_bounds = display.scene.bounds().unwrap();
    drop(display);
    let step = backend.paths.root.join("documents/assembly-export.step");
    api::export_step_file(
        &backend.state,
        backend.host.clone(),
        Some(step.to_string_lossy().into_owned()),
    )
    .await
    .unwrap();
    assert!(step.metadata().unwrap().len() > 0);
    if let Some(path) = std::env::var_os("ONECAD_ASSEMBLY_STEP_CAPTURE") {
        std::fs::copy(&step, path).unwrap();
    }
    let inspection = backend
        .state
        .step_import()
        .inspect_step(&step, false)
        .await
        .unwrap();
    assert_eq!(
        inspection.solid_count, 21,
        "STEP must retain the root body and every occurrence"
    );
    assert_eq!(inspection.source_unit, "MM");
    for (expected, actual) in expected_bounds
        .min
        .to_array()
        .into_iter()
        .zip(inspection.bbox.0)
    {
        assert!(
            (expected - actual).abs() < 1e-3,
            "STEP minimum bound differs: {expected} vs {actual}"
        );
    }
    for (expected, actual) in expected_bounds
        .max
        .to_array()
        .into_iter()
        .zip(inspection.bbox.1)
    {
        assert!(
            (expected - actual).abs() < 1e-3,
            "STEP maximum bound differs: {expected} vs {actual}"
        );
    }
    let session = api::get_projection(&backend.state)
        .await
        .unwrap()
        .runtime_session;
    api::apply_assembly_edit(
        &backend.state,
        &session,
        onecad_core::edit::AssemblyEdit::SetPose {
            occurrence: OccurrenceId(uuid::Uuid::from_u128(0x2001)),
            pose: RigidTransform::new(Quat::IDENTITY, Vec3d::new(200.0, 0.0, 0.0)),
        },
    )
    .await
    .unwrap();
    let protected = backend.paths.root.join("documents/protected.step");
    fs::write(&protected, b"existing target").unwrap();
    let error = api::export_step_file(
        &backend.state,
        backend.host.clone(),
        Some(protected.to_string_lossy().into_owned()),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("coincident identical solids"));
    assert_eq!(fs::read(&protected).unwrap(), b"existing target");
    api::apply_assembly_edit(
        &backend.state,
        &session,
        onecad_core::edit::AssemblyEdit::SetPose {
            occurrence: OccurrenceId(uuid::Uuid::from_u128(0x2001)),
            pose: RigidTransform::new(Quat::IDENTITY, Vec3d::new(250.0, 0.0, 0.0)),
        },
    )
    .await
    .unwrap();
    assert!(
        api::save_document(&backend.state, backend.host.clone(), None, None)
            .await
            .unwrap()
            .clean
    );
    api::close_document(&backend.state, backend.host.clone())
        .await
        .unwrap();
    assert!(matches!(
        intent(&mut backend, Request::Open(Some(path)))
            .await
            .unwrap(),
        Outcome::Replaced { .. }
    ));
    let reopened = crate::capture::load_display(&mut backend).await.unwrap();
    assert_eq!(reopened.scene.bodies().len(), 21);
    eprintln!(
        "10-definition 20-occurrence reopen: {:?}",
        backend.shared.host_budget.snapshot()
    );
    assert!(reopened.scene.bodies()[1].shares_geometry(&reopened.scene.bodies()[2]));
    drop(reopened);
    api::close_document(&backend.state, backend.host.clone())
        .await
        .unwrap();
    backend.state.retire_all();
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn native_assembly_controls_author_solve_undo_save_reopen() {
    assert_eq!(std::env::var("ONECAD_REQUIRE_WORKER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("ONECAD_REQUIRE_GPU").as_deref(), Ok("1"));
    assert!(
        Path::new(
            &std::env::var_os("ONECAD_WORKER_PATH").expect("explicit production worker required")
        )
        .is_file()
    );
    use crate::assembly_controls::{Action, Request as AssemblyRequest};
    use onecad_assembly::{
        joint::{Joint, JointId, JointKind, JointOrigin},
        math::{Frame, RigidTransform, Vec3d},
        occurrence::{ComponentDefId, Occurrence, OccurrenceId},
    };
    use onecad_core::edit::AssemblyEdit;
    let area = tempfile::tempdir().unwrap();
    let (mut backend, _) = backend(area.path()).await;
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../onecad-app/tests/fixtures/qa_plate_hole.onecad");
    let path = backend.paths.root.join("documents/part.onecad");
    fs::copy(source, &path).unwrap();
    intent(&mut backend, Request::Open(Some(path)))
        .await
        .unwrap();
    let session = api::get_projection(&backend.state)
        .await
        .unwrap()
        .runtime_session;
    async fn command(
        backend: &mut Backend,
        session: &str,
        action: Action,
    ) -> Result<crate::assembly_controls::Reply, String> {
        let generation = api::get_assembly_projection(&backend.state, session)
            .await
            .unwrap()
            .persisted_generation;
        crate::assembly_controls::run(
            backend,
            AssemblyRequest {
                sequence: 1,
                session: session.into(),
                generation,
                cancellation: matches!(&action, Action::Solve { .. } | Action::SolveBound { .. })
                    .then(|| Arc::new(std::sync::atomic::AtomicBool::new(false))),
                action,
            },
        )
        .await
    }
    let definition = ComponentDefId(uuid::Uuid::from_u128(0xC1));
    command(
        &mut backend,
        &session,
        Action::Edit(AssemblyEdit::CopyRoot {
            id: definition,
            name: "Part".into(),
        }),
    )
    .await
    .unwrap();
    for n in 0..2u128 {
        command(
            &mut backend,
            &session,
            Action::Edit(AssemblyEdit::InsertOccurrence {
                occurrence: Occurrence {
                    id: OccurrenceId(uuid::Uuid::from_u128(0xC2 + n)),
                    def: definition,
                    parent: None,
                    local: RigidTransform::IDENTITY,
                    grounded: n == 0,
                    name: format!("Instance {n}"),
                },
            }),
        )
        .await
        .unwrap();
    }
    crate::assembly_controls::definition_edit_tests::qualify(&mut backend, &session, definition)
        .await;
    let before = api::get_assembly_projection(&backend.state, &session)
        .await
        .unwrap();
    let stale = crate::assembly_controls::run(
        &mut backend,
        AssemblyRequest {
            sequence: 2,
            session: session.clone(),
            generation: before.persisted_generation.saturating_sub(1),
            cancellation: None,
            action: Action::Edit(AssemblyEdit::SetVisibility {
                occurrence: OccurrenceId(uuid::Uuid::from_u128(0xC3)),
                visible: false,
            }),
        },
    )
    .await;
    assert!(stale.is_err());
    assert!(
        api::get_assembly_projection(&backend.state, &session)
            .await
            .unwrap()
            .occurrences
            .iter()
            .all(|item| item.visible)
    );
    let frame = Frame {
        origin: Vec3d::ZERO,
        z: Vec3d::Z,
        x: Vec3d::X,
    };
    let joint = Joint {
        id: JointId(uuid::Uuid::from_u128(0xC4)),
        name: "Slider".into(),
        kind: JointKind::Slider,
        a: JointOrigin {
            occurrence: OccurrenceId(uuid::Uuid::from_u128(0xC2)),
            frame,
        },
        b: JointOrigin {
            occurrence: OccurrenceId(uuid::Uuid::from_u128(0xC3)),
            frame,
        },
        limits: None,
        value: 2.0,
        suppressed: false,
    };
    command(
        &mut backend,
        &session,
        Action::Edit(AssemblyEdit::SetPose {
            occurrence: OccurrenceId(uuid::Uuid::from_u128(0xC2)),
            pose: RigidTransform::new(
                onecad_assembly::math::Quat::IDENTITY,
                Vec3d::new(100.0, 0.0, 0.0),
            ),
        }),
    )
    .await
    .unwrap();
    Box::pin(async {
        let display = crate::capture::load_display(&mut backend).await.unwrap();
        backend.shared.publish_display(display);
        let display = backend
            .shared
            .display
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .clone();
        let target = display
            .scene
            .bodies()
            .iter()
            .find(|body| {
                body.occurrence_path()
                    .and_then(|path| path.0.last())
                    .is_some_and(|id| id.0 == 0xC2)
            })
            .unwrap();
        let display_body = target.body_id();
        let bounds = target.bounds();
        let center = (bounds.min + bounds.max) * 0.5;
        let camera = onecad_scene::Camera {
            target: center,
            offset: onecad_scene::DVec3::X * 80.0,
            ..Default::default()
        };
        let viewport = onecad_scene::Viewport {
            width: 800.0,
            height: 600.0,
            scale_factor: 1.0,
        };
        let view =
            onecad_scene::ViewSnapshot::new(display.scene.identity(), 1, camera, viewport).unwrap();
        let mesh_generation = display
            .assembly
            .definitions
            .iter()
            .find(|d| d.id == definition.to_string())
            .unwrap()
            .mesh_generation
            .unwrap();
        let path = onecad_core::document::component::OccurrencePath(vec![OccurrenceId(
            uuid::Uuid::from_u128(0xC2),
        )]);
        let mut selected = None;
        let mut tried = std::collections::BTreeSet::new();
        'pick: for x in (20..780).step_by(2) {
            for y in (20..580).step_by(2) {
                let hit = onecad_scene::pick_with_policy(
                    &display.scene,
                    &view,
                    view.identity(),
                    onecad_scene::DVec2::new(x as f64, y as f64),
                    onecad_scene::PickPolicy::FacesAndEdges,
                )
                .unwrap();
                let Some(hit) = hit.filter(|hit| {
                    hit.body_id == display_body
                        && hit.kind == onecad_scene::PickKind::Face
                        && hit.ids_have_element_ids
                }) else {
                    continue;
                };
                if !tried.insert(hit.topology_id.to_owned()) {
                    continue;
                }
                let result = api::component_joint_origin(
                    &backend.state,
                    api::JointOriginRequest {
                        session: &session,
                        generation: display.assembly.persisted_generation,
                        path: &path,
                        body: hit.source_body_id,
                        mesh_generation,
                        element: hit.topology_id,
                        x: Vec3d::Z,
                    },
                )
                .await;
                if let Ok(resolved) = result {
                    selected = crate::viewport_selection::OccurrenceSelection::from_hit(&hit)
                        .map(|selection| (selection, x, y, resolved.frame));
                    break 'pick;
                }
            }
        }
        let (selected, picked_x, picked_y, picked_frame) =
            selected.expect("visible anchored occurrence face must be pickable");
        let pick = crate::assembly_controls::OriginPick {
            side: onecad_core::document::component::JointSide::A,
            selection: selected,
            mesh_generation,
        };
        let mut picked_joint = joint.clone();
        picked_joint.kind = JointKind::Rigid;
        picked_joint.value = 0.0;
        picked_joint.a.frame.x = Vec3d::Z;
        picked_joint.b.frame = picked_frame;
        picked_joint.b.frame.origin.x += 100.0;
        drop(display);
        let stale = crate::assembly_controls::run(
            &mut backend,
            AssemblyRequest {
                sequence: 7,
                session: "stale-session".into(),
                generation: before.persisted_generation,
                cancellation: Some(Arc::new(std::sync::atomic::AtomicBool::new(false))),
                action: Action::SolveBound {
                    joints: vec![joint.clone()],
                    drives: vec![],
                    joint: joint.id,
                    picks: vec![pick.clone()],
                    unbound: vec![],
                },
            },
        )
        .await;
        assert!(stale.is_err());
        assert!(
            api::get_assembly_projection(&backend.state, &session)
                .await
                .unwrap()
                .joint_bindings
                .is_empty()
        );
        let picked = command(
            &mut backend,
            &session,
            Action::SolveBound {
                joints: vec![picked_joint.clone()],
                drives: vec![],
                joint: joint.id,
                picks: vec![pick.clone()],
                unbound: vec![],
            },
        )
        .await
        .unwrap();
        assert!(picked.changed);
        assert!(
            !api::get_assembly_projection(&backend.state, &session)
                .await
                .unwrap()
                .joint_bindings[0]
                .needs_repair
        );
        let before_repair = api::get_assembly_projection(&backend.state, &session)
            .await
            .unwrap();
        command(
            &mut backend,
            &session,
            Action::Edit(AssemblyEdit::EditDefinition {
                id: definition,
                command: Box::new(onecad_core::edit::EditCommand::AddVariable {
                    variable: onecad_core::document::variables::Variable {
                        id: onecad_core::ids::VariableId(uuid::Uuid::from_u128(0xC5)),
                        name: "repair_probe".into(),
                        value: onecad_core::document::variables::Scalar::new(1.0),
                        unit: Default::default(),
                    },
                }),
            }),
        )
        .await
        .unwrap();
        let damaged = api::get_assembly_projection(&backend.state, &session)
            .await
            .unwrap();
        assert!(damaged.joint_bindings[0].needs_repair);
        assert_eq!(
            before_repair
                .occurrences
                .iter()
                .map(|o| o.local)
                .collect::<Vec<_>>(),
            damaged
                .occurrences
                .iter()
                .map(|o| o.local)
                .collect::<Vec<_>>()
        );
        let stale_pick = command(
            &mut backend,
            &session,
            Action::SolveBound {
                joints: damaged.joints.clone(),
                drives: vec![],
                joint: joint.id,
                picks: vec![pick],
                unbound: vec![],
            },
        )
        .await;
        assert!(stale_pick.is_err());
        let display = backend
            .shared
            .display
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .clone();
        let view =
            onecad_scene::ViewSnapshot::new(display.scene.identity(), 1, camera, viewport).unwrap();
        let target = display
            .scene
            .bodies()
            .iter()
            .find(|body| {
                body.occurrence_path()
                    .and_then(|p| p.0.last())
                    .is_some_and(|id| id.0 == 0xC2)
            })
            .unwrap();
        let display_body = target.body_id();
        let hit = onecad_scene::pick_with_policy(
            &display.scene,
            &view,
            view.identity(),
            onecad_scene::DVec2::new(picked_x as f64, picked_y as f64),
            onecad_scene::PickPolicy::FacesAndEdges,
        )
        .unwrap()
        .unwrap();
        assert_eq!(hit.body_id, display_body);
        let selection = crate::viewport_selection::OccurrenceSelection::from_hit(&hit).unwrap();
        let mesh_generation = display
            .assembly
            .definitions
            .iter()
            .find(|d| d.id == definition.to_string())
            .unwrap()
            .mesh_generation
            .unwrap();
        let repick = crate::assembly_controls::OriginPick {
            side: onecad_core::document::component::JointSide::A,
            selection,
            mesh_generation,
        };
        drop(display);
        command(
            &mut backend,
            &session,
            Action::SolveBound {
                joints: damaged.joints,
                drives: vec![],
                joint: joint.id,
                picks: vec![repick],
                unbound: vec![],
            },
        )
        .await
        .unwrap();
        assert!(
            !api::get_assembly_projection(&backend.state, &session)
                .await
                .unwrap()
                .joint_bindings[0]
                .needs_repair
        );
        let current = api::get_assembly_projection(&backend.state, &session)
            .await
            .unwrap();
        command(
            &mut backend,
            &session,
            Action::SolveBound {
                joints: current.joints,
                drives: vec![],
                joint: joint.id,
                picks: vec![],
                unbound: vec![onecad_core::document::component::JointSide::A],
            },
        )
        .await
        .unwrap();
        assert!(
            api::get_assembly_projection(&backend.state, &session)
                .await
                .unwrap()
                .joint_bindings
                .is_empty()
        );
        api::undo(&backend.state, backend.host.clone())
            .await
            .unwrap();
        api::undo(&backend.state, backend.host.clone())
            .await
            .unwrap();
        api::undo(&backend.state, backend.host.clone())
            .await
            .unwrap();
        api::undo(&backend.state, backend.host.clone())
            .await
            .unwrap();
        api::undo(&backend.state, backend.host.clone())
            .await
            .unwrap();
    })
    .await;
    backend.shared.attach_test_state(&backend.state);
    let before_cancel = api::get_assembly_projection(&backend.state, &session)
        .await
        .unwrap();
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let locked = backend.state.runtime.lock().await;
    let (started, ready) = tokio::sync::oneshot::channel();
    let worker_state = backend.state.clone();
    let worker_session = session.clone();
    let worker_token = cancel.clone();
    let candidate = joint.clone();
    let worker = tokio::spawn(async move {
        let _ = started.send(());
        onecad_app::joint_execution::solve_joints_at_generation_cancellable(
            &worker_state,
            &worker_session,
            before_cancel.persisted_generation,
            vec![candidate.clone()],
            vec![(candidate.id, 2.0)],
            worker_token,
        )
        .await
    });
    ready.await.unwrap();
    crate::assembly_controls::cancel_pending(&backend.shared, &cancel).unwrap();
    drop(locked);
    assert!(worker.await.unwrap().is_err());
    let after_cancel = api::get_assembly_projection(&backend.state, &session)
        .await
        .unwrap();
    assert!(after_cancel.joints.is_empty());
    assert!(
        after_cancel
            .occurrences
            .iter()
            .all(|occurrence| occurrence.local == RigidTransform::IDENTITY)
    );
    let report = command(
        &mut backend,
        &session,
        Action::Solve {
            joints: vec![joint.clone()],
            drives: vec![(joint.id, 2.0)],
        },
    )
    .await
    .unwrap()
    .report
    .unwrap();
    assert_eq!(report.remaining_dof, 0);
    assert_eq!(
        api::get_assembly_projection(&backend.state, &session)
            .await
            .unwrap()
            .joints
            .len(),
        1
    );
    api::undo(&backend.state, backend.host.clone())
        .await
        .unwrap();
    assert!(
        api::get_assembly_projection(&backend.state, &session)
            .await
            .unwrap()
            .joints
            .is_empty()
    );
    api::redo(&backend.state, backend.host.clone())
        .await
        .unwrap();
    let mut edited = joint.clone();
    edited.value = 1.5;
    command(
        &mut backend,
        &session,
        Action::Solve {
            joints: vec![edited],
            drives: vec![(joint.id, 1.5)],
        },
    )
    .await
    .unwrap();
    command(
        &mut backend,
        &session,
        Action::Edit(AssemblyEdit::SetGrounded {
            occurrence: OccurrenceId(uuid::Uuid::from_u128(0xC3)),
            grounded: true,
        }),
    )
    .await
    .unwrap();
    command(
        &mut backend,
        &session,
        Action::Solve {
            joints: Vec::new(),
            drives: Vec::new(),
        },
    )
    .await
    .unwrap();
    assert!(
        api::get_assembly_projection(&backend.state, &session)
            .await
            .unwrap()
            .joints
            .is_empty()
    );
    api::undo(&backend.state, backend.host.clone())
        .await
        .unwrap();
    command(
        &mut backend,
        &session,
        Action::Edit(AssemblyEdit::SetGrounded {
            occurrence: OccurrenceId(uuid::Uuid::from_u128(0xC3)),
            grounded: false,
        }),
    )
    .await
    .unwrap();
    let saved = backend.paths.root.join("documents/assembly-saved.onecad");
    assert!(
        api::save_document(
            &backend.state,
            backend.host.clone(),
            Some(saved.to_string_lossy().into_owned()),
            None
        )
        .await
        .unwrap()
        .clean
    );
    api::close_document(&backend.state, backend.host.clone())
        .await
        .unwrap();
    intent(&mut backend, Request::Open(Some(saved)))
        .await
        .unwrap();
    let reopened_session = api::get_projection(&backend.state)
        .await
        .unwrap()
        .runtime_session;
    let assembly = api::get_assembly_projection(&backend.state, &reopened_session)
        .await
        .unwrap();
    assert_eq!((assembly.occurrences.len(), assembly.joints.len()), (2, 1));
    assert_eq!(assembly.joints[0].value, 1.5);
    assert!(
        backend
            .state
            .readiness()
            .wait_ready(std::time::Duration::from_secs(15))
            .await
    );
    let first_display = crate::services::load_ready_display(&mut backend)
        .await
        .unwrap();
    let root_count = first_display
        .projection
        .bodies
        .values()
        .filter(|body| body.visible)
        .count();
    assert!(root_count > 0);
    backend.shared.publish_display(first_display);
    let target = crate::budget::MAX_BODIES - root_count;
    for n in 2..target {
        api::apply_assembly_edit(
            &backend.state,
            &reopened_session,
            AssemblyEdit::InsertOccurrence {
                occurrence: Occurrence {
                    id: OccurrenceId(uuid::Uuid::from_u128(0x10000 + n as u128)),
                    def: definition,
                    parent: None,
                    local: RigidTransform::IDENTITY,
                    grounded: false,
                    name: format!("Capacity {n}"),
                },
            },
        )
        .await
        .unwrap();
    }
    let full = crate::capture::load_display(&mut backend).await.unwrap();
    assert_eq!(full.scene.bodies().len(), crate::budget::MAX_BODIES);
    backend.shared.publish_display(full);
    let before = api::get_projection(&backend.state).await.unwrap();
    let generation = api::get_assembly_projection(&backend.state, &reopened_session)
        .await
        .unwrap()
        .persisted_generation;
    let rejected = crate::assembly_controls::run(
        &mut backend,
        AssemblyRequest {
            sequence: 3,
            session: reopened_session.clone(),
            generation,
            cancellation: None,
            action: Action::Edit(AssemblyEdit::InsertOccurrence {
                occurrence: Occurrence {
                    id: OccurrenceId(uuid::Uuid::from_u128(0x20000)),
                    def: definition,
                    parent: None,
                    local: RigidTransform::IDENTITY,
                    grounded: false,
                    name: "Over capacity".into(),
                },
            }),
        },
    )
    .await;
    assert!(rejected.is_err());
    let after = api::get_projection(&backend.state).await.unwrap();
    assert_eq!(after.undo_depth, before.undo_depth);
    assert_eq!(
        api::get_assembly_projection(&backend.state, &reopened_session)
            .await
            .unwrap()
            .occurrences
            .len(),
        target
    );
    assert!(
        backend
            .shared
            .valid
            .load(std::sync::atomic::Ordering::Acquire)
    );
    backend
        .shared
        .fail_next_assembly_capture
        .store(true, std::sync::atomic::Ordering::Release);
    let generation = api::get_assembly_projection(&backend.state, &reopened_session)
        .await
        .unwrap()
        .persisted_generation;
    let accepted = crate::assembly_controls::run(
        &mut backend,
        AssemblyRequest {
            sequence: 4,
            session: reopened_session.clone(),
            generation,
            cancellation: None,
            action: Action::Edit(AssemblyEdit::SetVisibility {
                occurrence: OccurrenceId(uuid::Uuid::from_u128(0xC3)),
                visible: false,
            }),
        },
    )
    .await
    .unwrap();
    assert!(accepted.changed && accepted.capture_error.is_some());
    assert!(
        !backend
            .shared
            .valid
            .load(std::sync::atomic::Ordering::Acquire)
    );
    let permit = Arc::new(tokio::sync::Semaphore::new(1))
        .acquire_owned()
        .await
        .unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    crate::services::execute(&mut backend, crate::bridge::Command::Undo, permit, &sender).await;
    assert!(matches!(
        receiver.recv().unwrap(),
        crate::bridge::Message::Terminal { result: Ok(_), .. }
    ));
    assert!(
        backend
            .shared
            .valid
            .load(std::sync::atomic::Ordering::Acquire)
    );
    assert!(
        api::get_assembly_projection(&backend.state, &reopened_session)
            .await
            .unwrap()
            .occurrences
            .iter()
            .find(|occurrence| occurrence.id == uuid::Uuid::from_u128(0xC3).to_string())
            .unwrap()
            .visible
    );
    api::close_document(&backend.state, backend.host.clone())
        .await
        .unwrap();
    backend.state.retire_all();
}

#[path = "document_io_tests/repeated_occurrence_repair.rs"]
mod repeated_occurrence_repair;

#[path = "document_io_tests/deleted_body_repair.rs"]
mod deleted_body_repair;
