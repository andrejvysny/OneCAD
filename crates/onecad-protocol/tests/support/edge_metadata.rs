use super::*;

fn with_metadata(metadata: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let original = fixture();
    let header = validate_mesh_blob(&original).unwrap();
    let mut sections: Vec<_> = header
        .sections
        .iter()
        .map(|s| {
            (
                s.section_type,
                original[s.offset as usize..s.offset as usize + s.byte_len as usize].to_vec(),
            )
        })
        .collect();
    sections.extend_from_slice(metadata);
    assemble(0x17, &sections)
}

#[test]
fn optional_edge_metadata_preserves_legacy_and_checked_ordinals() {
    use onecad_protocol::mesh::EdgeClass;
    let legacy = fixture();
    let view = decode(&legacy).unwrap();
    assert_eq!(view.edge_class(0), EdgeClass::Unknown);
    assert!(view.edge_incident_faces(0).is_empty());
    for code in 0..=6 {
        let blob = with_metadata(&[
            (14, words(&[code])),
            (15, words(&[0, 1])),
            (16, words(&[0])),
        ]);
        let view = decode(&blob).unwrap();
        assert_eq!(view.edge_class(0) as u32, code);
        assert_eq!(view.edge_incident_faces(0).iter().collect::<Vec<_>>(), [0]);
        assert_eq!(view.edge_id(0), Some("e:5"));
        for invalid in [1, usize::MAX] {
            assert_eq!(view.edge_class(invalid), EdgeClass::Unknown);
            assert!(view.edge_incident_faces(invalid).is_empty());
        }
    }
}

#[test]
fn metadata_lengths_pairing_values_and_adjacency_bounds_are_checked() {
    for (metadata, expected) in [
        (vec![(14, vec![])], Error::Length(14)),
        (vec![(14, words(&[7]))], Error::EdgeClass),
        (vec![(15, words(&[0, 1]))], Error::MissingSection(16)),
        (vec![(16, words(&[0]))], Error::MissingSection(15)),
        (vec![(15, words(&[0, 1])), (16, vec![0])], Error::Length(16)),
        (
            vec![(15, words(&[1, 1])), (16, words(&[0]))],
            Error::Ranges(15),
        ),
        (
            vec![(15, words(&[0, u32::MAX])), (16, words(&[0]))],
            Error::Ranges(15),
        ),
        (
            vec![(15, words(&[0, 1])), (16, words(&[1]))],
            Error::IncidentFace,
        ),
        (
            vec![(15, words(&[0, 2])), (16, words(&[0, 0]))],
            Error::IncidentFaceOrder,
        ),
    ] {
        assert!(
            matches!(decode(&with_metadata(&metadata)), Err(error) if error == expected),
            "expected {expected:?}"
        );
    }
}
