//! Same application/runtime measurements; production uses cold auxiliary replay workers.
use super::*;
#[path = "measurement.rs"]
mod measurement;
use measurement::{rss, Campaign};
use onecad_core::regen::GeometryEngine;
use serde_json::{json, Value};
use std::time::Instant;

pub(super) async fn run() {
    measurement::require_release();
    measurement::optional_tracing();
    let area = tempfile::tempdir().unwrap();
    let path = match std::env::var_os("ONECAD_PERF_FIXTURE") {
        Some(path) => std::path::PathBuf::from(path),
        None => build_input(area.path()).await,
    };
    let loaded = ContainerReader::open(&path).unwrap();
    assert_eq!(
        loaded.document().definitions.len(),
        10,
        "initial fixture only"
    );
    assert_eq!(loaded.document().assembly.occurrences.len(), 20);
    let mut campaign = Campaign::begin(
        "public-mixed-10x20",
        &path,
        json!({
        "warmClassification":"warm_application_cold_auxiliary_replay",
        "workerReuse":"main application worker and runtime retained; each production definition replay starts an auxiliary worker",
        "mutation":"alternate 1 mm and 0 mm translation of native lever in definition 6",
        "replay":"all ten source definitions on every sample; per-definition wall time includes worker startup and verification",
        "notApplicable":["joint solving","bound-origin resolution"],
        "availablePhases":["assembly edit","per-definition replay","checked CPU mesh capture"]}),
    );
    let path = campaign.root.join("fixture.onecad");
    for batch in 0..campaign.batches {
        let (h, session) = super::open(&path, &area.path().join(format!("warm-{batch}"))).await;
        super::replay(&h.state, &session, &(0..10).collect::<Vec<_>>()).await;
        super::verify_scene(&h.state, &session).await;
        for sample in 0..campaign.warmups + campaign.warm {
            let phase = if sample < campaign.warmups {
                "warmup"
            } else {
                "warm"
            };
            sample_once(
                &mut campaign,
                &h,
                &session,
                batch,
                phase,
                sample,
                sample == 0,
            )
            .await;
        }
        h.close().await;
        for sample in 0..campaign.cold {
            let started = Instant::now();
            let (h, session) =
                super::open(&path, &area.path().join(format!("cold-{batch}-{sample}"))).await;
            campaign.record(
                json!({"event":"result","batch":batch,"phase":"cold","sample":sample,
                "case":"fresh-worker-runtime-open","wallMs":started.elapsed().as_secs_f64()*1000.0,
                "runtimeSession":session,"outcome":"opened","rssAfter":rss()}),
            );
            sample_once(&mut campaign, &h, &session, batch, "cold", sample, true).await;
            h.close().await;
        }
    }
    campaign.finish();
}

async fn build_input(area: &Path) -> std::path::PathBuf {
    let cover = area.join("cover.onecad");
    let lever = area.join("lever.onecad");
    let plate = area.join("plate.onecad");
    let constrained = area.join("constrained.onecad");
    super::cover::build(&cover).await;
    super::lever::build(&lever).await;
    super::native::build_hole_plate(&plate, &area.join("plate-host")).await;
    super::native::build_constrained(&constrained, &area.join("constrained-host")).await;
    let path = area.join("mixed.onecad");
    fixture::write(&path, &cover, &lever, &plate, &constrained);
    path
}

async fn sample_once(
    campaign: &mut Campaign,
    h: &Harness,
    session: &str,
    batch: usize,
    phase: &str,
    sample: usize,
    first: bool,
) {
    let epoch = h.state.live_worker().unwrap().current_epoch().0;
    let base = json!({"batch":batch,"phase":phase,"sample":sample,"runtimeSession":session,"workerEpochBefore":epoch});
    let mut start = base.clone();
    start["event"] = json!("started");
    start["rssBefore"] = rss();
    start["workerHello"] = json!(format!("{:?}", h.state.live_worker().unwrap().hello()));
    campaign.record(start);
    let sample_started = Instant::now();
    let started = Instant::now();
    let distance = if sample.is_multiple_of(2) { 1.0 } else { 0.0 };
    let result = measured_edit(h, session, first, distance).await;
    record_result(campaign, &base, "assembly-edit", started, result);
    for definition in 0..10 {
        let started = Instant::now();
        let result = api::regenerate_component(&h.state, session, fixture::def(definition)).await;
        let result=result.map(|(generation,projection)|json!({"generation":generation,"persistedGeneration":projection.persisted_generation})).map_err(|error|error.to_string());
        record_result(
            campaign,
            &base,
            &format!("definition-{definition}-replay"),
            started,
            result,
        );
    }
    let started = Instant::now();
    let result = capture(h, session, distance).await;
    record_result(campaign, &base, "checked-cpu-mesh-capture", started, result);
    let after = h.state.live_worker().unwrap().current_epoch().0;
    let mut completed = base;
    completed["event"] = json!("result");
    completed["case"] = json!("edit-and-ten-definition-replay");
    completed["wallMs"] = json!(sample_started.elapsed().as_secs_f64() * 1000.0);
    completed["workerEpochAfter"] = json!(after);
    completed["outcome"] = json!(if after == epoch {
        "completed"
    } else {
        "worker-restarted"
    });
    completed["rssAfter"] = rss();
    campaign.record(completed);
    assert_eq!(
        after, epoch,
        "main application worker changed within a sample"
    );
}

async fn measured_edit(
    h: &Harness,
    session: &str,
    first: bool,
    distance: f64,
) -> Result<Value, String> {
    let projection = api::get_assembly_projection(&h.state, session)
        .await
        .map_err(|e| e.to_string())?;
    let part = projection
        .definitions
        .iter()
        .find(|d| d.id == fixture::def(6).to_string())
        .ok_or("native lever missing")?;
    let op = Operation::Known(KnownOperation::TransformBody(TransformBodyParams {
        targets: part
            .bodies
            .iter()
            .map(|body| body.id.parse().unwrap())
            .collect(),
        translate: [Scalar::new(distance), Scalar::new(0.0), Scalar::new(0.0)],
        rotate: Default::default(),
        copy: false,
        extra: Default::default(),
    }));
    // Cold capture has not replayed yet, so use the pinned source producer body.
    let op = if part.bodies.is_empty() {
        return Err("native lever has no persisted body authority".into());
    } else {
        op
    };
    let record = RecordId(Uuid::from_u128(0xD6F0));
    let command = if first {
        EditCommand::AddOperation {
            record: OperationRecord::new(record, 0, "Measured lever translation", op),
            at_cursor: true,
        }
    } else {
        EditCommand::UpdateOperationParams { record, op }
    };
    let result = api::apply_assembly_edit_at_generation(
        &h.state,
        session,
        projection.persisted_generation,
        AssemblyEdit::EditDefinition {
            id: fixture::def(6),
            command: Box::new(command),
        },
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(json!({"persistedGeneration":result.persisted_generation,"translationMm":distance}))
}

async fn capture(h: &Harness, session: &str, distance: f64) -> Result<Value, String> {
    let mut bytes = 0usize;
    let mut triangles = 0usize;
    for index in 0..10 {
        for body in super::meshes(&h.state, session, index, index * 2).await {
            let mesh = MeshView::decode(&body, MeshLimits::default()).map_err(|e| e.to_string())?;
            if mesh.header().triangle_count == 0 {
                return Err(format!("definition {index} published an empty mesh"));
            }
            if index == 6 {
                check_translation(&mesh, distance)?;
            }
            bytes += body.len();
            triangles += mesh.header().triangle_count as usize;
        }
    }
    Ok(
        json!({"uniqueMeshPayloadBytes":bytes,"trianglesPerDefinition":triangles,"excludes":"scene metadata, BVH, GPU upload and presentation"}),
    )
}

fn check_translation(mesh: &MeshView<'_>, distance: f64) -> Result<(), String> {
    let oracle: Value = serde_json::from_slice(include_bytes!(
        "../../../../fixtures/reference-oracles/prusa-sensor.json"
    ))
    .unwrap();
    for (side, actual) in [mesh.header().bbox_min[0], mesh.header().bbox_max[0]]
        .into_iter()
        .enumerate()
    {
        let expected = oracle["parts"]["lever"]["boundsMm"][side][0]
            .as_f64()
            .unwrap()
            + distance;
        if (f64::from(actual) - expected).abs() > 0.15 {
            return Err(format!("measured lever translation {distance} mm did not match existing source-bounds oracle"));
        }
    }
    Ok(())
}

fn record_result(
    campaign: &mut Campaign,
    base: &Value,
    case: &str,
    start: Instant,
    result: Result<Value, String>,
) {
    let mut row = base.clone();
    row["event"] = json!("result");
    row["case"] = json!(case);
    row["wallMs"] = json!(start.elapsed().as_secs_f64() * 1000.0);
    row["rssAfter"] = rss();
    row["outcome"] = json!(if result.is_ok() {
        "completed"
    } else {
        "failed"
    });
    row["terminal"] = json!(result.as_ref().ok());
    row["failure"] = json!(result.as_ref().err());
    campaign.record(row);
    result.unwrap_or_else(|error| panic!("measurement failed: {error}"));
}
