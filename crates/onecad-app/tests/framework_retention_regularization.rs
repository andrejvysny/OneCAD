//! Imported captured operands exercise a local Boolean rewrite, not source equivalence.
#[allow(dead_code)]
#[path = "native_plate_support/host.rs"]
mod host;

use host::Harness;
use onecad_app::{api, dto::DocumentProjection};
use onecad_core::{
    document::{
        record::{
            BooleanOp, BooleanParams, ImportSourceCodec, ImportStepParams, KnownOperation,
            Operation, OperationRecord, IMPORT_HEAL_POLICY_V1,
        },
        variables::Scalar,
        Document,
    },
    edit::EditCommand,
    ids::{BodyId, DocumentId, RecordId},
    io::{
        container::{ContainerCaches, ContainerWriter, SaveMeta},
        imports::{ImportBlob, ImportBlobs, OwnedSource},
    },
};
use onecad_protocol::mesh::{MeshLimits, MeshView};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path, sync::Arc};
use uuid::Uuid;

const DATA: [(&str, &str, &[u8]); 3] = [
    (
        "p.brep",
        "f215fd6dc72943da29fbfbdfdc3136bc7d95306d264b7284c11967d888bd34bf",
        include_bytes!("../../../fixtures/qualification/framework-retention/p.brep"),
    ),
    (
        "sharp.brep",
        "65602a458049e58d7e8193169d1abf45b97c63ac32783537c7cbddd61a5c6355",
        include_bytes!("../../../fixtures/qualification/framework-retention/sharp.brep"),
    ),
    (
        "c69.brep",
        "137820d1a285b08c9e8a9ddaccc895be56acf494b0f76ed7189ebe5875279c61",
        include_bytes!("../../../fixtures/qualification/framework-retention/c69.brep"),
    ),
];
fn rid(n: u128) -> RecordId {
    RecordId(Uuid::from_u128(0xF100 + n))
}

fn fixture(path: &Path) {
    let mut doc = Document::new(DocumentId(Uuid::from_u128(0xF000)));
    let mut blobs = ImportBlobs::new();
    for (i, (name, sha, bytes)) in DATA.iter().enumerate() {
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), *sha);
        let params = ImportStepParams {
            source_sha256: (*sha).into(),
            source_codec: ImportSourceCodec::Brep,
            source_name: (*name).into(),
            heal_policy: IMPORT_HEAL_POLICY_V1.into(),
            unit_scale: Scalar::new(1.0),
            brep_format: Some(4),
            provenance_sha256: None,
            extra: Default::default(),
        };
        doc.timeline.insert_at_cursor(OperationRecord::new(
            rid(i as u128),
            i as u32,
            *name,
            Operation::Known(KnownOperation::ImportStep(params)),
        ));
        blobs.insert(
            (*sha).into(),
            ImportBlob {
                codec: ImportSourceCodec::Brep,
                bytes: Arc::new(OwnedSource::untracked(bytes.to_vec())),
            },
        );
    }
    let meta = SaveMeta {
        app_version: "framework-retention-test".into(),
        occt_fingerprint: None,
        created: String::new(),
        modified: String::new(),
    };
    ContainerWriter::save_with_imports(path, &doc, &ContainerCaches::none(), &blobs, &meta)
        .unwrap();
}

async fn open(h: &Harness, path: &Path) -> DocumentProjection {
    api::open_document(
        &h.state,
        h.host.clone(),
        path.to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    h.settled(&h.projection().await).await
}
fn imported(p: &DocumentProjection, index: u128) -> BodyId {
    let bodies: Vec<_> = p
        .bodies
        .values()
        .filter(|b| b.created_by == rid(index).to_string())
        .collect();
    assert_eq!(bodies.len(), 1, "import {index}: {p:?}");
    bodies[0].id.parse().unwrap()
}
async fn boolean(
    h: &Harness,
    id: u128,
    op: BooleanOp,
    target: BodyId,
    tool: BodyId,
) -> onecad_app::dto::RegenFinished {
    let record = OperationRecord::new(
        rid(id),
        id as u32,
        "Retention diagnostic",
        Operation::Known(KnownOperation::Boolean(BooleanParams::single(
            op, target, tool,
        ))),
    );
    let applied = api::apply_operation(
        &h.state,
        h.host.clone(),
        vec![EditCommand::AddOperation {
            record,
            at_cursor: true,
        }],
        None,
    )
    .await
    .unwrap();
    h.terminal(&applied.projection).await
}
async fn checked_mesh(h: &Harness, p: &DocumentProjection, id: &str) -> String {
    let mesh = api::get_mesh(
        &h.state,
        id.to_owned(),
        "fine".into(),
        None,
        Some(p.runtime_session.clone()),
    )
    .await
    .unwrap();
    let decoded = MeshView::decode(&mesh, MeshLimits::default()).unwrap();
    assert!(decoded.header().triangle_count > 0, "empty mesh for {id}");
    format!("{:x}", Sha256::digest(&mesh))
}

async fn body_meshes(h: &Harness, p: &DocumentProjection) -> BTreeMap<String, String> {
    let mut hashes = BTreeMap::new();
    for id in p.bodies.keys() {
        hashes.insert(id.clone(), checked_mesh(h, p, id).await);
    }
    hashes
}

async fn check_children(
    h: &Harness,
    p: &DocumentProjection,
    ids: &[String],
) -> BTreeMap<String, (u64, String)> {
    assert_eq!(ids.len(), 2, "both split children must publish: {ids:?}");
    assert_ne!(ids[0], ids[1]);
    let mut evidence = BTreeMap::new();
    let mut volumes = Vec::new();
    for id in ids {
        assert_eq!(p.bodies[id].created_by, rid(11).to_string());
        let mesh_hash = checked_mesh(h, p, id).await;
        let body: BodyId = id.parse().unwrap();
        let mass = api::query_mass_properties(
            &h.state,
            onecad_app::worker::wire::body_id_wire(body),
            None,
        )
        .await
        .unwrap();
        assert!(
            mass.volume.is_finite() && mass.volume > 0.0,
            "child {id}: {mass:?}"
        );
        eprintln!(
            "split child {id} volume {} mm³, mesh SHA-256 {mesh_hash}",
            mass.volume
        );
        volumes.push(mass.volume);
        evidence.insert(id.clone(), (mass.volume.to_bits(), mesh_hash));
    }
    let small = volumes[0].min(volumes[1]);
    let large = volumes[0].max(volumes[1]);
    // This broad size-class check is not a manufacturing or source-accuracy tolerance.
    assert!(
        small / large < 1e-5,
        "expected a tiny split child: {volumes:?}"
    );
    evidence
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_retention_regularization_preserves_split_children() {
    assert_eq!(std::env::var("ONECAD_REQUIRE_WORKER").as_deref(), Ok("1"));
    host::required_worker();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("inputs.onecad");
    fixture(&path);
    let baseline = Harness::new(&root.path().join("baseline")).await;
    let p = open(&baseline, &path).await;
    let (target, cutter) = (imported(&p, 0), imported(&p, 2));
    let before = body_meshes(&baseline, &p).await;
    let result = boolean(&baseline, 10, BooleanOp::Cut, target, cutter).await;
    eprintln!("baseline Cut completion: {result:?}");
    let failed = result
        .failed_steps
        .iter()
        .find(|step| step.record_id == rid(10).to_string())
        .expect("authored baseline Cut must report a failed step");
    assert!(
        format!("{failed:?}").contains("PUBLICATION_OPEN_MANIFOLD"),
        "{failed:?}"
    );
    let after = baseline.projection().await;
    assert_eq!(
        body_meshes(&baseline, &after).await,
        before,
        "refused Cut must preserve each checked fine mesh byte-for-byte"
    );
    baseline.close().await;
    let h = Harness::new(&root.path().join("rewrite")).await;
    let p = open(&h, &path).await;
    let (target, sharp, cutter) = (imported(&p, 0), imported(&p, 1), imported(&p, 2));
    let intersect = boolean(&h, 10, BooleanOp::Intersect, target, sharp).await;
    assert_eq!(intersect.outcome, "published", "{intersect:?}");
    assert!(
        intersect.failed_steps.is_empty() && intersect.repair_steps.is_empty(),
        "{intersect:?}"
    );
    let cut = boolean(&h, 11, BooleanOp::Cut, target, cutter).await;
    assert_eq!(cut.outcome, "published", "{cut:?}");
    assert!(
        cut.failed_steps.is_empty() && cut.repair_steps.is_empty(),
        "{cut:?}"
    );
    let p = h.projection().await;
    let ids = cut
        .affected_bodies
        .get(&rid(11).to_string())
        .expect("split lineage");
    assert!(
        !p.bodies.contains_key(&target.to_string()),
        "split parent retired"
    );
    let before_save = check_children(&h, &p, ids).await;
    let saved = root.path().join("saved.onecad");
    assert!(
        api::save_document(
            &h.state,
            h.host.clone(),
            Some(saved.to_string_lossy().into_owned()),
            None
        )
        .await
        .unwrap()
        .clean
    );
    h.close().await;
    let reopened = Harness::new(&root.path().join("fresh-worker")).await;
    let p = open(&reopened, &saved).await;
    let after_reopen = check_children(&reopened, &p, ids).await;
    assert_eq!(
        before_save, after_reopen,
        "child volume bits and checked meshes must survive replay"
    );
    reopened.close().await;
}
