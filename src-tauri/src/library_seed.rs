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
//! ever CREATES a package directory that does not exist. It never overwrites,
//! never merges, and never deletes — a package the user edited, or authored
//! themselves under the same id, is left exactly as it is. A `.seed-version`
//! marker records that the pass ran, so deleting a built-in package keeps it
//! deleted; bumping [`SEED_VERSION`] (adding a family, correcting a manifest)
//! re-runs the pass, which restores anything missing.

use std::path::Path;

use onecad_library::package::COMPONENT_MANIFEST_FILE;

/// Bumped whenever the shipped set of packages changes (a new family, a
/// corrected manifest). A root whose marker is missing or lower re-runs the
/// seeding pass; one at this version is left alone.
pub const SEED_VERSION: u32 = 1;

const SEED_MARKER_FILE: &str = ".seed-version";

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

/// The built-in catalog (spec §6.2). Every entry is a `generator` package —
/// one per registered worker generator id, which is the pairing
/// `seed_ids_match_the_worker_generators` in the worker's own test suite and
/// `every_seed_package_parses_and_validates` here keep honest.
pub const SEED_PACKAGES: &[SeedPackage] = &[
    seed!("onecad.std.iso4762"),
    seed!("onecad.std.iso7380"),
    seed!("onecad.std.iso4014"),
    seed!("onecad.std.iso4017"),
    seed!("onecad.std.iso4032"),
    seed!("onecad.std.iso7089"),
    seed!("onecad.std.iso7093"),
];

/// What one seeding pass did. `installed` names packages written now;
/// `kept` names shipped packages already present on disk and therefore left
/// untouched.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SeedOutcome {
    pub installed: Vec<String>,
    pub kept: Vec<String>,
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
    for pkg in SEED_PACKAGES {
        let dir = root.join(pkg.id);
        if dir.exists() {
            // The user's copy wins — including a package they authored under
            // this id, which is why the check is on the DIRECTORY and not on
            // the manifest's contents.
            outcome.kept.push(pkg.id.to_string());
            continue;
        }
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(COMPONENT_MANIFEST_FILE), pkg.manifest)?;
        outcome.installed.push(pkg.id.to_string());
    }
    std::fs::write(root.join(SEED_MARKER_FILE), SEED_VERSION.to_string())?;
    Ok(outcome)
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
        // …and the rest still arrived.
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
}
