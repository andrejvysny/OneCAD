use onecad_protocol::mesh::{
    validate_mesh_blob, MeshDecodeError as Error, MeshLimits, MeshView, MESH_MAGIC_LE,
};
use std::path::Path;

fn put(blob: &mut [u8], off: usize, value: u32) {
    blob[off..off + 4].copy_from_slice(&value.to_le_bytes());
}

fn words(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn fixture() -> Vec<u8> {
    assemble(
        0x17,
        &[
            (
                1,
                words(&[0, 0, 0, 1f32.to_bits(), 0, 0, 0, 1f32.to_bits(), 0]),
            ),
            (
                2,
                words(&[
                    0,
                    0,
                    1f32.to_bits(),
                    0,
                    0,
                    1f32.to_bits(),
                    0,
                    0,
                    1f32.to_bits(),
                ]),
            ),
            (3, words(&[0, 1, 2])),
            (4, words(&[0, 1])),
            (5, words(&[0, 3])),
            (6, b"f:7".to_vec()),
            (7, words(&[0, 2])),
            (8, words(&[0, 0, 0, 1f32.to_bits(), 0, 0])),
            (9, words(&[0, 3])),
            (10, b"e:5".to_vec()),
            (11, words(&[0, 0, 0, 1f32.to_bits(), 1f32.to_bits(), 0])),
            (12, vec![12, 34, 56, 255]),
        ],
    )
}

fn assemble(flags: u16, sections: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut blob = vec![0; 64 + sections.len() * 16];
    put(&mut blob, 0, MESH_MAGIC_LE);
    blob[4..6].copy_from_slice(&1u16.to_le_bytes());
    blob[6..8].copy_from_slice(&flags.to_le_bytes());
    for (off, value) in [(8, 3), (12, 1), (16, 1), (20, 1), (24, 2)] {
        put(&mut blob, off, value);
    }
    put(&mut blob, 44, 1f32.to_bits());
    put(&mut blob, 48, 1f32.to_bits());
    blob[30..32].copy_from_slice(&(sections.len() as u16).to_le_bytes());
    for (i, (ty, data)) in sections.iter().enumerate() {
        let entry = 64 + i * 16;
        let offset = blob.len() as u32;
        put(&mut blob, entry, *ty);
        put(&mut blob, entry + 8, offset);
        put(&mut blob, entry + 12, data.len() as u32);
        blob.extend_from_slice(data);
        while !blob.len().is_multiple_of(4) {
            blob.push(0);
        }
    }
    blob
}

fn data_offset(blob: &[u8], ty: u32) -> usize {
    validate_mesh_blob(blob)
        .unwrap()
        .section(ty)
        .unwrap()
        .offset as usize
}

fn decode(blob: &[u8]) -> Result<MeshView<'_>, Error> {
    MeshView::decode(blob, MeshLimits::default())
}

#[test]
fn checked_view_borrows_and_exposes_safe_indexed_data() {
    let blob = fixture();
    let mut unaligned = vec![0xff];
    unaligned.extend_from_slice(&blob);
    let view = decode(&unaligned[1..]).unwrap();
    assert_eq!(view.bytes().as_ptr(), unaligned[1..].as_ptr());
    assert_eq!(view.position(1), Some([1.0, 0.0, 0.0]));
    assert_eq!(view.normal(0), Some([0.0, 0.0, 1.0]));
    assert_eq!(view.triangle(0), Some([0, 1, 2]));
    assert_eq!(view.face_range(0).unwrap().count, 1);
    assert_eq!(view.edge_range(0).unwrap().count, 2);
    assert_eq!(view.edge_point(1), Some([1.0, 0.0, 0.0]));
    assert_eq!(view.face_id(0), Some("f:7"));
    assert_eq!(view.edge_id(0), Some("e:5"));
    assert_eq!(view.face_color(0), Some([12, 34, 56, 255]));
    assert_eq!(view.face_bbox(0), Some(([0.0; 3], [1.0, 1.0, 0.0])));
    assert_eq!(view.positions().iter().count(), 9);
    assert_eq!(view.indices().iter().collect::<Vec<_>>(), [0, 1, 2]);
    for index in [1, usize::MAX] {
        assert_eq!(view.face_id(index), None);
        assert_eq!(view.edge_id(index), None);
        assert_eq!(view.triangle(index), None);
        assert_eq!(view.face_range(index), None);
        assert_eq!(view.edge_range(index), None);
        assert_eq!(view.face_bbox(index), None);
        assert_eq!(view.face_color(index), None);
    }
    assert_eq!(view.position(usize::MAX), None);
    assert_eq!(view.positions().get(usize::MAX), None);
    assert_eq!(view.indices().get(usize::MAX), None);
}

#[test]
fn every_truncation_rejected_without_panicking() {
    let blob = fixture();
    for end in 0..blob.len() {
        assert!(
            decode(&blob[..end]).is_err(),
            "accepted truncation at {end}"
        );
    }
}

#[test]
fn header_only_api_still_accepts_invalid_body() {
    let mut blob = fixture();
    let offset = data_offset(&blob, 3);
    put(&mut blob, offset, 100);
    assert!(validate_mesh_blob(&blob).is_ok());
    assert!(matches!(decode(&blob), Err(Error::VertexIndex)));
}

#[test]
fn resource_limits_and_count_overflow_reject_before_body_access() {
    let blob = fixture();
    let limits = [
        MeshLimits {
            max_blob_bytes: 63,
            ..MeshLimits::default()
        },
        MeshLimits {
            max_sections: 1,
            ..MeshLimits::default()
        },
        MeshLimits {
            max_vertices: 1,
            ..MeshLimits::default()
        },
        MeshLimits {
            max_triangles: 0,
            ..MeshLimits::default()
        },
        MeshLimits {
            max_faces: 0,
            ..MeshLimits::default()
        },
        MeshLimits {
            max_edges: 0,
            ..MeshLimits::default()
        },
        MeshLimits {
            max_edge_points: 1,
            ..MeshLimits::default()
        },
        MeshLimits {
            max_id_bytes: 5,
            ..MeshLimits::default()
        },
    ];
    for limit in limits {
        assert!(matches!(
            MeshView::decode(&blob, limit),
            Err(Error::Limit(_))
        ));
    }
    for (off, ty) in [(8, 1), (12, 3), (16, 4), (20, 7), (24, 8)] {
        let mut blob = blob.clone();
        put(&mut blob, off, u32::MAX);
        let limits = MeshLimits {
            max_vertices: u32::MAX,
            max_triangles: u32::MAX,
            max_faces: u32::MAX,
            max_edges: u32::MAX,
            max_edge_points: u32::MAX,
            ..MeshLimits::default()
        };
        assert!(matches!(MeshView::decode(&blob, limits), Err(Error::CountOverflow(t)) if t == ty));
    }
}

#[test]
fn finite_geometry_normals_edges_and_ordered_bounding_boxes_required() {
    let blob = fixture();
    for ty in [1, 2, 8, 11] {
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut blob = blob.clone();
            let offset = data_offset(&blob, ty);
            put(&mut blob, offset, value.to_bits());
            assert!(matches!(decode(&blob), Err(Error::NonFinite(t)) if t == ty));
        }
    }
    for offset in [32, 44] {
        let mut blob = blob.clone();
        put(&mut blob, offset, f32::NAN.to_bits());
        assert!(matches!(decode(&blob), Err(Error::NonFinite(0))));
    }
    for ty in [0, 11] {
        let mut blob = blob.clone();
        let offset = if ty == 0 { 32 } else { data_offset(&blob, ty) };
        put(&mut blob, offset, 2f32.to_bits());
        assert!(matches!(decode(&blob), Err(Error::BoundingBox(t)) if t == ty));
    }
}

#[test]
fn invalid_indices_and_noncontiguous_or_overflowing_spans_rejected() {
    for index in [3, u32::MAX] {
        let mut blob = fixture();
        let offset = data_offset(&blob, 3);
        put(&mut blob, offset, index);
        assert!(matches!(decode(&blob), Err(Error::VertexIndex)));
    }
    for ty in [4, 7] {
        for pair in [[1, 1], [0, u32::MAX], [0, 0], [u32::MAX, 2]] {
            let mut blob = fixture();
            let offset = data_offset(&blob, ty);
            blob[offset..offset + 8].copy_from_slice(&words(&pair));
            assert!(matches!(decode(&blob), Err(Error::Ranges(t)) if t == ty));
        }
    }
}

#[test]
fn ids_require_zero_start_monotonic_exact_end_and_utf8_boundaries() {
    for ty in [5, 9] {
        for pair in [[1, 3], [0, 2], [0, 4], [2, 1], [0, u32::MAX]] {
            let mut blob = fixture();
            let offset = data_offset(&blob, ty);
            blob[offset..offset + 8].copy_from_slice(&words(&pair));
            assert!(matches!(decode(&blob), Err(Error::IdOffsets(t)) if t == ty));
        }
    }
    for ty in [6, 10] {
        let mut blob = fixture();
        let offset = data_offset(&blob, ty);
        blob[offset] = 0xff;
        assert!(matches!(decode(&blob), Err(Error::IdUtf8(t)) if t == ty));
    }
    let mut blob = fixture();
    // Valid complete UTF-8 is accepted without imposing ID syntax/ASCII rules.
    let offset = data_offset(&blob, 6);
    blob[offset..offset + 3].copy_from_slice("水".as_bytes());
    assert_eq!(decode(&blob).unwrap().face_id(0), Some("水"));
}

#[test]
fn missing_wrong_length_or_unflagged_known_sections_rejected() {
    for (index, ty) in [
        (0, 1),
        (1, 2),
        (2, 3),
        (3, 4),
        (4, 5),
        (5, 6),
        (6, 7),
        (7, 8),
        (8, 9),
        (9, 10),
        (10, 11),
        (11, 12),
    ] {
        let mut blob = fixture();
        put(&mut blob, 64 + index * 16, 99);
        assert!(matches!(decode(&blob), Err(Error::MissingSection(t)) if t == ty));
    }
    for index in [0, 1, 2, 3, 4, 6, 7, 8, 10, 11] {
        let mut blob = fixture();
        let header = validate_mesh_blob(&blob).unwrap();
        let section = header.sections[index];
        blob[section.offset as usize..section.offset as usize + section.byte_len as usize].fill(0);
        put(&mut blob, 64 + index * 16 + 12, 0);
        assert!(matches!(decode(&blob), Err(Error::Length(_))));
    }
    for flag in [1u16, 2, 4, 16] {
        let mut blob = fixture();
        blob[6..8].copy_from_slice(&(0x17 ^ flag).to_le_bytes());
        assert!(matches!(decode(&blob), Err(Error::FlagMismatch(_))));
    }
    let mut blob = fixture();
    blob[28..30].copy_from_slice(&3u16.to_le_bytes());
    assert!(matches!(decode(&blob), Err(Error::UnsupportedLod(3))));
}

#[test]
fn unknown_sections_skipped_only_after_table_and_padding_checks() {
    let unknown = words(&[f32::NAN.to_bits(), u32::MAX]);
    let blob = assemble(
        0,
        &[
            (1, vec![]),
            (3, vec![]),
            (4, vec![]),
            (5, words(&[0])),
            (6, vec![]),
            (999, unknown),
        ],
    );
    let mut blob = blob;
    for off in [8, 12, 16, 20, 24] {
        put(&mut blob, off, 0);
    }
    assert!(decode(&blob).is_ok());
    let mut invalid = blob.clone();
    put(&mut invalid, 64 + 5 * 16 + 8, u32::MAX - 3);
    assert!(matches!(decode(&invalid), Err(Error::Header(_))));
    let mut duplicate = blob;
    put(&mut duplicate, 64 + 4 * 16, 999);
    assert!(matches!(decode(&duplicate), Err(Error::Header(_))));
    let mut padding = fixture();
    let offset = data_offset(&padding, 6) + 3;
    padding[offset] = 1;
    assert!(matches!(decode(&padding), Err(Error::Padding)));
    let mut trailing = fixture();
    trailing.push(1);
    assert!(matches!(decode(&trailing), Err(Error::Padding)));
}

fn validate_fixture_directory(path: &Path) -> usize {
    let mut count = 0;
    for entry in std::fs::read_dir(path).expect("fixture directory must exist") {
        let path = entry.unwrap().path();
        if path.is_dir() {
            count += validate_fixture_directory(&path);
        } else if path.extension().is_some_and(|ext| ext == "mesh1") {
            let blob = std::fs::read(&path).unwrap();
            decode(&blob).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
            count += 1;
        }
    }
    count
}

#[test]
fn worker_produced_repository_fixtures_decode() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spikes/fixtures");
    assert!(
        validate_fixture_directory(&root) >= 2,
        "worker fixtures must not silently disappear"
    );
}

/// Explicit opt-in: missing env/path/no blobs is a failure, never a silent pass.
#[test]
#[ignore = "requires ONECAD_MESH_FIXTURES path to worker-produced mesh1 files"]
fn external_worker_fixture_directory() {
    let path = std::env::var_os("ONECAD_MESH_FIXTURES").expect("ONECAD_MESH_FIXTURES required");
    assert!(
        validate_fixture_directory(Path::new(&path)) > 0,
        "no mesh1 fixtures found"
    );
}

fn multi_span_fixture() -> Vec<u8> {
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
    for (ty, data) in &mut sections {
        *data = match ty {
            3 => words(&[0, 1, 2, 0, 2, 1]),
            4 | 7 => words(&[0, 1, 1, 1]),
            5 | 9 => words(&[0, 2, 4]),
            6 | 10 => "éλ".as_bytes().to_vec(),
            11 | 12 => [data.clone(), data.clone()].concat(),
            _ => continue,
        };
    }
    let mut blob = assemble(0x17, &sections);
    for off in [12, 16, 20] {
        put(&mut blob, off, 2);
    }
    blob
}

#[test]
fn multiple_spans_cover_all_data_and_each_id_has_utf8_boundaries() {
    let blob = multi_span_fixture();
    let view = decode(&blob).unwrap();
    assert_eq!(view.face_range(1).unwrap().first, 1);
    assert_eq!(view.edge_range(1).unwrap().first, 1);
    assert_eq!(view.face_id(1), Some("λ"));
    for ty in [4, 7] {
        for pair in [[0, 1], [2, 0], [1, u32::MAX]] {
            let mut invalid = blob.clone();
            let offset = data_offset(&invalid, ty) + 8;
            invalid[offset..offset + 8].copy_from_slice(&words(&pair));
            assert!(matches!(decode(&invalid), Err(Error::Ranges(t)) if t == ty));
        }
    }
    for ty in [5, 9] {
        let mut invalid = blob.clone();
        let offset = data_offset(&invalid, ty) + 4;
        put(&mut invalid, offset, 1);
        assert!(matches!(decode(&invalid), Err(Error::IdUtf8(_))));
        put(&mut invalid, offset, 4);
        put(&mut invalid, offset + 4, 2);
        assert!(matches!(decode(&invalid), Err(Error::IdOffsets(t)) if t == ty));
    }
}

#[test]
fn arbitrary_byte_mutations_and_random_inputs_never_panic() {
    let source = fixture();
    for index in 0..source.len() {
        for value in [0, 0xff] {
            let mut blob = source.clone();
            blob[index] = value;
            if let Ok(mesh) = decode(&blob) {
                assert_eq!(mesh.position(usize::MAX), None);
                assert_eq!(mesh.triangle(usize::MAX), None);
                assert_eq!(mesh.face_id(usize::MAX), None);
                assert_eq!(mesh.edge_range(usize::MAX), None);
            }
        }
    }
    let mut seed = 0x123456789abcdefu64;
    for len in 0..1024 {
        let blob: Vec<_> = (0..len)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                seed as u8
            })
            .collect();
        assert!(decode(&blob).is_err());
    }
}

#[test]
fn final_id_section_needs_no_optional_trailing_padding() {
    let original = fixture();
    let header = validate_mesh_blob(&original).unwrap();
    let sections: Vec<_> = header
        .sections
        .iter()
        .filter(|s| s.section_type <= 10)
        .map(|s| {
            (
                s.section_type,
                original[s.offset as usize..s.offset as usize + s.byte_len as usize].to_vec(),
            )
        })
        .collect();
    let mut blob = assemble(3, &sections);
    assert!(decode(&blob).is_ok());
    assert_eq!(blob.pop(), Some(0));
    assert_eq!(blob.len() % 4, 3);
    let view = decode(&blob).expect("unpadded final ID bytes remain wire compatible");
    assert_eq!(view.edge_id(0), Some("e:5"));
}
