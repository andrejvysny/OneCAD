use std::fs;
use std::path::Path;

use onecad_embedded_worker::{JournalEvent, JournalKind};
use serde_json::{json, Value};

use super::*;

const FP: &str = "1235abe6387959f8";

fn event(kind: JournalKind, name: &str, prefix: &str) -> JournalEvent {
    JournalEvent {
        kind,
        request_id: 7,
        name: name.into(),
        step_index: 1,
        op_position: 0,
        prefix_hash: prefix.into(),
        occt_fingerprint: FP.into(),
    }
}

fn read(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

/// An `inflight.json` as a crashed EARLIER launch left it (a fresh session per
/// call: each call is another launch). Returns that session.
fn leave_leftover(dir: &Path, jobs: &[Value]) -> String {
    let session = uuid::Uuid::new_v4().to_string();
    leave_session(dir, &session, jobs);
    session
}

fn leave_session(dir: &Path, session: &str, jobs: &[Value]) {
    fs::create_dir_all(dir).unwrap();
    let file = json!({ "version": 1, "session": session, "jobs": jobs });
    fs::write(dir.join(INFLIGHT), serde_json::to_vec(&file).unwrap()).unwrap();
}

/// The persisted strikes of `key` (0 when absent).
fn strikes(dir: &Path, key: &str) -> u64 {
    let Ok(bytes) = fs::read(dir.join(POISON)) else {
        return 0;
    };
    let file: Value = serde_json::from_slice(&bytes).unwrap();
    file["entries"][key]["strikes"].as_u64().unwrap_or(0)
}

fn op_record(key: Option<&str>, fingerprint: &str) -> Value {
    json!({ "instance": 1, "epoch": 1, "requestId": 3, "verb": "ExecutePlan",
            "opId": key.map(|k| k.split('|').nth(1).unwrap()), "stepIndex": 0,
            "opPosition": 0, "key": key, "fingerprint": fingerprint, "startedMs": 0 })
}

#[test]
fn records_follow_the_job_and_vanish_after_its_terminal_frame() {
    let dir = tempfile::tempdir().unwrap();
    let journal = Arc::new(KernelJournal::open(dir.path()));
    let writer = journal.writer(4);
    writer.record(&event(JournalKind::JobBegin, "ExecutePlan", ""));
    writer.record(&event(JournalKind::PlanOpBegin, "op_extrude", "ab12"));
    let file = read(&dir.path().join(INFLIGHT));
    assert_eq!(file["session"], session());
    assert_eq!(file["jobs"][0]["key"], format!("ab12|op_extrude|{FP}"));
    assert_eq!(file["jobs"][0]["epoch"], 4);
    writer.record(&event(JournalKind::JobEnd, "ExecutePlan", ""));
    assert!(
        !dir.path().join(INFLIGHT).exists(),
        "no job left: no record"
    );
    assert_eq!(journal.write_failures(), 0);
}

#[test]
fn a_leftover_op_record_strikes_only_its_own_key_and_opens_at_the_threshold() {
    let dir = tempfile::tempdir().unwrap();
    let crashing = format!("aa|op_bad|{FP}");
    let leftover = [
        op_record(Some(&crashing), FP),
        // A non-plan job (no op) and another OCCT build: neither poisons.
        op_record(None, FP),
        op_record(Some("bb|op_other|0000000000000000"), "0000000000000000"),
    ];
    leave_leftover(dir.path(), &leftover);
    let first = KernelJournal::open(dir.path()).seeds(FP);
    assert_eq!(first.len(), 1, "only the crashing op's key: {first:?}");
    assert_eq!(
        (first[0].key.as_str(), first[0].strikes, first[0].open),
        (crashing.as_str(), 1, false)
    );
    assert!(
        !dir.path().join(INFLIGHT).exists(),
        "the leftover is consumed"
    );

    leave_leftover(dir.path(), &leftover[..1]);
    let second = KernelJournal::open(dir.path()).seeds(FP);
    assert_eq!(
        (second[0].strikes, second[0].open),
        (2, true),
        "a repeat opens the circuit"
    );
    assert!(second[0].message.contains("op_bad"));
}

#[test]
fn a_live_sibling_record_of_this_launch_is_never_a_leftover() {
    let dir = tempfile::tempdir().unwrap();
    let journal = Arc::new(KernelJournal::open(dir.path()));
    let writer = journal.writer(1);
    writer.record(&event(JournalKind::JobBegin, "ExecutePlan", ""));
    writer.record(&event(JournalKind::PlanOpBegin, "op_live", "cc"));
    assert!(dir.path().join(INFLIGHT).exists());
    // A second instance of THIS launch opens the same directory.
    assert!(KernelJournal::open(dir.path()).seeds(FP).is_empty());
}

#[test]
fn healing_and_clearing_forget_persisted_strikes() {
    let dir = tempfile::tempdir().unwrap();
    let key = format!("aa|op_bad|{FP}");
    leave_leftover(dir.path(), &[op_record(Some(&key), FP)]);
    let journal = KernelJournal::open(dir.path());
    assert_eq!(journal.seeds(FP).len(), 1);
    journal.heal(std::slice::from_ref(&key));
    assert!(journal.seeds(FP).is_empty());
    assert!(!dir.path().join(POISON).exists());

    leave_leftover(dir.path(), &[op_record(Some(&key), FP)]);
    let journal = KernelJournal::open(dir.path());
    assert_eq!(journal.seeds(FP).len(), 1);
    journal.clear();
    assert!(journal.seeds(FP).is_empty());
}

#[test]
fn a_failing_write_is_counted_and_never_panics() {
    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("not-a-directory");
    fs::write(&blocker, b"x").unwrap();
    let journal = Arc::new(KernelJournal::open(&blocker.join("kernel-journal")));
    let writer = journal.writer(1);
    writer.record(&event(JournalKind::JobBegin, "Tessellate", ""));
    writer.record(&event(JournalKind::JobEnd, "Tessellate", ""));
    assert!(
        journal.write_failures() >= 2,
        "{}",
        journal.write_failures()
    );
}

#[test]
fn an_ambiguous_leftover_never_opens_and_only_an_unambiguous_repeat_does() {
    let dir = tempfile::tempdir().unwrap();
    let crashing = format!("aa|op_bad|{FP}");
    let innocent = format!("bb|op_good|{FP}");
    let both = [
        op_record(Some(&crashing), FP),
        op_record(Some(&innocent), FP),
    ];
    // Two tabs had ops in flight: each is struck once, neither opens…
    leave_leftover(dir.path(), &both);
    let seeds = KernelJournal::open(dir.path()).seeds(FP);
    assert_eq!(seeds.len(), 2, "{seeds:?}");
    assert!(seeds.iter().all(|s| s.strikes == 1 && !s.open), "{seeds:?}");
    // …and a second ambiguous crash still opens nothing (capped below 2).
    leave_leftover(dir.path(), &both);
    let seeds = KernelJournal::open(dir.path()).seeds(FP);
    assert!(seeds.iter().all(|s| s.strikes == 1 && !s.open), "{seeds:?}");
    // The crashing op, run alone, crashes alone: now it opens; the innocent
    // sibling keeps its single strike (it heals once it runs alone).
    leave_leftover(dir.path(), &both[..1]);
    let journal = KernelJournal::open(dir.path());
    let seeds = journal.seeds(FP);
    let by_key = |key: &str| seeds.iter().find(|s| s.key == key).cloned().unwrap();
    assert_eq!(
        (by_key(&crashing).strikes, by_key(&crashing).open),
        (2, true)
    );
    assert_eq!(
        (by_key(&innocent).strikes, by_key(&innocent).open),
        (1, false)
    );
    journal.heal(std::slice::from_ref(&innocent));
    assert_eq!(strikes(dir.path(), &innocent), 0, "healed: forgotten");
    assert_eq!(strikes(dir.path(), &crashing), 2);
}

#[test]
fn a_recovery_killed_before_the_leftover_was_consumed_strikes_once() {
    let dir = tempfile::tempdir().unwrap();
    let key = format!("aa|op_bad|{FP}");
    let session = leave_leftover(dir.path(), &[op_record(Some(&key), FP)]);
    assert_eq!(KernelJournal::open(dir.path()).seeds(FP)[0].strikes, 1);
    // poison.json is written first; a launch killed before inflight.json was
    // replaced leaves the same leftover behind.
    leave_session(dir.path(), &session, &[op_record(Some(&key), FP)]);
    let seeds = KernelJournal::open(dir.path()).seeds(FP);
    assert_eq!(
        seeds[0].strikes, 1,
        "the same leftover is never applied twice"
    );
    assert!(
        !dir.path().join(INFLIGHT).exists(),
        "and it is consumed now"
    );
}

/// `Ok` when `permit` is granted within 100 ms, `Err` while it waits.
async fn pending<F: std::future::Future>(
    permit: F,
) -> Result<F::Output, tokio::time::error::Elapsed> {
    tokio::time::timeout(std::time::Duration::from_millis(100), permit).await
}

#[tokio::test]
async fn a_struck_key_dispatches_alone_and_others_share() {
    let dir = tempfile::tempdir().unwrap();
    let struck = format!("aa|op_bad|{FP}");
    let plain = vec![format!("cc|op_plain|{FP}")];
    leave_leftover(dir.path(), &[op_record(Some(&struck), FP)]);
    let journal = Arc::new(KernelJournal::open(dir.path()));
    assert_eq!(journal.seeds(FP).len(), 1);

    let first = journal.plan_permit(&plain).await;
    let second = pending(journal.plan_permit(&plain)).await;
    assert!(second.is_ok(), "plans without a strike share the kernel");
    let keys = vec![plain[0].clone(), struck.clone()];
    assert!(
        pending(journal.plan_permit(&keys)).await.is_err(),
        "a struck key waits for every running plan"
    );
    drop((first, second));
    let alone = journal.plan_permit(&keys).await;
    assert!(
        pending(journal.plan_permit(&plain)).await.is_err(),
        "nothing runs beside it"
    );
    drop(alone);
    journal.heal(&keys);
    let healed = journal.plan_permit(&keys).await;
    assert!(
        pending(journal.plan_permit(&plain)).await.is_ok(),
        "healed: shared again"
    );
    drop(healed);
}
