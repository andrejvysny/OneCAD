use super::*;

pub(super) async fn validate_cover(h: &Harness) {
    let source: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../fixtures/reference-oracles/prusa-sensor.json"
    ))
    .unwrap();
    let stl: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../fixtures/reference-oracles/prusa-cover-stl.json"
    ))
    .unwrap();
    let stl_bytes = include_bytes!("../../../../fixtures/public-cad/prusa/MINI-fsenzor-cover.stl");
    assert_eq!(
        format!("{:x}", Sha256::digest(stl_bytes)),
        stl["sourceSha256"].as_str().unwrap()
    );
    let mass = api::query_mass_properties(&h.state, body_id_wire(BodyId(id(3))), None)
        .await
        .unwrap()
        .volume;
    let same_kernel = source["parts"]["cover"]["volumeMm3"].as_f64().unwrap();
    let independent = stl["observation"]["signedVolumeMm3"].as_f64().unwrap();
    let volume_trial = stl["comparisonLimits"]["relativeVolumeTrial"]
        .as_f64()
        .unwrap();
    assert!(
        (mass - independent).abs() <= independent * volume_trial,
        "BRep volume {mass} vs STL trial {independent}"
    );
    eprintln!(
        "native BRep {mass} mm³; same-kernel STEP {same_kernel}; independent STL {independent}; trial {volume_trial}"
    );
    let bytes = api::get_mesh(
        &h.state,
        BodyId(id(3)).to_string(),
        "fine".into(),
        None,
        None,
    )
    .await
    .unwrap();
    let mesh = MeshView::decode(&bytes, MeshLimits::default()).unwrap();
    assert!(mesh.header().triangle_count > 0);
    mesh_topology::assert_upstream_topology(&mesh, &stl);
    let (low, high, mesh_volume) = mesh_measurements(&mesh);
    check_mesh_trials(&source["parts"]["cover"], &stl, low, high, mesh_volume);
}

fn mesh_measurements(mesh: &MeshView<'_>) -> ([f64; 3], [f64; 3], f64) {
    let mut low = [f64::INFINITY; 3];
    let mut high = [f64::NEG_INFINITY; 3];
    let mut six_volume = 0.0;
    for tri in 0..mesh.header().triangle_count as usize {
        let points = mesh
            .triangle(tri)
            .unwrap()
            .map(|index| mesh.position(index as usize).unwrap().map(f64::from));
        for point in points {
            for i in 0..3 {
                low[i] = low[i].min(point[i]);
                high[i] = high[i].max(point[i]);
            }
        }
        let [a, b, c] = points;
        six_volume += a[0] * (b[1] * c[2] - b[2] * c[1])
            + a[1] * (b[2] * c[0] - b[0] * c[2])
            + a[2] * (b[0] * c[1] - b[1] * c[0]);
    }
    (low, high, six_volume.abs() / 6.0)
}

fn check_mesh_trials(
    source: &serde_json::Value,
    stl: &serde_json::Value,
    low: [f64; 3],
    high: [f64; 3],
    volume: f64,
) {
    let tolerance = stl["comparisonLimits"]["boundsMmTrial"].as_f64().unwrap();
    for i in 0..3 {
        assert!((low[i] - source["boundsMm"][0][i].as_f64().unwrap()).abs() <= tolerance);
        assert!((high[i] - source["boundsMm"][1][i].as_f64().unwrap()).abs() <= tolerance);
    }
    let expected = stl["observation"]["signedVolumeMm3"].as_f64().unwrap();
    let relative = stl["comparisonLimits"]["relativeVolumeTrial"]
        .as_f64()
        .unwrap();
    assert!(
        (volume - expected).abs() <= expected * relative,
        "native mesh {volume} vs STL {expected}"
    );
    let mut measured_spans = std::array::from_fn::<_, 3, _>(|i| high[i] - low[i]);
    let mut stl_spans = std::array::from_fn::<_, 3, _>(|i| {
        stl["observation"]["boundsMm"][1][i].as_f64().unwrap()
            - stl["observation"]["boundsMm"][0][i].as_f64().unwrap()
    });
    measured_spans.sort_by(f64::total_cmp);
    stl_spans.sort_by(f64::total_cmp);
    for i in 0..3 {
        assert!((measured_spans[i] - stl_spans[i]).abs() <= tolerance);
    }
    eprintln!(
        "native mesh bounds {low:?} to {high:?}, volume {volume}; sorted STL span trial {stl_spans:?}"
    );
}

pub(super) async fn save_native_capture(h: &Harness, root: &std::path::Path) -> std::path::PathBuf {
    let path = root.join("native-cover.onecad");
    let result = api::save_document(
        &h.state,
        h.host.clone(),
        Some(path.to_string_lossy().into_owned()),
        None,
    )
    .await
    .unwrap();
    assert!(result.clean);
    let saved = ContainerReader::open(&path).unwrap();
    assert!(saved.import_blobs().is_empty() && saved.library_archives().is_empty());
    assert_eq!(saved.document().timeline.records().len(), 32);
    assert!(saved
        .document()
        .timeline
        .records()
        .iter()
        .all(|record| matches!(
            record.op,
            Operation::Known(
                KnownOperation::Sketch(_) | KnownOperation::Extrude(_) | KnownOperation::Revolve(_)
            )
        )));
    if let Some(destination) = std::env::var_os("ONECAD_COVER_CAPTURE") {
        use std::io::Write;
        let bytes = std::fs::read(&path).unwrap();
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .unwrap();
        output.write_all(&bytes).unwrap();
        output.sync_all().unwrap();
        eprintln!(
            "native cover capture {} SHA-256 {:x}",
            std::path::Path::new(&destination).display(),
            Sha256::digest(&bytes)
        );
    }
    path
}
