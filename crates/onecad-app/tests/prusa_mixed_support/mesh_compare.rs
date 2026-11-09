use onecad_protocol::mesh::{MeshLimits, MeshView};
use sha2::{Digest, Sha256};

pub fn equal(label: &str, expected: &[u8], actual: &[u8]) {
    assert!(
        expected == actual,
        "{label}: mesh bytes differ: {}",
        describe(expected, actual)
    );
}

pub fn different(label: &str, left: &[u8], right: &[u8]) {
    assert!(
        left != right,
        "{label}: mesh bytes unexpectedly equal: {}",
        describe(left, right)
    );
}

fn describe(left: &[u8], right: &[u8]) -> String {
    let first = left
        .iter()
        .zip(right)
        .position(|(a, b)| a != b)
        .or_else(|| (left.len() != right.len()).then_some(left.len().min(right.len())));
    let prefix = format!(
        "left len={} sha256={:x}; right len={} sha256={:x}; first differing offset={first:?}",
        left.len(),
        Sha256::digest(left),
        right.len(),
        Sha256::digest(right)
    );
    match (
        MeshView::decode(left, MeshLimits::default()),
        MeshView::decode(right, MeshLimits::default()),
    ) {
        (Ok(a), Ok(b)) => format!("{prefix}; {}", mesh_changes(&a, &b)),
        (a, b) => format!(
            "{prefix}; checked decode left={:?}, right={:?}",
            a.err(),
            b.err()
        ),
    }
}

fn mesh_changes(left: &MeshView<'_>, right: &MeshView<'_>) -> String {
    let a = left.header();
    let b = right.header();
    let mut changed = Vec::new();
    for section in &a.sections {
        let other = b.section(section.section_type);
        let same = other.is_some_and(|entry| {
            let left_start = section.offset as usize;
            let right_start = entry.offset as usize;
            left.bytes()
                .get(left_start..left_start + section.byte_len as usize)
                == right
                    .bytes()
                    .get(right_start..right_start + entry.byte_len as usize)
        });
        if !same {
            changed.push(section.section_type);
        }
    }
    for section in &b.sections {
        if a.section(section.section_type).is_none() {
            changed.push(section.section_type);
        }
    }
    format!(
        "headers_equal={}; left header={a:?}; right header={b:?}; differing section types={changed:?}; face IDs {}; edge IDs {}",
        a == b,
        id_changes(left, right, true),
        id_changes(left, right, false)
    )
}

fn id_changes(left: &MeshView<'_>, right: &MeshView<'_>, faces: bool) -> String {
    let (a, b) = if faces {
        (left.header().face_count, right.header().face_count)
    } else {
        (left.header().edge_count, right.header().edge_count)
    };
    let mut count = 0;
    let mut examples = Vec::new();
    for i in 0..a.max(b) as usize {
        let (x, y) = if faces {
            (left.face_id(i), right.face_id(i))
        } else {
            (left.edge_id(i), right.edge_id(i))
        };
        if x != y {
            count += 1;
            if examples.len() < 3 {
                examples.push((i, x, y));
            }
        }
    }
    format!("changed={count}, examples={examples:?}")
}
