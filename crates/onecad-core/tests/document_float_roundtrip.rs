use onecad_core::{
    document::{
        variables::{Scalar, Variable},
        Document,
    },
    ids::{DocumentId, VariableId},
    io::container::{ContainerCaches, ContainerReader, ContainerWriter, SaveMeta},
};

fn coordinate_bits(document: &Document) -> u64 {
    document
        .variables
        .get("precise_coordinate")
        .unwrap()
        .value
        .value
        .to_bits()
}

#[test]
fn document_parameter_bits_survive_json_and_container_roundtrip() {
    let value = f64::from_bits(0xc035_9d3a_5c9b_b28b);
    let mut document = Document::new(DocumentId::new());
    document.variables.upsert(Variable {
        id: VariableId::new(),
        name: "precise_coordinate".into(),
        value: Scalar::new(value),
        unit: Default::default(),
    });
    let bytes = serde_json::to_vec(&document).unwrap();
    let reopened: Document = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        coordinate_bits(&reopened),
        value.to_bits(),
        "document JSON changed coordinate bits"
    );

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("precise.onecad");
    let meta = SaveMeta {
        app_version: "test".into(),
        occt_fingerprint: None,
        created: String::new(),
        modified: String::new(),
    };
    ContainerWriter::save(&path, &document, &ContainerCaches::none(), &meta).unwrap();
    let reopened = ContainerReader::open(&path).unwrap();
    assert_eq!(
        coordinate_bits(reopened.document()),
        value.to_bits(),
        "container changed coordinate bits"
    );
}
