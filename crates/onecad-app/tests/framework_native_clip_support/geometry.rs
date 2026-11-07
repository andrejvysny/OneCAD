//! Analytic source-plane intersections; all dimensions remain in millimetres.
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy)]
pub struct Plane {
    pub origin: [f64; 3],
    pub normal: [f64; 3],
    pub offset: f64,
}

pub fn source_planes() -> std::collections::BTreeMap<u64, Plane> {
    let step = include_bytes!(
        "../../../../fixtures/public-cad/framework/ExpansionCard_ThreadedInsert.stp"
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(step)),
        "6128186260b22ace271e83f096e2c16707b60866b3aa1b3a3f2ea0e38c3c7c09"
    );
    let source: Value = serde_json::from_slice(include_bytes!(
        "../../../../fixtures/reference-histories/framework-enclosure-profiles.json"
    ))
    .unwrap();
    assert_eq!(
        source["sourceSha256"],
        "6128186260b22ace271e83f096e2c16707b60866b3aa1b3a3f2ea0e38c3c7c09"
    );
    source["clipCutPlanes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            let arr = |key: &str| {
                let a = v[key].as_array().unwrap();
                [
                    a[0].as_f64().unwrap(),
                    -a[2].as_f64().unwrap(),
                    a[1].as_f64().unwrap(),
                ]
            };
            let origin = arr("origin");
            let normal = arr("normal");
            (
                v["faceIndex"].as_u64().unwrap(),
                Plane {
                    origin,
                    normal,
                    offset: dot(origin, normal),
                },
            )
        })
        .collect()
}

pub fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}
pub fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub fn unit(a: [f64; 3]) -> [f64; 3] {
    let n = dot(a, a).sqrt();
    assert!(n > 0.0);
    a.map(|v| v / n)
}

pub fn triple_intersection(a: Plane, b: Plane, c: Plane) -> [f64; 3] {
    let bc = cross(b.normal, c.normal);
    let ca = cross(c.normal, a.normal);
    let ab = cross(a.normal, b.normal);
    let determinant = dot(a.normal, bc);
    assert!(determinant.abs() > f64::EPSILON);
    std::array::from_fn(|i| (a.offset * bc[i] + b.offset * ca[i] + c.offset * ab[i]) / determinant)
}

fn y(p: Plane, x: f64) -> f64 {
    assert_ne!(p.normal[1], 0.0);
    (p.offset - p.normal[0] * x) / p.normal[1]
}
fn z(p: Plane, x: f64, y: f64) -> f64 {
    assert_ne!(p.normal[2], 0.0);
    (p.offset - p.normal[0] * x - p.normal[1] * y) / p.normal[2]
}

pub fn sections(
    planes: &std::collections::BTreeMap<u64, Plane>,
    x: f64,
    upper: bool,
    end_plane: u64,
) -> [[f64; 2]; 4] {
    let left = y(planes[&end_plane], x);
    let right = y(planes[&86], x);
    let (low, high) = if upper { (81, 90) } else { (84, 81) };
    [
        [left, z(planes[&low], x, left)],
        [right, z(planes[&low], x, right)],
        [right, z(planes[&high], x, right)],
        [left, z(planes[&high], x, left)],
    ]
}
