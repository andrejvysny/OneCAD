//! The in-process kernel's crash journal (docs/ios/ARCHITECTURE.md "In-process
//! failure policy"; format: docs/ios/KERNEL.md).
//!
//! In-process an OCCT crash kills the app, and the in-memory crash circuit dies
//! with it. The worker therefore reports every kernel job and plan op through
//! the ABI journal callback, synchronously before queue admission and before each
//! plan op runs, and this module persists it:
//!
//! * `inflight.json` — this process's admitted kernel and solver jobs (independent
//!   identities within every live instance), rewritten by temp + rename on every
//!   event (no fsync: it must
//!   survive process death, not power loss) and removed when none is left. It
//!   carries a per-launch session token, so a record from another launch is
//!   told apart from a live sibling instance's without asking about pids.
//! * `poison.json` — strikes per crash-circuit key (`prefixHash|opId|fingerprint`,
//!   the `plan_op_keys` format). A leftover op record from an EARLIER launch is a
//!   *suspected* interruption: one strike for its key. A record under another
//!   OCCT fingerprint is discarded, and a record without an op (a non-plan job)
//!   poisons nothing — termination, jetsam or backgrounding never poison an op
//!   without a leftover op record. At [`IN_PROCESS_POISON_THRESHOLD`] strikes the
//!   key's circuit opens until the user clears it.
//! * Attribution: when several jobs were unfinished (across lanes or tabs), the
//!   leftover cannot say which one caused process loss. Each gets its strike, but an
//!   AMBIGUOUS strike never opens a circuit; and every key holding a strike
//!   runs ALONE among this process's plan dispatches ([`KernelJournal::plan_permit`]),
//!   reducing plan overlap without claiming exclusive access against solvers.
//!   Successful plans heal their keys; only a complete, single supported plan
//!   record can advance a suspicion to quarantine.
//!
//! Every write is best effort: a failure is logged, counted and surfaced in
//! worker status, and never blocks the op.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use onecad_embedded_worker::{JournalEvent, JournalKind};
use serde::{Deserialize, Serialize};

/// Leftover strikes at which a key's circuit opens. Two, not one: a single
/// leftover record can be an innocent jetsam or a kill during a long op, while
/// a deterministic crash repeats at the same key on the very next replay — so
/// a crashing op costs at most two crashes before regeneration stops before it.
pub(crate) const IN_PROCESS_POISON_THRESHOLD: u32 = 2;

const INFLIGHT: &str = "inflight.json";
const POISON: &str = "poison.json";
const FORMAT_VERSION: u32 = 2;
const MAX_JOBS: usize = 512;
const MAX_POISON: usize = 128;
const MAX_FILE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum JobKind {
    #[default]
    Kernel,
    Solver,
}

impl JobKind {
    fn for_verb(verb: &str) -> Self {
        if matches!(
            verb,
            "SketchUpsert" | "SketchRegions" | "BeginGesture" | "SolveDrag" | "EndGesture"
        ) {
            Self::Solver
        } else {
            Self::Kernel
        }
    }
}

/// One in-flight kernel job of one instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct JobRecord {
    pub(crate) instance: u64,
    pub(crate) epoch: u64,
    pub(crate) request_id: u64,
    pub(crate) verb: String,
    #[serde(default)]
    pub(crate) kind: JobKind,
    pub(crate) op_id: Option<String>,
    pub(crate) step_index: Option<u64>,
    pub(crate) op_position: Option<u64>,
    /// The crash-circuit key of the op in progress; `None` outside plan ops.
    pub(crate) key: Option<String>,
    pub(crate) fingerprint: String,
    pub(crate) started_ms: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InflightFile {
    version: u32,
    session: String,
    jobs: Vec<JobRecord>,
    #[serde(default)]
    complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PoisonRecord {
    pub(crate) strikes: u32,
    pub(crate) op_id: String,
    pub(crate) last_ms: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PoisonFile {
    version: u32,
    entries: BTreeMap<String, PoisonRecord>,
    /// The leftover session whose strikes these entries already contain:
    /// poison.json is written BEFORE that leftover is consumed, so a launch
    /// killed in between must not strike it twice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    applied_session: Option<String>,
}

/// Another launch's `inflight.json`, read at open, applied at the first seed.
struct Leftover {
    session: String,
    jobs: Vec<JobRecord>,
    complete: bool,
}

#[derive(Default)]
struct State {
    jobs: BTreeMap<(u64, u64, u64), JobRecord>,
    leftover: Option<Leftover>,
    poison: BTreeMap<String, PoisonRecord>,
    applied_session: Option<String>,
    incomplete: bool,
}

/// The process-wide journal of one directory.
pub(crate) struct KernelJournal {
    dir: PathBuf,
    state: Mutex<State>,
    failures: AtomicU64,
    next_instance: AtomicU64,
    /// Plan dispatch order across this process's instances: shared for a
    /// plan with no struck key, exclusive for one with (module docs).
    order: Arc<tokio::sync::RwLock<()>>,
}

impl std::fmt::Debug for KernelJournal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KernelJournal")
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

/// This launch's token (stamped in `inflight.json`).
fn session() -> &'static str {
    static SESSION: OnceLock<String> = OnceLock::new();
    SESSION.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// temp + rename, no fsync (docs/ios/ARCHITECTURE.md durability).
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

fn remove_if_present(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

impl KernelJournal {
    /// The one journal for `dir` in this process (every in-process instance
    /// shares its `inflight.json`, so their writes must be serialized here).
    pub(crate) fn shared(dir: &Path) -> Arc<KernelJournal> {
        static REGISTRY: OnceLock<Mutex<HashMap<PathBuf, Arc<KernelJournal>>>> = OnceLock::new();
        let mut registry = REGISTRY.get_or_init(Mutex::default).lock().unwrap();
        registry
            .entry(dir.to_path_buf())
            .or_insert_with(|| Arc::new(Self::open(dir)))
            .clone()
    }

    fn open(dir: &Path) -> KernelJournal {
        let journal = KernelJournal {
            dir: dir.to_path_buf(),
            state: Mutex::default(),
            failures: AtomicU64::new(0),
            next_instance: AtomicU64::new(1),
            order: Arc::default(),
        };
        if let Err(e) = fs::create_dir_all(dir) {
            journal.note_failure("create journal dir", &e);
        }
        let mut state = journal.state.lock().unwrap();
        state.leftover = journal.read_leftover();
        let poison = journal.read_poison();
        state.poison = poison.entries;
        state.applied_session = poison.applied_session;
        drop(state);
        journal
    }

    /// Records of an earlier launch; `None` when there is nothing to recover.
    fn read_leftover(&self) -> Option<Leftover> {
        let path = self.dir.join(INFLIGHT);
        let bytes = self.read_bounded(&path)?;
        match serde_json::from_slice::<InflightFile>(&bytes) {
            Ok(mut file)
                if matches!(file.version, 1 | FORMAT_VERSION)
                    && file.jobs.len() <= MAX_JOBS
                    && !file.session.is_empty()
                    && file.session != session() =>
            {
                if file.version == 1 {
                    // Version 1 never recorded solver overlap.
                    file.complete = false;
                    for record in &mut file.jobs {
                        record.kind = JobKind::for_verb(&record.verb);
                    }
                }
                Some(Leftover {
                    session: file.session,
                    jobs: file.jobs,
                    complete: file.complete,
                })
            }
            Ok(file) if file.session == session() => None,
            Ok(_) => {
                self.note_failure(
                    "inflight.json read",
                    &"unsupported version or oversized journal",
                );
                Some(Leftover {
                    session: String::new(),
                    jobs: Vec::new(),
                    complete: false,
                })
            }
            Err(e) => {
                self.note_failure("inflight.json parse", &e);
                Some(Leftover {
                    session: String::new(),
                    jobs: Vec::new(),
                    complete: false,
                })
            }
        }
    }

    fn read_bounded(&self, path: &Path) -> Option<Vec<u8>> {
        use std::io::Read;
        let file = match fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
            Err(error) => {
                self.note_failure("journal read", &error);
                return None;
            }
        };
        let mut bytes = Vec::new();
        if let Err(error) = file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes) {
            self.note_failure("journal read", &error);
            return None;
        }
        if bytes.len() as u64 > MAX_FILE_BYTES {
            self.note_failure("journal read", &"oversized journal discarded");
            return None;
        }
        Some(bytes)
    }

    fn read_poison(&self) -> PoisonFile {
        let Some(bytes) = self.read_bounded(&self.dir.join(POISON)) else {
            return PoisonFile::default();
        };
        match serde_json::from_slice::<PoisonFile>(&bytes) {
            Ok(mut file)
                if matches!(file.version, 1 | FORMAT_VERSION)
                    && file.entries.len() <= MAX_POISON =>
            {
                if file.version == 1 {
                    for record in file.entries.values_mut() {
                        record.strikes = record.strikes.min(IN_PROCESS_POISON_THRESHOLD - 1);
                    }
                }
                file
            }
            _ => {
                self.note_failure(
                    "poison.json read",
                    &"malformed or unsupported journal discarded",
                );
                PoisonFile::default()
            }
        }
    }

    /// A writer for one new instance (its records are keyed by a fresh id).
    pub(crate) fn writer(self: &Arc<Self>, epoch: u64) -> JournalWriter {
        JournalWriter {
            journal: self.clone(),
            instance: self.next_instance.fetch_add(1, Ordering::Relaxed),
            epoch,
        }
    }

    pub(crate) fn write_failures(&self) -> u64 {
        self.failures.load(Ordering::SeqCst)
    }

    fn note_failure(&self, what: &str, error: &dyn std::fmt::Display) {
        let count = self.failures.fetch_add(1, Ordering::SeqCst) + 1;
        // Rate-limited: a dead disk must not turn every op into a log line.
        if count.is_power_of_two() {
            tracing::warn!(failures = count, "kernel journal: {what} failed: {error}");
        }
    }

    /// Rewrites `inflight.json` from `state.jobs` (removed when empty).
    fn persist_inflight(&self, state: &State) {
        let path = self.dir.join(INFLIGHT);
        let complete = !state.incomplete && self.write_failures() == 0;
        let result = if state.jobs.is_empty() && complete {
            remove_if_present(&path)
        } else {
            let file = InflightFile {
                version: FORMAT_VERSION,
                session: session().to_string(),
                jobs: state.jobs.values().cloned().collect(),
                complete,
            };
            serde_json::to_vec(&file)
                .map_err(std::io::Error::other)
                .and_then(|bytes| write_atomic(&path, &bytes))
        };
        if let Err(e) = result {
            self.note_failure("inflight.json write", &e);
        }
    }

    fn persist_poison(&self, state: &State) {
        let path = self.dir.join(POISON);
        let result = if state.poison.is_empty() {
            remove_if_present(&path)
        } else {
            let file = PoisonFile {
                version: FORMAT_VERSION,
                entries: state.poison.clone(),
                applied_session: state.applied_session.clone(),
            };
            serde_json::to_vec(&file)
                .map_err(std::io::Error::other)
                .and_then(|bytes| write_atomic(&path, &bytes))
        };
        if let Err(e) = result {
            self.note_failure("poison.json write", &e);
        }
    }
}

/// One instance's journal: callbacks may overlap on reader and lane threads.
pub(crate) struct JournalWriter {
    journal: Arc<KernelJournal>,
    instance: u64,
    epoch: u64,
}

impl JournalWriter {
    pub(crate) fn record(&self, event: &JournalEvent) {
        let journal = &self.journal;
        let mut state = journal.state.lock().unwrap();
        if [&event.name, &event.prefix_hash, &event.occt_fingerprint]
            .iter()
            .any(|value| value.len() > 128)
        {
            state.incomplete = true;
            journal.note_failure("journal event", &"identity field exceeds 128 bytes");
            journal.persist_inflight(&state);
            return;
        }
        let identity = (self.instance, self.epoch, event.request_id);
        match event.kind {
            JournalKind::JobBegin => {
                if state.jobs.contains_key(&identity) || state.jobs.len() >= MAX_JOBS {
                    state.incomplete = true;
                    journal.note_failure("job begin", &"duplicate identity or job limit reached");
                    journal.persist_inflight(&state);
                    return;
                }
                let record = JobRecord {
                    instance: self.instance,
                    epoch: self.epoch,
                    request_id: event.request_id,
                    verb: event.name.clone(),
                    kind: JobKind::for_verb(&event.name),
                    op_id: None,
                    step_index: None,
                    op_position: None,
                    key: None,
                    fingerprint: event.occt_fingerprint.clone(),
                    started_ms: now_ms(),
                };
                state.jobs.insert(identity, record);
            }
            JournalKind::PlanOpBegin => {
                let Some(record) = state.jobs.get_mut(&identity) else {
                    return; // an op outside a recorded job: nothing to attribute
                };
                if record.kind != JobKind::Kernel
                    || !matches!(record.verb.as_str(), "ExecutePlan" | "PreviewPlan")
                    || record.fingerprint != event.occt_fingerprint
                    || event.name.is_empty()
                    || event.prefix_hash.is_empty()
                {
                    return;
                }
                record.op_id = Some(event.name.clone());
                record.step_index = Some(event.step_index);
                record.op_position = Some(event.op_position);
                record.key = Some(format!(
                    "{}|{}|{}",
                    event.prefix_hash, event.name, event.occt_fingerprint
                ));
            }
            JournalKind::JobEnd => {
                if state.jobs.get(&identity).is_some_and(|record| {
                    record.verb == event.name
                        && record.fingerprint == event.occt_fingerprint
                        && record.kind == JobKind::for_verb(&event.name)
                }) {
                    state.jobs.remove(&identity);
                }
            }
        }
        journal.persist_inflight(&state);
    }
}

mod recovery;

#[cfg(test)]
mod tests;
