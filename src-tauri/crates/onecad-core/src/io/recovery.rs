//! Autosave layout + crash-recovery session markers.
//!
//! # Layout
//!
//! Under an app-data root, autosave state lives in an `autosave/` subdirectory:
//!
//! ```text
//! <app_data>/autosave/<documentId>.onecad         the autosave container
//! <app_data>/autosave/<documentId>.session.json   the live-session crash marker
//! ```
//!
//! While a document is open, the app writes a [`SessionMarker`] recording its
//! `pid`, the document's real on-disk path (if any), its title, and the last
//! autosave time. On a clean close the marker is removed ([`remove_marker`]). If the
//! app crashes, the marker survives.
//!
//! The next launch calls [`scan_recoverable`], which iterates the autosave
//! CONTAINERS and uses the marker only as evidence of who owns each one: a live
//! owner means skip, a dead owner means offer, and no marker at all still means
//! offer — as an unlabelled orphan. Keying discovery to the marker instead made a
//! container unreachable the moment its marker was consumed or lost, however recent
//! and however large the work in it. Containers unclaimed past [`ORPHAN_TTL`] are
//! swept.
//!
//! ## The autosave writer is the app's job
//!
//! This module owns only the **layout and marker lifecycle**. Writing the autosave
//! container reuses [`ContainerWriter`](super::container::ContainerWriter) at the
//! path from [`autosave_path`]; scheduling is the app layer's responsibility — there
//! are no timers here (the core stays pure and side-effect-explicit). It debounces,
//! bounded by [`AUTOSAVE_INTERVAL_SECS`], which `autosave::AUTOSAVE_MAX_AGE` reads.
//!
//! ## Platform note — liveness is injected
//!
//! [`scan_recoverable`] takes `pid_alive: impl Fn(u32) -> bool` rather than
//! probing processes itself, so `onecad-core` needs **no `libc`/`sysinfo`
//! dependency**. On Unix the app typically implements it as `kill(pid, 0) != ESRCH`;
//! on Windows via `OpenProcess`. A conservative predicate (treat unknown as alive)
//! simply defers recovery to a later launch — never a false "safe to discard".

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::ids::DocumentId;

use super::{IoError, IoResult};

/// The longest an autosave may be deferred once there is unsaved work, in seconds
/// (~2 min; V1/V2 plan). The app scheduler owns the cadence and debounces below
/// this; `autosave::AUTOSAVE_MAX_AGE` is the ceiling built from this constant.
pub const AUTOSAVE_INTERVAL_SECS: u64 = 120;

/// How long an unclaimed autosave container stays on offer before
/// [`scan_recoverable`] deletes it.
///
/// Recovery state is otherwise only ever removed by a deliberate act — a save, a
/// clean close, or the user pressing Discard. A container whose owner died and whom
/// nobody came back for has no such act coming, so without a TTL the autosave
/// directory grows for the life of the install. 30 days is long enough that "I was
/// on holiday" is not a data-loss event.
pub const ORPHAN_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// The `.onecad` container extension (autosave files and real documents share it).
pub const CONTAINER_EXT: &str = "onecad";

/// The autosave directory under an app-data root (`<app_data>/autosave`).
#[must_use]
pub fn autosave_dir(app_data: &Path) -> PathBuf {
    app_data.join("autosave")
}

/// The autosave container path for a document
/// (`<app_data>/autosave/<documentId>.onecad`).
#[must_use]
pub fn autosave_path(app_data: &Path, document_id: DocumentId) -> PathBuf {
    autosave_dir(app_data).join(format!("{document_id}.{CONTAINER_EXT}"))
}

/// The session-marker path for a document
/// (`<app_data>/autosave/<documentId>.session.json`).
#[must_use]
pub fn marker_path(app_data: &Path, document_id: DocumentId) -> PathBuf {
    autosave_dir(app_data).join(format!("{document_id}.session.json"))
}

/// A live-session crash marker (V1/V2 plan "pid crash marker").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMarker {
    /// The document this session is editing.
    pub document_id: DocumentId,
    /// The owning process id (checked for liveness on the next launch).
    pub pid: u32,
    /// The document's real on-disk path, if it has been saved (RFC3339-free — a
    /// filesystem path). `None` for an unsaved (autosave-only) document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opened_path: Option<PathBuf>,
    /// RFC3339 timestamp of the last autosave (caller-supplied — the core does not
    /// read the wall clock).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub last_autosave: String,
    /// The document's display title at the last autosave.
    ///
    /// Recorded because the autosave container is named `<documentId>.onecad`, so
    /// re-opening it derives the title from that path and the recovered document
    /// comes up titled with a raw UUID. `opened_path` covers a saved document; this
    /// covers the never-saved one, which is exactly the case recovery matters most
    /// for. `None` on a marker written by a build older than this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// A recovery candidate produced by [`scan_recoverable`]: an autosave container no
/// live session owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryOffer {
    /// The document to offer for recovery.
    pub document_id: DocumentId,
    /// The autosave container to recover from.
    pub autosave_path: PathBuf,
    /// The session marker, when one survives — it carries the last-autosave time,
    /// the document's real path and its title, all of which the offer UI wants.
    ///
    /// `None` for an **orphan**: a container whose marker was lost, corrupted, or
    /// written by a build that crashed between the two writes. The container is
    /// still the user's work and is still offered; only the labelling degrades.
    pub marker: Option<SessionMarker>,
    /// The container's last-modified time in Unix-epoch milliseconds (`0` when
    /// unreadable). The offer ordering key — recency is what the user reasons
    /// about, and it is the one fact available with or without a marker.
    pub modified_ms: u64,
}

/// Writes (or overwrites) the session marker for the marker's document, atomically
/// (tmp + rename). Creates the autosave directory if absent.
///
/// # Errors
/// [`IoError::Io`] on a filesystem failure.
pub fn write_marker(app_data: &Path, marker: &SessionMarker) -> IoResult<()> {
    let dir = autosave_dir(app_data);
    std::fs::create_dir_all(&dir)?;
    let path = marker_path(app_data, marker.document_id);
    let bytes = serde_json::to_vec_pretty(marker)
        .map_err(|e| IoError::Io(format!("marker serialize: {e}")))?;
    let tmp = path.with_extension("session.json.tmp");
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Removes a document's session marker (a clean close). Absent marker is not an
/// error.
///
/// # Errors
/// [`IoError::Io`] on a filesystem failure other than "not found".
pub fn remove_marker(app_data: &Path, document_id: DocumentId) -> IoResult<()> {
    let path = marker_path(app_data, document_id);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Scans the autosave directory for containers **no live session owns**, newest
/// first, sweeping any that have gone stale past [`ORPHAN_TTL`].
///
/// ## Why this iterates containers, not markers
///
/// The container holds the work; the marker is only evidence about who owns it. An
/// earlier version iterated markers, which made every container reachable *solely*
/// through its marker — so consuming a marker (which `recover_document` did) or
/// losing one to a crash between the two writes silently orphaned the user's work
/// forever, however recent and however large. Inverting the loop means a lost
/// marker costs a label, not a document.
///
/// Per container:
///
/// * **marker present, owner alive** ⇒ skipped. A running session owns it, and
///   offering a document that is open in another window would be a lie.
/// * **marker present, owner dead** ⇒ offered, fully labelled.
/// * **no marker** ⇒ offered as an orphan (`marker: None`), labelled by mtime only.
/// * **older than [`ORPHAN_TTL`]** ⇒ deleted, with its marker, and not offered.
///
/// A missing autosave directory yields an empty list. Foreign files, unparseable
/// names and unreadable markers are skipped, never fatal.
///
/// `now` is injected for the same reason `pid_alive` is: this module does not read
/// the wall clock, so a TTL test can age a container without touching the
/// filesystem's own timestamps.
///
/// # Errors
/// [`IoError::Io`] if the directory exists but cannot be read.
pub fn scan_recoverable(
    app_data: &Path,
    pid_alive: impl Fn(u32) -> bool,
    now: SystemTime,
) -> IoResult<Vec<RecoveryOffer>> {
    let dir = autosave_dir(app_data);
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };

    let mut offers = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        // `<documentId>.onecad`, and nothing else in this directory.
        if path.extension().and_then(|e| e.to_str()) != Some(CONTAINER_EXT) {
            continue;
        }
        let Some(document_id) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.parse::<uuid::Uuid>().ok())
            .map(DocumentId)
        else {
            continue; // not one of ours.
        };

        let marker = read_marker(app_data, document_id);
        if marker.as_ref().is_some_and(|m| pid_alive(m.pid)) {
            continue; // a live session owns this container.
        }

        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if is_expired(modified, now) {
            // Sweep. This is the only place recovery state is deleted without the
            // user saying so, which is why it needs a real age and not merely a
            // failed stat: `is_expired` answers false for an unreadable or
            // future-dated mtime.
            sweep_expired(app_data, document_id, &path);
            continue;
        }
        offers.push(RecoveryOffer {
            document_id,
            autosave_path: path,
            marker,
            modified_ms: modified.map(unix_millis).unwrap_or(0),
        });
    }
    // Newest first — recency is what the user reasons about when several sessions
    // crashed. `document_id` breaks ties so the order is total and reproducible
    // (directory iteration order is unspecified).
    offers.sort_by(|a, b| {
        b.modified_ms
            .cmp(&a.modified_ms)
            .then_with(|| a.document_id.as_uuid().cmp(&b.document_id.as_uuid()))
    });
    Ok(offers)
}

/// Reads a document's session marker, or `None` when it is absent, unreadable or
/// unparseable. All three mean the same thing to discovery: no owner evidence.
fn read_marker(app_data: &Path, document_id: DocumentId) -> Option<SessionMarker> {
    let bytes = std::fs::read(marker_path(app_data, document_id)).ok()?;
    serde_json::from_slice::<SessionMarker>(&bytes).ok()
}

/// Whether a container's mtime puts it past [`ORPHAN_TTL`] as of `now`.
///
/// Deliberately conservative — `false` unless the age is both readable and sane. A
/// missing mtime, or one in the future (a clock jump, a restored backup, a file
/// copied off another machine), must never be read as "old enough to delete".
fn is_expired(modified: Option<SystemTime>, now: SystemTime) -> bool {
    modified
        .and_then(|t| now.duration_since(t).ok())
        .is_some_and(|age| age > ORPHAN_TTL)
}

/// Deletes an expired container and its marker. Best-effort; a failure is logged by
/// the caller's absence of an offer, not by an error (a sweep that cannot run must
/// not fail the scan and hide every other offer).
fn sweep_expired(app_data: &Path, document_id: DocumentId, container: &Path) {
    let _ = std::fs::remove_file(container);
    let _ = remove_marker(app_data, document_id);
}

/// A `SystemTime` as Unix-epoch milliseconds (`0` before the epoch).
fn unix_millis(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn marker(app_data: &Path, id: DocumentId, pid: u32) -> SessionMarker {
        SessionMarker {
            document_id: id,
            pid,
            opened_path: Some(app_data.join("real.onecad")),
            last_autosave: "2026-07-16T12:00:00Z".into(),
            title: Some("real".into()),
        }
    }

    #[test]
    fn marker_lifecycle_write_read_remove() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let id = DocumentId(Uuid::from_u128(0xABCD));
        write_marker(root, &marker(root, id, 4321)).unwrap();
        assert!(marker_path(root, id).exists());
        // idempotent overwrite
        write_marker(root, &marker(root, id, 4321)).unwrap();
        remove_marker(root, id).unwrap();
        assert!(!marker_path(root, id).exists());
        // remove of an absent marker is fine
        remove_marker(root, id).unwrap();
    }

    /// Writes a container, pausing first so its mtime is strictly newer than the
    /// previous one (the ordering key must be unambiguous).
    fn write_container_after_a_beat(root: &Path, id: DocumentId) {
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(autosave_path(root, id), b"x").unwrap();
    }

    #[test]
    fn scan_offers_only_containers_no_live_session_owns() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        let dead = DocumentId(Uuid::from_u128(1));
        let alive = DocumentId(Uuid::from_u128(2));
        let dead_no_file = DocumentId(Uuid::from_u128(3));

        write_marker(root, &marker(root, dead, 1000)).unwrap();
        write_marker(root, &marker(root, alive, 2000)).unwrap();
        write_marker(root, &marker(root, dead_no_file, 3000)).unwrap();

        // The container is what is offered, so only `dead` and `alive` get one.
        std::fs::write(autosave_path(root, dead), b"x").unwrap();
        std::fs::write(autosave_path(root, alive), b"x").unwrap();

        let alive_pids = [2000u32];
        let offers =
            scan_recoverable(root, |pid| alive_pids.contains(&pid), SystemTime::now()).unwrap();
        assert_eq!(
            offers.len(),
            1,
            "the live session's container is not offered"
        );
        assert_eq!(offers[0].document_id, dead);
        assert_eq!(offers[0].marker.as_ref().unwrap().pid, 1000);
        // A marker with no container is not an offer — there is nothing to recover.
        assert!(!offers.iter().any(|o| o.document_id == dead_no_file));
    }

    #[test]
    fn scan_missing_dir_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let offers =
            scan_recoverable(&tmp.path().join("nope"), |_| false, SystemTime::now()).unwrap();
        assert!(offers.is_empty());
    }

    /// FIX PIN (was a characterization test of DI-1, data-integrity audit
    /// 2026-08-14). Discovery used to be keyed to the MARKER, so an autosave whose
    /// marker had been removed was unreachable no matter how recent or how large.
    /// That is exactly the state `api::recover_document` used to leave behind, and
    /// it meant a second crash after a Restore recovered nothing.
    ///
    /// Discovery now iterates CONTAINERS, so the work survives the loss of its
    /// marker. A green-to-red flip here means discovery went back to being
    /// marker-keyed.
    #[test]
    fn an_autosave_whose_marker_is_gone_is_still_offered() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let id = DocumentId(Uuid::from_u128(0xDEC0));

        write_marker(root, &marker(root, id, 1000)).unwrap();
        std::fs::write(autosave_path(root, id), b"recovered work").unwrap();
        // Control: with the marker present, a dead owner IS offered, fully labelled.
        let labelled = scan_recoverable(root, |_| false, SystemTime::now()).unwrap();
        assert_eq!(labelled.len(), 1);
        assert!(labelled[0].marker.is_some());

        remove_marker(root, id).unwrap();
        assert!(
            autosave_path(root, id).exists(),
            "precondition: the work is still on disk"
        );

        let orphaned = scan_recoverable(root, |_| false, SystemTime::now()).unwrap();
        assert_eq!(
            orphaned.len(),
            1,
            "the surviving container is still discoverable without its marker"
        );
        assert_eq!(orphaned[0].document_id, id);
        assert!(
            orphaned[0].marker.is_none(),
            "an orphan is offered, just unlabelled"
        );
    }

    #[test]
    fn scan_orders_newest_first() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(autosave_dir(root)).unwrap();

        // Written oldest-first with ASCENDING uuids, so the expected answer
        // ([newest, middle, oldest]) is the exact reverse of a uuid-ordered scan —
        // which is what this replaced.
        let oldest = DocumentId(Uuid::from_u128(1));
        let middle = DocumentId(Uuid::from_u128(2));
        let newest = DocumentId(Uuid::from_u128(3));
        for id in [oldest, middle, newest] {
            write_container_after_a_beat(root, id);
        }

        let offers = scan_recoverable(root, |_| false, SystemTime::now()).unwrap();
        let order: Vec<DocumentId> = offers.iter().map(|o| o.document_id).collect();
        assert_eq!(order, vec![newest, middle, oldest]);
    }

    #[test]
    fn scan_sweeps_containers_past_the_ttl() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let fresh = DocumentId(Uuid::from_u128(0xF1));
        let expired = DocumentId(Uuid::from_u128(0xE2));

        // Ageing runs off the injected clock, so the filesystem's own timestamps stay
        // untouched. `now` is TTL+100 ms past `base`: `expired` is written at ~`base`
        // and lands just over the line, `fresh` is written 300 ms later and lands
        // 200 ms inside it.
        let base = SystemTime::now();
        write_marker(root, &marker(root, expired, 1000)).unwrap();
        std::fs::write(autosave_path(root, expired), b"x").unwrap();
        std::thread::sleep(Duration::from_millis(300));
        write_marker(root, &marker(root, fresh, 1000)).unwrap();
        std::fs::write(autosave_path(root, fresh), b"x").unwrap();
        let now = base + ORPHAN_TTL + Duration::from_millis(100);

        let offers = scan_recoverable(root, |_| false, now).unwrap();
        assert_eq!(offers.len(), 1);
        assert_eq!(offers[0].document_id, fresh);
        assert!(
            !autosave_path(root, expired).exists() && !marker_path(root, expired).exists(),
            "an expired container is swept with its marker"
        );
        assert!(
            autosave_path(root, fresh).exists(),
            "a fresh one is untouched"
        );
    }

    /// The sweep must not fire on an mtime it cannot trust. A container dated in the
    /// FUTURE relative to the clock — a clock jump backwards, a restored backup, a
    /// copy off another machine — yields no age at all from `duration_since`, and
    /// reading that as "old" would delete live work.
    #[test]
    fn a_future_dated_container_is_never_swept() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let id = DocumentId(Uuid::from_u128(0xF117));
        std::fs::create_dir_all(autosave_dir(root)).unwrap();
        std::fs::write(autosave_path(root, id), b"x").unwrap();
        // A clock a year BEHIND the file puts its mtime in the future.
        let now = SystemTime::now() - Duration::from_secs(365 * 24 * 3600);

        let offers = scan_recoverable(root, |_| false, now).unwrap();
        assert_eq!(offers.len(), 1, "still offered");
        assert!(autosave_path(root, id).exists(), "and not deleted");
    }

    #[test]
    fn scan_skips_foreign_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(autosave_dir(root)).unwrap();
        std::fs::write(autosave_dir(root).join("junk.session.json"), b"{not json").unwrap();
        std::fs::write(autosave_dir(root).join("readme.txt"), b"hi").unwrap();
        // Right extension, but the stem is not a document id.
        std::fs::write(autosave_dir(root).join("notauuid.onecad"), b"x").unwrap();
        let offers = scan_recoverable(root, |_| false, SystemTime::now()).unwrap();
        assert!(offers.is_empty());
    }

    /// An old marker (written before the `title` field existed) must still parse —
    /// `scan_recoverable` treats an unparseable marker as absent, so a strict field
    /// would silently downgrade every pre-upgrade offer to an unlabelled orphan.
    #[test]
    fn a_marker_without_a_title_still_parses() {
        let json = br#"{"documentId":"00000000-0000-0000-0000-0000000000ff","pid":7}"#;
        let parsed: SessionMarker = serde_json::from_slice(json).unwrap();
        assert_eq!(parsed.pid, 7);
        assert!(parsed.title.is_none());
        assert!(parsed.opened_path.is_none());
    }
}
