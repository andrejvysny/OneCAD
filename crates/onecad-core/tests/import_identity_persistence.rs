//! A stale certificate remains durable evidence; applicability is resolved by the worker.
use onecad_core::document::record::ImportSourceCodec;
use onecad_core::document::{
    element_index::ElementEntry,
    import_source_identity::ImportSourceIdentity,
    refs::{ElementKind, ElementRef, PrimaryRef},
    Document,
};
use onecad_core::ids::{BodyId, DocumentId, ElementId, SketchId};
use onecad_core::io::container::{ContainerCaches, ContainerReader, ContainerWriter, SaveMeta};
use onecad_core::io::imports::{ImportBlob, ImportBlobs, OwnedSource};
use onecad_core::sketch::{Sketch, SketchAttachment};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

#[test]
fn stale_source_identity_survives_save_and_fresh_open() {
    let source = b"preserved stale source".to_vec();
    let digest = format!("{:x}", Sha256::digest(&source));
    let body = BodyId(Uuid::from_u128(2));
    let element = ElementId::new("el_stale");
    let certificate: ImportSourceIdentity = serde_json::from_value(serde_json::json!({
        "version":1,"sourceSha256":digest,"codec":"step","codecVersion":"occt-step-stream-v1",
        "readerPolicy":"step-v1","kernelFingerprint":"0a6a1dce34181289",
        "producerRecordId":"00000000-0000-0000-0000-000000000001", "bodyId":body,
        "unitScaleBits":"3ff0000000000000","sourceAddress":{"kind":"step",
        "rootEntityLabel":5850,"occurrencePath":[],"faceEntityLabel":3042}}))
    .unwrap();
    let mut document = Document::new(DocumentId(Uuid::from_u128(3)));
    document.elements.insert(
        element.clone(),
        ElementEntry::new(body, ElementKind::Face)
            .with_import_source_identity(Some(certificate.clone())),
    );
    let sketch = Sketch::new(
        SketchId(Uuid::from_u128(4)),
        "stale face",
        SketchAttachment::HostFace {
            face: ElementRef {
                primary: Some(PrimaryRef {
                    body,
                    element,
                    kind: ElementKind::Face,
                    extra: Default::default(),
                }),
                intent: None,
                anchor: None,
                import_source_identity: Some(certificate.clone()),
                extra: Default::default(),
            },
            projected_boundary_version: 0,
        },
    );
    document.sketches.insert(sketch.id, sketch);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("stale.onecad");
    let meta = SaveMeta {
        app_version: "test".into(),
        occt_fingerprint: None,
        created: String::new(),
        modified: String::new(),
    };
    ContainerWriter::save(&path, &document, &ContainerCaches::none(), &meta).unwrap();
    let reopened = ContainerReader::open(&path).unwrap();
    assert_eq!(
        reopened
            .document()
            .sketches
            .values()
            .next()
            .unwrap()
            .host_face()
            .unwrap()
            .import_source_identity
            .as_ref(),
        Some(&certificate)
    );
    let path_with_source = temp.path().join("stale-with-source.onecad");
    let mut imports = ImportBlobs::new();
    imports.insert(
        digest.clone(),
        ImportBlob {
            codec: ImportSourceCodec::Step,
            bytes: Arc::new(OwnedSource::untracked(source.clone())),
        },
    );
    ContainerWriter::save_with_imports(
        &path_with_source,
        &document,
        &ContainerCaches::none(),
        &imports,
        &meta,
    )
    .unwrap();
    let reopened = ContainerReader::open(&path_with_source).unwrap();
    assert_eq!(
        reopened.read_import_blob(&digest).unwrap().as_slice(),
        source
    );
}
