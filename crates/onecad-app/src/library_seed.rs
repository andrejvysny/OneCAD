//! First-run seeding of the built-in component packages (Component Library
//! WP-A2, spec §6.2's seed catalog).
//!
//! **Why this exists.** The library root is `<app_data_dir>/library`, which is
//! empty on a fresh install — so until WP-A2 a real user opened the library
//! panel and saw nothing at all, and spec §12's flagship flow ("search M8
//! socket head, drag it onto a hole") was unreachable in the shipped app no
//! matter what the worker could generate. The catalog has to arrive with the
//! application.
//!
//! **Why the manifests are EMBEDDED (`include_str!`) rather than bundled
//! resources.** A generator package is one small text file: no blob, no
//! thumbnail. Embedding makes seeding behave identically in `tauri dev`, in a
//! packaged bundle, in `cargo test`, and in CI — none of which resolve a
//! resource directory the same way — and removes a packaging step that can
//! silently ship an app with an empty library. When a package needs binary
//! content (a `preview.webp`), `include_bytes!` extends this the same way.
//!
//! **The one rule that matters: the user's copy always wins.** Seeding only
//! ever CREATES a package directory that does not exist. It never overwrites
//! and never merges — a package the user edited, or authored themselves under
//! the same id, is left exactly as it is. A `.seed-version` marker records that
//! the pass ran, so deleting a built-in package keeps it deleted; bumping
//! [`SEED_VERSION`] (adding a family, correcting a manifest) re-runs the pass,
//! which restores anything missing.
//!
//! **The one exception, and why it is not one: pruning a RETIRED built-in.**
//! When a family leaves [`SEED_PACKAGES`], the directory it left behind on
//! every already-seeded root becomes dead weight — the worker no longer
//! registers its generator, so it can never preview and can never place. The
//! pass removes such a directory ONLY when it can PROVE it wrote it and the
//! user never touched it: the `.seed-ledger.json` written at install time
//! records the SHA-256 of the manifest this app put there, and prune requires
//! the id to be ledgered, the directory to hold nothing but that one file, and
//! the file to still hash to the ledgered value. Anything else — an edited
//! manifest, an added preview, a package the user authored under the same id —
//! fails a check, is left untouched, and is DROPPED FROM THE LEDGER, because
//! at that point it is theirs and this module has no further claim on it.
//!
//! So the rule is unchanged. Seeding still never deletes anything a user
//! wrote; it only takes back what it demonstrably put there and no longer
//! ships.
//!
//! **Project templates seed in the same pass** (WP-F2b, spec §8) under the same
//! marker and the same "the user's copy wins" rule — see
//! [`crate::library_seed_templates`], which owns the starters because their
//! content is a generated `.onecad` document rather than an embedded string.
//! Templates are NOT pruned: their bytes are generated per install (a fresh
//! `DocumentId` every time), so there is no "still unmodified" proof to check,
//! and without one a prune would be the guess this design exists to avoid.

use std::collections::BTreeMap;
use std::path::Path;

use onecad_library::package::COMPONENT_MANIFEST_FILE;
use sha2::{Digest, Sha256};

/// Bumped whenever the shipped set of packages changes (a new family, a
/// corrected manifest, a family retired). A root whose marker is missing or
/// lower re-runs the seeding pass; one at this version is left alone.
///
/// **A bump installs what is missing and prunes what is retired** — see the
/// module header for the proof prune requires. The one population it cannot
/// help is a root seeded BEFORE the ledger existed (marker ≤ 4): there is no
/// record of what this app wrote there, so nothing can be proven and nothing is
/// removed. Those roots need a manual `rm` of the dead directory, once.
pub const SEED_VERSION: u32 = 5;

const SEED_MARKER_FILE: &str = ".seed-version";

/// Records what THIS app installed, so a later version can tell its own
/// leftovers from the user's files. See the module header.
const SEED_LEDGER_FILE: &str = ".seed-ledger.json";

/// One shipped package: its directory name (which is also its component id)
/// and its `component.toml` text.
pub struct SeedPackage {
    pub id: &'static str,
    pub manifest: &'static str,
}

macro_rules! seed {
    ($id:literal) => {
        SeedPackage {
            id: $id,
            manifest: include_str!(concat!(
                "../resources/library-seed/",
                $id,
                "/component.toml"
            )),
        }
    };
}

/// The built-in catalog. Every entry is a `generator` package, and its id must
/// be one the worker's `build_component` dispatch actually registers —
/// `known_generator_ids_covers_the_seed_catalog` (ctest) is the pairing from
/// the worker's side, `every_seed_package_parses_and_validates` the one from
/// here.
///
/// **The catalog is deliberately ONE family.** SEED_VERSION 4 dropped the six
/// other fastener families and the three machine elements (bearing, two NEMA
/// frames) that spec §6.2 sketched, along with their worker generators and
/// dimension tables. The socket cap screw is the flagship §12 flow ("search M8
/// socket head, drag it onto a hole") and the only family with pinned
/// exact-volume ctests; the rest were breadth without a consumer. Re-adding one
/// means a manifest here, a generator + table in the worker, and a
/// `SEED_VERSION` bump — not a revert.
pub const SEED_PACKAGES: &[SeedPackage] = &[seed!("onecad.std.iso4762")];

/// What one seeding pass did. `installed` names packages written now;
/// `kept` names shipped packages already present on disk and therefore left
/// untouched. The `templates_*` pair is the same split for the project
/// templates (WP-F2b), reported separately because a template is not an
/// [`IndexEntry`] and never enters the component index.
///
/// `pruned` names retired built-ins this pass removed after proving it wrote
/// them; `adopted` names retired built-ins it left alone because the proof
/// failed — the user has edited or replaced them, so they are the user's now
/// and the ledger forgets them. Both are reported rather than silent: a pass
/// that deletes from the user's library must say what it deleted.
///
/// [`IndexEntry`]: onecad_library::index::IndexEntry
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SeedOutcome {
    pub installed: Vec<String>,
    pub kept: Vec<String>,
    pub pruned: Vec<String>,
    pub adopted: Vec<String>,
    pub templates_installed: Vec<String>,
    pub templates_kept: Vec<String>,
    /// `false` when the marker already said this version — nothing was even
    /// examined.
    pub ran: bool,
}

/// Installs any missing built-in package under `root`.
///
/// Best-effort by design: this runs at application start, and a library that
/// cannot be seeded (read-only volume, permission denied) must degrade to an
/// empty panel, never block startup. Errors surface to the caller, which logs
/// them.
///
/// # Errors
/// I/O failure creating the root, writing a manifest, or writing the marker.
pub fn seed_library(root: &Path) -> std::io::Result<SeedOutcome> {
    if marker_version(root) >= Some(SEED_VERSION) {
        return Ok(SeedOutcome::default());
    }

    let mut outcome = SeedOutcome {
        ran: true,
        ..Default::default()
    };
    std::fs::create_dir_all(root)?;
    let mut ledger = read_ledger(root);
    for pkg in SEED_PACKAGES {
        let dir = root.join(pkg.id);
        let digest = manifest_digest(pkg.manifest.as_bytes());
        if dir.exists() {
            // The user's copy wins — including a package they authored under
            // this id, which is why the check is on the DIRECTORY and not on
            // the manifest's contents.
            outcome.kept.push(pkg.id.to_string());
            // …but if what is there is byte-identical to what this build
            // ships, CLAIM it in the ledger. Without this, a root seeded
            // before the ledger existed (or by an older build) never ledgers
            // anything it already has, so retiring this family later could
            // never prune it — the gap would persist for the entire life of
            // the install. The evidence standard is the prune's own: same one
            // file, same bytes.
            if is_untouched(&dir, &digest) {
                ledger.insert(pkg.id.to_string(), digest);
            }
            continue;
        }
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(COMPONENT_MANIFEST_FILE), pkg.manifest)?;
        // Ledger the EXACT bytes written, which is what a later prune has to
        // match. Recording the id alone would prove nothing.
        ledger.insert(pkg.id.to_string(), digest);
        outcome.installed.push(pkg.id.to_string());
    }
    prune_retired(root, &mut ledger, &mut outcome)?;
    // Templates share the marker: "has this root been seeded" must stay ONE
    // question, or a half-seeded root becomes a state nothing tests.
    crate::library_seed_templates::install_missing(
        root,
        &mut outcome.templates_installed,
        &mut outcome.templates_kept,
    )?;
    write_ledger(root, &ledger)?;
    std::fs::write(root.join(SEED_MARKER_FILE), SEED_VERSION.to_string())?;
    Ok(outcome)
}

/// Removes the leftovers of families this build no longer ships, but ONLY the
/// ones it can prove it wrote — see the module header for why each check is
/// there. A ledgered id that fails any check is adopted by the user: dropped
/// from the ledger, left on disk, never reconsidered.
///
/// A prune failure is never fatal. This runs at application start, and a
/// directory that cannot be removed (permissions, a file held open) must leave
/// the user with a stale card, not a library that refused to open.
fn prune_retired(
    root: &Path,
    ledger: &mut BTreeMap<String, String>,
    outcome: &mut SeedOutcome,
) -> std::io::Result<()> {
    let retired: Vec<String> = ledger
        .keys()
        .filter(|id| !SEED_PACKAGES.iter().any(|p| p.id == id.as_str()))
        .cloned()
        .collect();

    for id in retired {
        let dir = root.join(&id);
        if !dir.exists() {
            // Already gone — the user deleted it, or a previous pass pruned it.
            ledger.remove(&id);
            continue;
        }
        if is_untouched(&dir, &ledger[&id]) && std::fs::remove_dir_all(&dir).is_ok() {
            ledger.remove(&id);
            outcome.pruned.push(id);
        } else {
            ledger.remove(&id);
            outcome.adopted.push(id);
        }
    }
    Ok(())
}

/// Whether `dir` still holds exactly the one manifest this app wrote there.
///
/// Both halves matter. The digest alone would let an ADDED file (a preview the
/// user dropped in, a `source.onecad`) be deleted along with the directory; the
/// file count alone would let an EDITED manifest be deleted. A retired package
/// is only reclaimable when neither has happened.
fn is_untouched(dir: &Path, expected_digest: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    let mut names: Vec<std::ffi::OsString> = Vec::new();
    for entry in entries.flatten() {
        names.push(entry.file_name());
    }
    if names.len() != 1 || names[0] != COMPONENT_MANIFEST_FILE {
        return false;
    }
    let Ok(bytes) = std::fs::read(dir.join(COMPONENT_MANIFEST_FILE)) else {
        return false;
    };
    manifest_digest(&bytes) == expected_digest
}

fn manifest_digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

/// The ledger, or an empty map when absent/unreadable/malformed. A ledger that
/// cannot be read means "nothing is proven", which prunes nothing — the safe
/// direction, and the same one a pre-ledger root lands in.
fn read_ledger(root: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(root.join(SEED_LEDGER_FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_ledger(root: &Path, ledger: &BTreeMap<String, String>) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(ledger).map_err(std::io::Error::other)?;
    std::fs::write(root.join(SEED_LEDGER_FILE), text)
}

/// The marker's version, or `None` when absent/unreadable/malformed — all of
/// which mean "seed", since re-running the pass is idempotent.
fn marker_version(root: &Path) -> Option<u32> {
    std::fs::read_to_string(root.join(SEED_MARKER_FILE))
        .ok()?
        .trim()
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use onecad_library::package;
    use std::path::PathBuf;

    fn manifest_path(id: &str) -> PathBuf {
        PathBuf::from(id).join(COMPONENT_MANIFEST_FILE)
    }

    #[test]
    fn every_seed_package_parses_and_validates() {
        for pkg in SEED_PACKAGES {
            let parsed = package::parse(pkg.manifest, &manifest_path(pkg.id))
                .unwrap_or_else(|e| panic!("{} failed to parse: {e:?}", pkg.id));
            package::validate_identity(&parsed)
                .unwrap_or_else(|e| panic!("{} failed identity validation: {e:?}", pkg.id));
            assert_eq!(
                parsed.identity.id, pkg.id,
                "the package directory name IS the component id (the index keys on it)"
            );
            assert!(
                !parsed.metadata.name.trim().is_empty(),
                "{}: a nameless package is unsearchable",
                pkg.id
            );
            assert!(
                !parsed.attachments.is_empty(),
                "{}: a component with no attachment can never snap (spec §5.3)",
                pkg.id
            );
        }
    }

    // Every seed package declares the content hash of a package carrying no
    // file but its own manifest — the SHA-256 of nothing. If a package later
    // gains a `preview.webp`, this fails and the manifest must be updated with
    // the real hash rather than shipping a revision that lies (which would
    // make every placement of it resolve to `NeedsRepair`).
    #[test]
    fn declared_revisions_match_a_manifest_only_package() {
        let dir = tempfile::tempdir().unwrap();
        let outcome = seed_library(dir.path()).unwrap();
        assert_eq!(outcome.installed.len(), SEED_PACKAGES.len());
        for pkg in SEED_PACKAGES {
            let computed = package::compute_revision(&dir.path().join(pkg.id)).unwrap();
            let declared = package::parse(pkg.manifest, &manifest_path(pkg.id))
                .unwrap()
                .identity
                .revision;
            assert_eq!(computed, declared, "{}: declared revision is stale", pkg.id);
        }
    }

    #[test]
    fn seeding_a_fresh_root_installs_every_package() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("library");
        let outcome = seed_library(&root).unwrap();
        assert!(outcome.ran);
        assert_eq!(outcome.installed.len(), SEED_PACKAGES.len());
        assert!(outcome.kept.is_empty());
        for pkg in SEED_PACKAGES {
            assert!(root.join(pkg.id).join(COMPONENT_MANIFEST_FILE).is_file());
        }
    }

    // WP-F2b: the same pass installs the project starters, and they list back
    // through the API the start screen actually calls.
    #[test]
    fn seeding_a_fresh_root_installs_every_template() {
        use crate::library_seed_templates::SEED_TEMPLATES;
        let dir = tempfile::tempdir().unwrap();
        let outcome = seed_library(dir.path()).unwrap();
        assert_eq!(outcome.templates_installed.len(), SEED_TEMPLATES.len());
        assert!(outcome.templates_kept.is_empty());
        let listed = onecad_library::template::list(dir.path());
        assert_eq!(listed.len(), SEED_TEMPLATES.len());
        for tpl in SEED_TEMPLATES {
            let entry = listed
                .iter()
                .find(|e| e.id == tpl.id)
                .unwrap_or_else(|| panic!("{} not listed", tpl.id));
            assert_eq!(entry.name, tpl.name);
            assert_eq!(entry.description.as_deref(), Some(tpl.description));
            assert!(entry.document_path.is_file());
        }
    }

    // Templates seed under the SAME marker as the packages: a root already at
    // this version is not re-examined for either.
    #[test]
    fn a_second_pass_installs_no_template_either() {
        let dir = tempfile::tempdir().unwrap();
        seed_library(dir.path()).unwrap();
        std::fs::remove_dir_all(dir.path().join(onecad_library::template::TEMPLATES_DIR)).unwrap();
        let again = seed_library(dir.path()).unwrap();
        assert!(again.templates_installed.is_empty());
        assert!(
            onecad_library::template::list(dir.path()).is_empty(),
            "user deletion is respected until SEED_VERSION moves"
        );
    }

    #[test]
    fn a_second_pass_at_the_same_version_does_nothing() {
        let dir = tempfile::tempdir().unwrap();
        seed_library(dir.path()).unwrap();
        let again = seed_library(dir.path()).unwrap();
        assert!(!again.ran, "the marker short-circuits the whole pass");
        assert!(again.installed.is_empty());
    }

    // The rule the whole module exists to protect: a package the user edited
    // (or authored under a built-in id) survives a re-seed byte for byte.
    #[test]
    fn a_user_modified_package_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let id = SEED_PACKAGES[0].id;
        let pkg_dir = root.join(id);
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join(COMPONENT_MANIFEST_FILE), "# mine\n").unwrap();

        let outcome = seed_library(root).unwrap();
        assert!(outcome.kept.contains(&id.to_string()));
        assert!(!outcome.installed.contains(&id.to_string()));
        assert_eq!(
            std::fs::read_to_string(pkg_dir.join(COMPONENT_MANIFEST_FILE)).unwrap(),
            "# mine\n"
        );
        // …and every OTHER shipped package still arrived (none, while the
        // catalog is one family — the assertion is written against the list so
        // it keeps meaning if a family is ever re-added).
        assert_eq!(outcome.installed.len(), SEED_PACKAGES.len() - 1);
    }

    // Deleting a built-in keeps it deleted: the marker is already current, so
    // the next pass never looks. Only a SEED_VERSION bump restores it.
    #[test]
    fn a_deleted_package_stays_deleted_until_the_seed_version_moves() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        seed_library(root).unwrap();
        let id = SEED_PACKAGES[0].id;
        std::fs::remove_dir_all(root.join(id)).unwrap();

        seed_library(root).unwrap();
        assert!(!root.join(id).exists(), "user deletion is respected");

        // Simulate the bump: an older marker re-runs the pass.
        std::fs::write(root.join(SEED_MARKER_FILE), "0").unwrap();
        let outcome = seed_library(root).unwrap();
        assert!(outcome.installed.contains(&id.to_string()));
    }

    // Seeded packages must be indexable end to end — the panel reads the
    // INDEX, not the directory, so a package that seeds but does not index is
    // still an empty panel.
    #[test]
    fn seeded_packages_reindex_into_a_full_catalog() {
        let dir = tempfile::tempdir().unwrap();
        seed_library(dir.path()).unwrap();
        let mut library = onecad_library::Library::open(dir.path()).unwrap();
        let report = library.reindex().unwrap();
        assert_eq!(
            report.indexed,
            SEED_PACKAGES.len(),
            "skipped: {:?}",
            report.skipped
        );
        assert!(report.skipped.is_empty());
        for pkg in SEED_PACKAGES {
            assert!(
                library.get(pkg.id, None).is_some(),
                "{} not indexed",
                pkg.id
            );
        }
    }

    // ── Retiring a family: the ledger and its prune ──────────────────────────
    //
    // These simulate a family LEAVING `SEED_PACKAGES`, which is the only way to
    // reach the prune. `retire` writes the package plus the ledger entry a real
    // install would have left, then the pass runs with that id no longer
    // shipped — exactly the state a user upgrading across the drop is in.

    const RETIRED_ID: &str = "onecad.std.retired-family";
    const RETIRED_MANIFEST: &str = "# a family this build no longer ships\n";

    /// Writes a retired package and the ledger a real install of it would have
    /// left behind. Returns its directory.
    fn retire(root: &Path, ledger_digest: Option<&str>) -> PathBuf {
        std::fs::create_dir_all(root).unwrap();
        let dir = root.join(RETIRED_ID);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(COMPONENT_MANIFEST_FILE), RETIRED_MANIFEST).unwrap();
        if let Some(digest) = ledger_digest {
            let mut ledger = read_ledger(root);
            ledger.insert(RETIRED_ID.to_string(), digest.to_string());
            write_ledger(root, &ledger).unwrap();
        }
        dir
    }

    fn shipped_digest() -> String {
        manifest_digest(RETIRED_MANIFEST.as_bytes())
    }

    // The case that motivated all of this: a root seeded before the drop shows
    // dead cards for a family whose generator no longer exists. The pass takes
    // back exactly what it put there.
    #[test]
    fn a_retired_package_this_app_installed_is_pruned() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let pkg_dir = retire(root, Some(&shipped_digest()));

        let outcome = seed_library(root).unwrap();
        assert_eq!(outcome.pruned, vec![RETIRED_ID.to_string()]);
        assert!(outcome.adopted.is_empty());
        assert!(!pkg_dir.exists(), "the dead directory is gone");
        assert!(
            !read_ledger(root).contains_key(RETIRED_ID),
            "and the ledger no longer claims it"
        );
        // The shipped family is untouched by any of it.
        assert!(root.join(SEED_PACKAGES[0].id).is_dir());
    }

    // The safety property, stated as its own test: an EDITED manifest fails the
    // digest check, so the package is left alone and the ledger gives up its
    // claim. Deleting here would be destroying the user's work.
    #[test]
    fn a_retired_package_the_user_edited_is_adopted_not_pruned() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let pkg_dir = retire(root, Some(&shipped_digest()));
        std::fs::write(pkg_dir.join(COMPONENT_MANIFEST_FILE), "# mine now\n").unwrap();

        let outcome = seed_library(root).unwrap();
        assert_eq!(outcome.adopted, vec![RETIRED_ID.to_string()]);
        assert!(outcome.pruned.is_empty());
        assert_eq!(
            std::fs::read_to_string(pkg_dir.join(COMPONENT_MANIFEST_FILE)).unwrap(),
            "# mine now\n",
            "byte for byte"
        );
        assert!(!read_ledger(root).contains_key(RETIRED_ID));
    }

    // The other half of `is_untouched`, and the reason the file COUNT is
    // checked and not just the digest: an added file means the user built on
    // top of this package, and `remove_dir_all` would take that with it.
    #[test]
    fn a_retired_package_the_user_added_a_file_to_is_adopted_not_pruned() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let pkg_dir = retire(root, Some(&shipped_digest()));
        std::fs::write(pkg_dir.join("preview.png"), b"mine").unwrap();

        let outcome = seed_library(root).unwrap();
        assert_eq!(outcome.adopted, vec![RETIRED_ID.to_string()]);
        assert!(
            pkg_dir.join("preview.png").is_file(),
            "the added file lives"
        );
        assert!(pkg_dir.join(COMPONENT_MANIFEST_FILE).is_file());
    }

    // A package this app never installed is invisible to the prune, however
    // dead it looks. Without a ledger entry there is no proof, and no proof
    // means no deletion — which is also every pre-ledger root's behaviour.
    #[test]
    fn an_unledgered_package_is_never_pruned() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let pkg_dir = retire(root, None);

        let outcome = seed_library(root).unwrap();
        assert!(outcome.pruned.is_empty());
        assert!(outcome.adopted.is_empty(), "never even considered");
        assert!(pkg_dir.is_dir());
    }

    // A user who deleted a retired package first gets no error and no ghost
    // entry: the ledger simply drops the claim.
    #[test]
    fn a_retired_package_already_gone_just_leaves_the_ledger() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let pkg_dir = retire(root, Some(&shipped_digest()));
        std::fs::remove_dir_all(&pkg_dir).unwrap();

        let outcome = seed_library(root).unwrap();
        assert!(outcome.pruned.is_empty());
        assert!(outcome.adopted.is_empty());
        assert!(!read_ledger(root).contains_key(RETIRED_ID));
    }

    // A root that already HAS a shipped package (seeded by an older build, or
    // before the ledger existed) still ends up ledgered, so retiring that
    // family later can prune it. Without this the gap would never close for an
    // existing install.
    #[test]
    fn an_already_present_shipped_package_is_claimed_into_the_ledger() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let pkg = &SEED_PACKAGES[0];
        let pkg_dir = root.join(pkg.id);
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join(COMPONENT_MANIFEST_FILE), pkg.manifest).unwrap();
        assert!(read_ledger(root).is_empty(), "no ledger to start with");

        let outcome = seed_library(root).unwrap();
        assert!(outcome.kept.contains(&pkg.id.to_string()));
        assert_eq!(
            read_ledger(root).get(pkg.id),
            Some(&manifest_digest(pkg.manifest.as_bytes())),
            "an identical copy is claimable on the prune's own evidence standard"
        );
    }

    // …but only an IDENTICAL copy. A package the user wrote under a shipped id
    // is never claimed, so it can never later be pruned.
    #[test]
    fn a_user_authored_package_under_a_shipped_id_is_never_claimed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let id = SEED_PACKAGES[0].id;
        let pkg_dir = root.join(id);
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join(COMPONENT_MANIFEST_FILE), "# mine\n").unwrap();

        seed_library(root).unwrap();
        assert!(
            !read_ledger(root).contains_key(id),
            "this app has no claim on a file it did not write"
        );
    }

    // A SHIPPED package is never a prune candidate, even though the ledger
    // carries it — the retired set is "ledgered AND not in SEED_PACKAGES", and
    // getting that backwards would empty the catalog on every start.
    #[test]
    fn a_still_shipped_package_is_never_pruned() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        seed_library(root).unwrap();
        let id = SEED_PACKAGES[0].id;
        assert!(read_ledger(root).contains_key(id), "install ledgered it");

        std::fs::write(root.join(SEED_MARKER_FILE), "0").unwrap();
        let again = seed_library(root).unwrap();
        assert!(again.pruned.is_empty());
        assert!(again.adopted.is_empty());
        assert!(root.join(id).is_dir());
    }
}
