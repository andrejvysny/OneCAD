//! The crash journal with the real kernel: a failing journal never blocks an
//! op and is surfaced; a previous launch's leftover op record strikes only its
//! own key, and the persisted circuit fails that plan fast until cleared.

use std::fs;

use onecad_core::ids::WorkerEpoch;
use onecad_core::regen::PlanRequest;

use super::failure::{drive, smoke_plan, PlanEnd};
use super::*;
use crate::worker::WorkerLifecycle;

/// The crash-circuit key of a one-op plan (`plan_op_keys` format).
fn key_of(plan: &PlanRequest, fingerprint: &str) -> String {
    format!(
        "{}|{}|{fingerprint}",
        plan.prefix_hashes[0], plan.ops[0].record_id
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failing_journal_is_counted_surfaced_and_never_blocks_an_op() {
    let _gate = GATE.lock().await;
    if !kernel_linked() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("not-a-directory");
    fs::write(&blocker, b"x").unwrap();
    let wm = spawn_ready(
        &embedded(Some(&blocker.join("kernel-journal"))),
        fast_config(),
    )
    .await;
    let mut lifecycle = wm.subscribe();
    let outcome = plate::plate_round(&wm).await;
    plate::assert_plate(&outcome);
    assert!(
        wm.journal_write_failures() > 0,
        "journal writes failed and were counted"
    );
    let surfaced = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(WorkerLifecycle::JournalDegraded { failures, .. }) = lifecycle.recv().await {
                return failures;
            }
        }
    })
    .await
    .expect("a JournalDegraded status event");
    assert!(surfaced > 0);
    assert_eq!(wm.state(), WorkerState::Ready, "the worker kept serving");
    retire(&wm).await;
}

/// An `inflight.json` as an EARLIER launch left it, plus one persisted strike.
fn leave_previous_launch(dir: &Path, crashed_key: &str, fingerprint: &str) {
    let op = |key: Option<&str>, fp: &str| {
        json!({ "instance": 1, "epoch": 1, "requestId": 9, "verb": "ExecutePlan",
                "opId": key.map(|k| k.split('|').nth(1).unwrap()), "stepIndex": 0,
                "opPosition": 0, "key": key, "fingerprint": fp, "startedMs": 0 })
    };
    let jobs = [
        op(Some(crashed_key), fingerprint),
        op(None, fingerprint), // a non-plan job: poisons nothing
        op(
            Some("ff|other-build-op|0000000000000000"),
            "0000000000000000",
        ),
    ];
    fs::create_dir_all(dir).unwrap();
    let inflight = json!({ "version": 1, "session": "an-earlier-launch", "jobs": jobs });
    fs::write(dir.join("inflight.json"), inflight.to_string()).unwrap();
    let op_id = crashed_key.split('|').nth(1).unwrap();
    let poison = json!({ "version": 1, "entries": { crashed_key:
        { "strikes": 1, "opId": op_id, "lastMs": 0 } } });
    fs::write(dir.join("poison.json"), poison.to_string()).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stale_journal_strikes_only_the_crashed_op_and_the_circuit_clears() {
    let _gate = GATE.lock().await;
    if !kernel_linked() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let journal = dir.path().join("kernel-journal");
    let fingerprint = fingerprint();
    let crashed = smoke_plan(0xc0de, WorkerEpoch(0));
    let crashed_key = key_of(&crashed, &fingerprint);
    leave_previous_launch(&journal, &crashed_key, &fingerprint);

    let wm = spawn_ready(&embedded(Some(&journal)), fast_config()).await;
    // Second strike (this launch found the record) ⇒ open: fails fast.
    let poisoned = drive(&wm, smoke_plan(0xc0de, wm.epoch())).await;
    assert!(
        matches!(&poisoned, PlanEnd::Failed(kind) if kind.contains("circuit") && kind.contains(&crashed_key)),
        "{poisoned:?}"
    );
    // An innocent op is dispatched and runs.
    let innocent = drive(&wm, smoke_plan(0x900d, wm.epoch())).await;
    assert!(matches!(innocent, PlanEnd::Prepared { .. }), "{innocent:?}");
    let persisted: Value =
        serde_json::from_slice(&fs::read(journal.join("poison.json")).unwrap()).unwrap();
    let keys: Vec<&String> = persisted["entries"].as_object().unwrap().keys().collect();
    assert_eq!(keys, [&crashed_key], "only the crashed op is struck");
    assert_eq!(persisted["entries"][&crashed_key]["strikes"], 2);
    // The user clears it: persisted strikes go too, and the op runs again.
    assert_eq!(wm.clear_circuit(), 1);
    assert!(!journal.join("poison.json").exists());
    let cleared = drive(&wm, smoke_plan(0xc0de, wm.epoch())).await;
    assert!(matches!(cleared, PlanEnd::Prepared { .. }), "{cleared:?}");
    retire(&wm).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_two_instance_leftover_opens_nothing_and_the_innocent_op_heals() {
    let _gate = GATE.lock().await;
    if !kernel_linked() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let journal = dir.path().join("kernel-journal");
    let fingerprint = fingerprint();
    let crashed = key_of(&smoke_plan(0xc0de, WorkerEpoch(0)), &fingerprint);
    let innocent = key_of(&smoke_plan(0x900d, WorkerEpoch(0)), &fingerprint);
    // Two tabs were regenerating when the previous launch died.
    let op = |key: &str, instance: u64| {
        json!({ "instance": instance, "epoch": 1, "requestId": 9, "verb": "ExecutePlan",
                "opId": key.split('|').nth(1).unwrap(), "stepIndex": 0,
                "opPosition": 0, "key": key, "fingerprint": fingerprint, "startedMs": 0 })
    };
    fs::create_dir_all(&journal).unwrap();
    let inflight = json!({ "version": 1, "session": "an-earlier-launch",
                           "jobs": [op(&crashed, 1), op(&innocent, 2)] });
    fs::write(journal.join("inflight.json"), inflight.to_string()).unwrap();

    let first = spawn_ready(&embedded(Some(&journal)), fast_config()).await;
    let second = spawn_ready(&embedded(Some(&journal)), fast_config()).await;
    let persisted = || -> Value {
        fs::read(journal.join("poison.json"))
            .map_or(Value::Null, |bytes| serde_json::from_slice(&bytes).unwrap())
    };
    assert_eq!(persisted()["entries"][&crashed]["strikes"], 1);
    assert_eq!(persisted()["entries"][&innocent]["strikes"], 1);
    // Neither circuit is open: the innocent op is dispatched (alone), runs and
    // heals; the other keeps its single strike until it, too, runs.
    let healed = drive(&second, smoke_plan(0x900d, second.epoch())).await;
    assert!(matches!(healed, PlanEnd::Prepared { .. }), "{healed:?}");
    assert!(persisted()["entries"].get(&innocent).is_none(), "healed");
    assert_eq!(persisted()["entries"][&crashed]["strikes"], 1);
    let ran = drive(&first, smoke_plan(0xc0de, first.epoch())).await;
    assert!(matches!(ran, PlanEnd::Prepared { .. }), "{ran:?}");
    assert!(!journal.join("poison.json").exists(), "no strike left");
    retire(&first).await;
    retire(&second).await;
}
