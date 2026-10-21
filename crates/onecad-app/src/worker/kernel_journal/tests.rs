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

/// An `inflight.json` as a crashed EARLIER launch left it.
fn leave_leftover(dir: &Path, jobs: &[Value]) {
    fs::create_dir_all(dir).unwrap();
    let file = json!({ "version": 1, "session": "an-earlier-launch", "jobs": jobs });
    fs::write(dir.join(INFLIGHT), serde_json::to_vec(&file).unwrap()).unwrap();
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
