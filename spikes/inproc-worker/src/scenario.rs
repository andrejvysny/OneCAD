//! The request shapes both hosts run. Copied from the canonical cross-track
//! fixture protocol/fixtures/execute_plan_ordinary.ndjson (ops op0 + op1 of it):
//! OpenSession → ExecutePlan(Sketch rectangle 40×20, Extrude 25 NewBody) →
//! AcceptPrepared → Tessellate(body_op1).
use std::time::{Duration, Instant};

use onecad_protocol::client::ProtocolClient;
use onecad_protocol::messages::Lane;
use serde_json::{json, Value};

pub const EMPTY_HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
pub const EPOCH: u64 = 3;

pub fn sketch_op(op_id: &str) -> Value {
    json!({"opType":"Sketch","opId":op_id,"stepIndex":0,"params":{"sketchId":"sk1","plane":{"kind":"XY"},
      "entities":[{"id":"e1","type":"Line","p0":[0,0],"p1":[40,0]},{"id":"e2","type":"Line","p0":[40,0],"p1":[40,20]},
                  {"id":"e3","type":"Line","p0":[40,20],"p1":[0,20]},{"id":"e4","type":"Line","p0":[0,20],"p1":[0,0]}],
      "constraints":[]}})
}

pub fn extrude_op(op_id: &str, distance: f64) -> Value {
    json!({"opType":"Extrude","opId":op_id,"stepIndex":1,
      "inputs":[{"primary":{"bodyId":"","elementId":"sk1.region.r0","kind":"face"}}],
      "params":{"sketchId":"sk1","distance":distance,"extrudeMode":"Blind","booleanMode":"NewBody"}})
}

pub fn plan_args(job_id: u64, ops: Vec<Value>, prefix_hashes: Vec<String>) -> Value {
    let target = ops.len() as u64 - 1;
    json!({"jobId":job_id,"documentRevision":1,"workerEpoch":EPOCH,"expectedBaseHash":EMPTY_HASH,
           "prefixHashes":prefix_hashes,"targetStep":target,"ops":ops})
}

pub fn fixture_prefixes() -> Vec<String> {
    vec!["a".repeat(64), "b".repeat(64)]
}

#[derive(Debug, Clone, Default)]
pub struct OpTimings {
    pub open: Duration,
    pub plan: Duration,
    pub accept: Duration,
    pub tessellate: Duration,
    pub mesh_sha: String,
    pub triangles: u64,
    pub mesh_bytes: usize,
}

pub fn expect_ok(verb: &str, resp: &onecad_protocol::messages::RespFrame) -> Result<(), String> {
    if resp.ok {
        Ok(())
    } else {
        Err(format!("{verb} failed: {:?}", resp.error))
    }
}

pub async fn open_session(c: &ProtocolClient, doc: &str) -> Result<(), String> {
    let r = c
        .request(
            "OpenSession",
            json!({"documentId":doc,"documentRevision":0,"workerEpoch":EPOCH,"mode":"determinism"}),
        )
        .await
        .map_err(|e| e.to_string())?;
    expect_ok("OpenSession", &r)
}

/// One full model round: fresh session, sketch→extrude plan, accept, mesh.
pub async fn model_round(c: &ProtocolClient, job_id: u64) -> Result<OpTimings, String> {
    let mut t = OpTimings::default();
    let s = Instant::now();
    open_session(c, "doc_spike").await?;
    t.open = s.elapsed();

    let s = Instant::now();
    let ops = vec![sketch_op("op0"), extrude_op("op1", 25.0)];
    let r = c
        .request("ExecutePlan", plan_args(job_id, ops, fixture_prefixes()))
        .await
        .map_err(|e| e.to_string())?;
    expect_ok("ExecutePlan", &r)?;
    let result = r.result.unwrap_or_default();
    if result["stoppedReason"] != "completed" {
        return Err(format!("ExecutePlan not completed: {result}"));
    }
    t.plan = s.elapsed();

    let s = Instant::now();
    let r = c
        .request(
            "AcceptPrepared",
            json!({"jobId":job_id,"documentRevision":1,"workerEpoch":EPOCH}),
        )
        .await
        .map_err(|e| e.to_string())?;
    expect_ok("AcceptPrepared", &r)?;
    t.accept = s.elapsed();

    let s = Instant::now();
    let inflight = c
        .start_request(
            "Tessellate",
            json!({"bodyIds":["body_op1"],"lod":"coarse","includeEdges":true}),
            Lane::Control,
        )
        .await
        .map_err(|e| e.to_string())?;
    let (r, tail) = inflight
        .response_with_bin()
        .await
        .map_err(|e| e.to_string())?;
    expect_ok("Tessellate", &r)?;
    t.tessellate = s.elapsed();
    let mesh = &r.result.unwrap_or_default()["meshes"][0];
    t.mesh_sha = mesh["sha256"].as_str().unwrap_or("").to_string();
    t.triangles = mesh["triangleCount"].as_u64().unwrap_or(0);
    t.mesh_bytes = tail.len();
    if t.mesh_sha.is_empty() || t.triangles == 0 {
        return Err(format!("Tessellate returned no mesh: {mesh}"));
    }
    Ok(t)
}

/// `n` GetWorkerHead round trips (status lane; pure transport + scheduling cost).
pub async fn ping_rtts(c: &ProtocolClient, n: usize) -> Result<Vec<Duration>, String> {
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let s = Instant::now();
        let r = c
            .request("GetWorkerHead", json!({}))
            .await
            .map_err(|e| e.to_string())?;
        expect_ok("GetWorkerHead", &r)?;
        out.push(s.elapsed());
    }
    Ok(out)
}

pub fn median_ms(mut v: Vec<Duration>) -> f64 {
    v.sort();
    v[v.len() / 2].as_secs_f64() * 1e3
}

pub fn pct_ms(mut v: Vec<Duration>, p: f64) -> f64 {
    v.sort();
    let i = ((v.len() as f64 - 1.0) * p).round() as usize;
    v[i].as_secs_f64() * 1e3
}
