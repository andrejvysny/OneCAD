//! Retained Framework refusal timings; a completed measurement is not acceptance.
use super::*;
#[path = "../public_mixed_stress_support/measurement.rs"]
mod measurement;
use measurement::{rss, Campaign};
use onecad_core::regen::GeometryEngine;
use serde_json::json;
use std::time::Instant;

pub(super) async fn run() {
    measurement::require_release();
    measurement::optional_tracing();
    let area = tempfile::tempdir().unwrap();
    let path = area.path().join("retained-inputs.onecad");
    fixture(&path);
    let mut campaign = Campaign::begin(
        "framework-retained-cut",
        &path,
        json!({
        "warmClassification":"same-application-worker-runtime-repeated-cut-and-undo",
        "expectedTerminal":"PUBLICATION_OPEN_MANIFOLD refusal, not successful geometry publication",
        "sourceOperands":DATA.iter().map(|(name,sha,_)|json!({"name":name,"sha256":sha})).collect::<Vec<_>>(),
        "boundsUnchanged":{"terminalTimeoutSeconds":30,"publicationResidualMm":0.001},
        "availablePhases":["fresh worker/runtime open","apply plus terminal wall time","checked fine mesh validation","undo cleanup"],
        "unavailablePhases":["internal replay/solve split","preview admission","GPU","presentation"]}),
    );
    let path = campaign.root.join("fixture.onecad");
    for batch in 0..campaign.batches {
        let h = measured_open(
            &mut campaign,
            &path,
            &area.path().join(format!("warm-{batch}")),
            batch,
            "bootstrap",
            0,
        )
        .await;
        let original = body_meshes(&h, &h.projection().await).await;
        for sample in 0..campaign.warmups + campaign.warm {
            let phase = if sample < campaign.warmups {
                "warmup"
            } else {
                "warm"
            };
            sample_once(&mut campaign, &h, &original, batch, phase, sample).await;
        }
        h.close().await;
        for sample in 0..campaign.cold {
            let h = measured_open(
                &mut campaign,
                &path,
                &area.path().join(format!("cold-{batch}-{sample}")),
                batch,
                "cold",
                sample,
            )
            .await;
            let original = body_meshes(&h, &h.projection().await).await;
            sample_once(&mut campaign, &h, &original, batch, "cold", sample).await;
            h.close().await;
        }
    }
    campaign.finish();
}

async fn measured_open(
    campaign: &mut Campaign,
    path: &Path,
    root: &Path,
    batch: usize,
    phase: &str,
    sample: usize,
) -> Harness {
    campaign.record(json!({"event":"started","batch":batch,"phase":phase,"sample":sample,"case":"fresh-worker-runtime-open","rssBefore":rss()}));
    let started = Instant::now();
    let h = Harness::new(root).await;
    let projection = open(&h, path).await;
    campaign.record(json!({"event":"result","batch":batch,"phase":phase,"sample":sample,"case":"fresh-worker-runtime-open",
        "wallMs":started.elapsed().as_secs_f64()*1000.0,"outcome":"published","runtimeSession":projection.runtime_session,
        "workerEpochAfter":h.state.live_worker().unwrap().current_epoch().0,"rssAfter":rss()}));
    h
}

async fn sample_once(
    campaign: &mut Campaign,
    h: &Harness,
    original: &BTreeMap<String, String>,
    batch: usize,
    phase: &str,
    sample: usize,
) {
    let projection = h.projection().await;
    let epoch = h.state.live_worker().unwrap().current_epoch().0;
    let mut base = json!({"batch":batch,"phase":phase,"sample":sample,"case":"retained-original-cut","runtimeSession":projection.runtime_session,"workerEpochBefore":epoch});
    base["event"] = json!("started");
    base["rssBefore"] = rss();
    base["workerHello"] = json!(format!("{:?}", h.state.live_worker().unwrap().hello()));
    campaign.record(base.clone());
    let started = Instant::now();
    let result = boolean(
        h,
        10,
        BooleanOp::Cut,
        imported(&projection, 0),
        imported(&projection, 2),
    )
    .await;
    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
    let expected_refusal = result.failed_steps.iter().any(|step| {
        step.record_id == rid(10).to_string()
            && format!("{step:?}").contains("PUBLICATION_OPEN_MANIFOLD")
    });
    base["event"] = json!("result");
    base["wallMs"] = json!(elapsed);
    base["terminal"] = json!(result);
    base["outcome"] = json!(if expected_refusal {
        "expected-refusal"
    } else {
        "unexpected-terminal"
    });
    base["failedSteps"] = json!(result.failed_steps);
    base["workerEpochAfter"] = json!(h.state.live_worker().unwrap().current_epoch().0);
    base["rssAfter"] = rss();
    campaign.record(base.clone());
    assert!(expected_refusal, "{result:?}");
    let started = Instant::now();
    let actual = body_meshes(h, &h.projection().await).await;
    base["case"] = json!("checked-fine-mesh-validation");
    base["wallMs"] = json!(started.elapsed().as_secs_f64() * 1000.0);
    base["outcome"] = json!(if &actual == original {
        "unchanged-accepted-geometry"
    } else {
        "failed"
    });
    base["terminal"] = json!({"meshHashes":actual});
    base["failedSteps"] = json!([]);
    base["rssAfter"] = rss();
    campaign.record(base.clone());
    assert_eq!(
        &actual, original,
        "refused retained Cut must preserve accepted mesh bytes"
    );
    let started = Instant::now();
    let undo = api::undo(&h.state, h.host.clone()).await.unwrap();
    let undo_terminal = h.terminal(&undo).await;
    base["case"] = json!("undo-cleanup");
    base["wallMs"] = json!(started.elapsed().as_secs_f64() * 1000.0);
    base["outcome"] = json!("published-original-inputs");
    base["terminal"] = json!(undo_terminal);
    base["rssAfter"] = rss();
    campaign.record(base);
    assert!(matches!(
        undo_terminal.outcome.as_str(),
        "published" | "noop"
    ));
    assert!(undo_terminal.failed_steps.is_empty() && undo_terminal.repair_steps.is_empty());
}
