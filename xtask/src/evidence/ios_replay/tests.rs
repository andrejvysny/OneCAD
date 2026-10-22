use super::*;
use serde_json::json;

fn sample(phase: &str, provenance: &str, time: f64, delta: f64, index: Option<u64>) -> Value {
    let mut sample = json!({"contact":17,"kind":"pencil","phase":phase,
        "provenance":provenance,"t":10.0+time,"x":100.0+delta,"y":200.0+delta});
    if let Some(index) = index {
        sample["estimation_index"] = json!(index);
    }
    sample
}

fn replay(samples: Vec<Value>) -> Value {
    json!({"request":{"cmd":"replay","generation":1,"samples":samples},
        "response":{"ok":true,"synthetic":true,"accepted":samples.len(),"overflowed":false}})
}

fn input(live: u64, prediction: Value, force: Option<f64>, refined: bool, old: u64) -> Value {
    let mut stats = COUNTERS
        .iter()
        .map(|key| ((*key).to_string(), json!(0)))
        .collect::<serde_json::Map<_, _>>();
    stats.insert(
        "duplicateCoalesced".into(),
        json!(u64::from(force.is_some())),
    );
    stats.insert("estimationMatched".into(), json!(u64::from(refined)));
    stats.insert("oldGeneration".into(), json!(old));
    let pose = force.map(|force| {
        json!({"force":force,"maxForce":1.0,"altitude":null,
        "azimuth":null,"roll":null,"hoverZ":null,"estimated":u64::from(!refined),
        "expectsUpdate":u64::from(!refined)})
    });
    json!({"request":{"cmd":"input_state","contact":17,"estimationIndex":102},
        "response":{"ok":true,"synthetic":true,"liveContacts":live,"stats":stats,
            "pose":pose,"prediction":prediction,
            "pencilPosition":force.map(|_| json!([120.0,220.0]))}})
}

fn cancel() -> Value {
    json!({"request":{"abi":"onecad_ios_input_cancel_all","reason":0},"response":{"status":0}})
}

fn document(kernel: &Value) -> Value {
    json!({"request":{"cmd":"document_summary"},"response":{"ok":true,
        "kernelIdentity":kernel,"summary":{"document":{"documentId":"test-doc","revision":2},
            "features":[],"sketches":[],"bodies":[]}}})
}

pub(in crate::evidence) fn valid(run_id: &str, kernel: &Value) -> Value {
    let mut actual = sample("moved", "actual", 0.02, 20.0, Some(102));
    actual["force"] = json!(0.25);
    actual["max_force"] = json!(1.0);
    actual["estimated"] = json!(["force"]);
    actual["expects_update"] = json!(["force"]);
    let mut duplicate = actual.clone();
    duplicate["provenance"] = json!("coalesced");
    let mut update = sample("moved", "estimation_update", 0.07, 200.0, Some(102));
    update["force"] = json!(0.75);
    update["max_force"] = json!(1.0);
    update["estimated"] = json!([]);
    update["expects_update"] = json!([]);
    let live = |delta: f64| {
        input(
            1,
            json!([100.0 + delta, 200.0 + delta]),
            Some(0.25),
            false,
            0,
        )
    };
    let refined = input(1, json!([160.0, 260.0]), Some(0.75), true, 0);
    let scenarios = vec![
        json!({"name":NAMES[0],"observations":[cancel(),document(kernel),input(0,Value::Null,None,false,0),
            replay(vec![actual,sample("moved","coalesced",0.01,10.0,Some(101)),
                sample("began","actual",0.0,0.0,Some(100)),duplicate,
                sample("moved","predicted",0.03,30.0,None)]),live(30.0),document(kernel)]}),
        json!({"name":NAMES[1],"observations":[live(30.0),
            replay(vec![sample("moved","predicted",0.04,40.0,None),sample("moved","predicted",0.05,50.0,None)]),
            live(50.0),replay(vec![sample("moved","predicted",0.06,60.0,None)]),live(60.0),document(kernel)]}),
        json!({"name":NAMES[2],"observations":[live(60.0),replay(vec![update]),refined,document(kernel)]}),
        json!({"name":NAMES[3],"observations":[refined,cancel(),input(0,Value::Null,Some(0.75),true,0),
            replay(vec![sample("ended","actual",0.08,120.0,None)]),
            input(0,Value::Null,Some(0.75),true,1),document(kernel)]}),
    ];
    json!({"schemaVersion":1,"runId":run_id,"synthetic":true,"kernelIdentity":kernel,"scenarios":scenarios})
}

#[test]
fn ios_replay_checks_observed_transitions_instead_of_passed_flags() {
    let kernel = super::super::ios_tests::kernel();
    let simulator = &kernel["embedded"];
    let trace = valid("run", &kernel);
    verify(&trace, "run", simulator).unwrap();
    let changes = [
        ("/schemaVersion", json!(2)),
        ("/runId", json!("stale")),
        ("/synthetic", json!(false)),
        ("/kernelIdentity/backend", json!("sidecar")),
        ("/scenarios/1/name", json!(NAMES[0])),
        ("/scenarios/0/observations", json!([])),
        ("/scenarios/0/observations/3/response/accepted", json!(4)),
        (
            "/scenarios/0/observations/3/response/overflowed",
            json!(true),
        ),
        (
            "/scenarios/0/observations/3/response/synthetic",
            json!(false),
        ),
        (
            "/scenarios/0/observations/3/request/samples/2/t",
            json!(100.0),
        ),
        (
            "/scenarios/0/observations/3/request/samples/3/estimation_index",
            json!(103),
        ),
        (
            "/scenarios/0/observations/4/response/stats/duplicateCoalesced",
            json!(0),
        ),
        ("/scenarios/0/observations/4/response/stats/stale", json!(1)),
        (
            "/scenarios/0/observations/4/response/pencilPosition",
            json!([110.0, 210.0]),
        ),
        (
            "/scenarios/1/observations/4/response/prediction",
            json!([150.0, 250.0]),
        ),
        (
            "/scenarios/1/observations/2/response/pose/force",
            json!(0.5),
        ),
        (
            "/scenarios/1/observations/2/response/stats/invalid",
            json!(1),
        ),
        ("/scenarios/1/observations/3/request/generation", json!(2)),
        (
            "/scenarios/1/observations/3/request/samples/0/t",
            json!(10.01),
        ),
        (
            "/scenarios/2/observations/2/response/pose/force",
            json!(0.25),
        ),
        (
            "/scenarios/2/observations/2/response/pose/estimated",
            json!(1),
        ),
        (
            "/scenarios/2/observations/2/response/stats/estimationMatched",
            json!(0),
        ),
        (
            "/scenarios/2/observations/2/response/pencilPosition",
            json!([300.0, 400.0]),
        ),
        ("/scenarios/3/observations/1/response/status", json!(1)),
        (
            "/scenarios/3/observations/2/response/liveContacts",
            json!(1),
        ),
        (
            "/scenarios/3/observations/4/response/prediction",
            json!([220.0, 320.0]),
        ),
        (
            "/scenarios/3/observations/4/response/stats/oldGeneration",
            json!(0),
        ),
        (
            "/scenarios/3/observations/5/response/summary/document/revision",
            json!(3),
        ),
        (
            "/scenarios/3/observations/5/response/summary/features",
            json!([{"id":"bad"}]),
        ),
        (
            "/scenarios/0/observations/1/response/summary/bodies",
            Value::Null,
        ),
    ];
    for (pointer, replacement) in changes {
        let mut bad = trace.clone();
        *bad.pointer_mut(pointer).unwrap() = replacement;
        bad["passed"] = json!(true);
        assert!(
            verify(&bad, "run", simulator).is_err(),
            "accepted {pointer}"
        );
    }
}

#[test]
fn ios_replay_rejects_missing_duplicate_and_malformed_records() {
    let kernel = super::super::ios_tests::kernel();
    let trace = valid("run", &kernel);
    let mutations: [fn(&mut Value); 6] = [
        |v| {
            v["scenarios"].as_array_mut().unwrap().pop();
        },
        |v| {
            let duplicate = v["scenarios"][0].clone();
            v["scenarios"].as_array_mut().unwrap().push(duplicate);
        },
        |v| {
            v["scenarios"][0]["observations"][2]["response"]
                .as_object_mut()
                .unwrap()
                .remove("stats");
        },
        |v| {
            v["scenarios"][2]["observations"][2]["response"]
                .as_object_mut()
                .unwrap()
                .remove("pose");
        },
        |v| {
            v["scenarios"][0]["observations"][3]["request"]["samples"] = json!([]);
        },
        |v| {
            v["scenarios"].as_array_mut().unwrap().swap(0, 1);
        },
    ];
    for mutate in mutations {
        let mut bad = trace.clone();
        mutate(&mut bad);
        assert!(verify(&bad, "run", &kernel["embedded"]).is_err());
    }
    for value in [Value::Null, json!({}), json!({"passed":true})] {
        assert!(verify(&value, "run", &kernel["embedded"]).is_err());
    }
}

#[test]
#[ignore = "requires explicit ONECAD_IOS_REPLAY_TRACE path to an actual sealed simulator trace"]
fn ios_replay_recorded_simulator_trace_satisfies_transition_checks() {
    let path = std::env::var_os("ONECAD_IOS_REPLAY_TRACE").expect("ONECAD_IOS_REPLAY_TRACE");
    let path = std::path::PathBuf::from(path);
    let provenance: Value = serde_json::from_slice(
        &std::fs::read(path.with_file_name(format!("{ARTIFACT}.provenance.json"))).unwrap(),
    )
    .unwrap();
    let run = provenance["runId"].as_str().unwrap();
    let simulator = &provenance["kernelIdentity"]["embedded"];
    ios_verify::verify_artifacts(path.parent().unwrap(), run, simulator, &[]).unwrap();
    let bytes = std::fs::read(path).unwrap();
    let trace: Value = serde_json::from_slice(&bytes).unwrap();
    verify(&trace, run, simulator).unwrap();
}
