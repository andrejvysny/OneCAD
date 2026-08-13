//! First-run seeding of the built-in project templates (Component Library
//! WP-F2b, spec §8's "ship 3–5 honest starters").
//!
//! **Why the documents are GENERATED rather than embedded.** A component seed
//! is one text file, so `include_str!` is the whole story; a template is a
//! frozen `.onecad` container — a ZIP whose manifest carries per-section
//! hashes and the ops hash. Checking in three binaries would mean re-freezing
//! them by hand every time the document schema, the container layout or a
//! record shape moves, and a stale one fails to open with no test able to see
//! it coming. Building the document here with `onecad-core` types and writing
//! it through the SAME [`ContainerWriter`] a real save uses means a schema
//! change either compiles or does not, and the seeded starter is by
//! construction whatever this build can open.
//!
//! **What the three starters actually contain** — this list is the honest
//! inventory, not a wish:
//!
//! * `blank` — an empty document in millimetres. Nothing else; that IS the
//!   starter.
//! * `printed-part` — an empty document plus one named `OffsetFromPlane` datum
//!   on XY at 0, "Build plate". Real, selectable content (it shows in the
//!   sketch-plane picker beside the world planes), and deliberately the ONLY
//!   thing here: there is no print-settings machinery in the document model to
//!   encode a nozzle, a layer height or a bed size, and inventing an
//!   unreferenced variable table would be decoration, not a starter.
//! * `nema17-mount` — an empty document plus one `PlaceComponent` record for
//!   the seeded `onecad.std.nema17` package at the origin, no mate. The motor
//!   arrives when the document is opened and regenerated, exactly like any
//!   other placed instance; nothing here needs a worker, because a record is
//!   authored, not executed.
//!
//! **The seeding rule is the component rule, verbatim: the user's copy always
//! wins.** A template directory that already exists is left untouched, and
//! deleting a built-in keeps it deleted until [`SEED_VERSION`] moves.
//!
//! [`SEED_VERSION`]: crate::library_seed::SEED_VERSION

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use onecad_core::document::datum::DatumPlane;
use onecad_core::document::record::{
    ComponentSourceRef, FrozenPlacement, KnownOperation, Operation, OperationRecord,
    PlaceComponentParams,
};
use onecad_core::document::variables::Scalar;
use onecad_core::document::Document;
use onecad_core::edit::{DocumentSession, EditCommand};
use onecad_core::ids::{DatumPlaneId, DocumentId, RecordId};
use onecad_core::io::container::{ContainerCaches, ContainerWriter};

use onecad_library::template::{self, NewTemplate, TEMPLATES_DIR};

/// The component whose instance the motor-mount starter opens with. Its
/// version and package revision are read from the seed manifest rather than
/// spelled here, so the starter can never claim a revision the shipped package
/// does not have.
const MOUNT_COMPONENT_ID: &str = "onecad.std.nema17";
/// The worker generator `MOUNT_COMPONENT_ID`'s package resolves to (its
/// `[source]` block). Kept in lockstep with that manifest by
/// `nema17_template_matches_the_shipped_package`.
const MOUNT_GENERATOR_ID: &str = "nema17";
const MOUNT_GENERATOR_VERSION: u32 = 1;

/// One shipped starter: the manifest metadata plus the builder for its frozen
/// document.
pub struct SeedTemplate {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    build: fn() -> Result<Document, String>,
}

/// The built-in starters (spec §8). Ids are namespaced like a component's for
/// the same reason — a user-authored template must be able to sit beside these
/// without colliding.
pub const SEED_TEMPLATES: &[SeedTemplate] = &[
    SeedTemplate {
        id: "onecad.std.template.blank",
        name: "Blank",
        description: "An empty document in millimetres.",
        build: blank_document,
    },
    SeedTemplate {
        id: "onecad.std.template.printed-part",
        name: "3D-Printed Part",
        description: "Millimetres, with a Build plate datum on XY to sketch the footprint on.",
        build: printed_part_document,
    },
    SeedTemplate {
        id: "onecad.std.template.nema17-mount",
        name: "NEMA 17 Motor Mount",
        description: "A NEMA 17 stepper placed at the origin — model the mounting plate around it.",
        build: nema17_mount_document,
    },
];

/// Installs any missing built-in template under `root`, appending what it did
/// to `installed` / `kept`.
///
/// Called from [`crate::library_seed::seed_library`] inside the same
/// `SEED_VERSION` gate, so the marker semantics are shared and there is exactly
/// one "has this root been seeded" question.
///
/// # Errors
/// I/O failure building or writing a template. Best-effort at the call site:
/// the caller logs and leaves the library as found.
pub fn install_missing(
    root: &Path,
    installed: &mut Vec<String>,
    kept: &mut Vec<String>,
) -> std::io::Result<()> {
    let scratch = scratch_dir();
    let mut wrote_scratch = false;
    for tpl in SEED_TEMPLATES {
        if root.join(TEMPLATES_DIR).join(tpl.id).exists() {
            // The user's copy wins — the DIRECTORY is the check, exactly as it
            // is for a component package.
            kept.push(tpl.id.to_string());
            continue;
        }
        if !wrote_scratch {
            std::fs::create_dir_all(&scratch)?;
            wrote_scratch = true;
        }
        let document = freeze(&scratch, tpl)?;
        template::save(
            root,
            NewTemplate {
                id: tpl.id.to_string(),
                name: tpl.name.to_string(),
                description: Some(tpl.description.to_string()),
                document,
                preview_png: None,
            },
        )
        .map_err(|e| std::io::Error::other(format!("seed template {}: {e:?}", tpl.id)))?;
        installed.push(tpl.id.to_string());
    }
    if wrote_scratch {
        let _ = std::fs::remove_dir_all(&scratch);
    }
    Ok(())
}

/// A process- and call-unique scratch directory. The counter matters: unit
/// tests seed several roots concurrently in one process, and a shared scratch
/// path would let them overwrite each other's container mid-write.
fn scratch_dir() -> std::path::PathBuf {
    static NONCE: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "onecad-seed-templates-{}-{}",
        std::process::id(),
        NONCE.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Builds a starter's document and returns its frozen container bytes.
fn freeze(scratch: &Path, tpl: &SeedTemplate) -> std::io::Result<Vec<u8>> {
    let document = (tpl.build)()
        .map_err(|e| std::io::Error::other(format!("build template {}: {e}", tpl.id)))?;
    let path = scratch.join(format!("{}.onecad", tpl.id));
    // No caches: a starter has never been regenerated, so there is no mesh and
    // no preview to carry — the from-0 regen at open produces both.
    ContainerWriter::save(
        &path,
        &document,
        &ContainerCaches::none(),
        &crate::api::save_meta(),
    )
    .map_err(|e| std::io::Error::other(format!("freeze template {}: {e}", tpl.id)))?;
    std::fs::read(&path)
}

// ── The documents ────────────────────────────────────────────────────────────

fn blank_document() -> Result<Document, String> {
    Ok(Document::new(DocumentId::new()))
}

fn printed_part_document() -> Result<Document, String> {
    let mut session = DocumentSession::new(Document::new(DocumentId::new()));
    session
        .apply(EditCommand::AddDatumPlane {
            datum: DatumPlane::offset_from_plane(DatumPlaneId::new(), "Build plate", "XY", 0.0),
        })
        .map_err(|e| format!("{e:?}"))?;
    Ok(session.into_document())
}

fn nema17_mount_document() -> Result<Document, String> {
    let (version, revision) = mount_component_identity()?;
    let params = PlaceComponentParams {
        component_id: MOUNT_COMPONENT_ID.to_string(),
        component_version: version,
        component_revision: revision,
        // No overrides: the generator's own default body length is the honest
        // starting point, and the user edits it on the instance.
        params: Default::default(),
        source: ComponentSourceRef::Generator {
            generator_id: MOUNT_GENERATOR_ID.to_string(),
            generator_version: MOUNT_GENERATOR_VERSION,
            params: Default::default(),
            extra: Default::default(),
        },
        // No mate: there is nothing in a starter to mate TO, and a recorded
        // mate that cannot resolve is exactly the NeedsRepair this project
        // exists to avoid.
        mate: None,
        placement: FrozenPlacement {
            translate: [Scalar::new(0.0), Scalar::new(0.0), Scalar::new(0.0)],
            rotate: Default::default(),
        },
        extra: Default::default(),
    };
    let record = OperationRecord::new(
        RecordId::new(),
        0,
        "NEMA 17 Stepper Motor",
        Operation::Known(KnownOperation::PlaceComponent(params)),
    );
    let mut session = DocumentSession::new(Document::new(DocumentId::new()));
    session
        .apply(EditCommand::AddOperation {
            record,
            at_cursor: false,
        })
        .map_err(|e| format!("{e:?}"))?;
    Ok(session.into_document())
}

/// The shipped NEMA 17 package's `(version, revision)`, read out of the seed
/// manifest that will be installed alongside this template.
fn mount_component_identity() -> Result<(String, String), String> {
    let pkg = crate::library_seed::SEED_PACKAGES
        .iter()
        .find(|p| p.id == MOUNT_COMPONENT_ID)
        .ok_or_else(|| format!("{MOUNT_COMPONENT_ID} is not a seeded package"))?;
    let parsed = onecad_library::package::parse(
        pkg.manifest,
        &std::path::PathBuf::from(pkg.id).join(onecad_library::package::COMPONENT_MANIFEST_FILE),
    )
    .map_err(|e| format!("parse {}: {e:?}", pkg.id))?;
    Ok((parsed.identity.version, parsed.identity.revision))
}

#[cfg(test)]
mod tests {
    use super::*;
    use onecad_core::io::container::ContainerReader;

    fn install(root: &Path) -> (Vec<String>, Vec<String>) {
        let (mut installed, mut kept) = (Vec::new(), Vec::new());
        install_missing(root, &mut installed, &mut kept).unwrap();
        (installed, kept)
    }

    /// Every starter must produce a container this build can OPEN — the whole
    /// point of generating them instead of checking bytes in.
    #[test]
    fn every_seeded_template_is_a_readable_container() {
        let dir = tempfile::tempdir().unwrap();
        let (installed, kept) = install(dir.path());
        assert_eq!(installed.len(), SEED_TEMPLATES.len());
        assert!(kept.is_empty());

        let listed = template::list(dir.path());
        assert_eq!(listed.len(), SEED_TEMPLATES.len());
        for entry in &listed {
            let loaded = ContainerReader::open(&entry.document_path)
                .unwrap_or_else(|e| panic!("{} did not open: {e:?}", entry.id));
            assert!(
                !loaded.outcome.read_only,
                "{}: a starter must open editable",
                entry.id
            );
        }
    }

    #[test]
    fn blank_is_actually_empty_and_millimetres() {
        let dir = tempfile::tempdir().unwrap();
        install(dir.path());
        let entry = template::get(dir.path(), "onecad.std.template.blank").unwrap();
        let loaded = ContainerReader::open(&entry.document_path).unwrap();
        let doc = loaded.document();
        assert_eq!(
            doc.settings.units,
            onecad_core::document::variables::Unit::Mm
        );
        assert!(doc.timeline.records().is_empty());
        assert!(doc.datum_planes.is_empty());
        assert!(doc.sketches.is_empty());
    }

    #[test]
    fn printed_part_carries_one_resolved_build_plate_datum() {
        let dir = tempfile::tempdir().unwrap();
        install(dir.path());
        let entry = template::get(dir.path(), "onecad.std.template.printed-part").unwrap();
        let loaded = ContainerReader::open(&entry.document_path).unwrap();
        let doc = loaded.document();
        assert!(
            doc.timeline.records().is_empty(),
            "no geometry in a starter"
        );
        assert_eq!(doc.datum_planes.len(), 1);
        let datum = doc.datum_planes.values().next().unwrap();
        assert_eq!(datum.name, "Build plate");
        assert_eq!(datum.base_plane_id, "XY");
        assert_eq!(datum.offset, 0.0);
        assert!(
            datum.resolved_valid,
            "an unresolved datum cannot host a sketch — the starter would be inert"
        );
    }

    /// The starter's recorded identity must be the package that seeds beside
    /// it: a stale version or revision is the silent-substitution failure
    /// spec §0 invariant 4 forbids.
    #[test]
    fn nema17_template_matches_the_shipped_package() {
        let dir = tempfile::tempdir().unwrap();
        install(dir.path());
        let entry = template::get(dir.path(), "onecad.std.template.nema17-mount").unwrap();
        let loaded = ContainerReader::open(&entry.document_path).unwrap();
        let records = loaded.document().timeline.records();
        assert_eq!(records.len(), 1);
        let Operation::Known(KnownOperation::PlaceComponent(p)) = &records[0].op else {
            panic!("expected a PlaceComponent record, got {:?}", records[0].op);
        };
        p.validate().expect("the seeded record must be valid");
        let (version, revision) = mount_component_identity().unwrap();
        assert_eq!(p.component_id, MOUNT_COMPONENT_ID);
        assert_eq!(p.component_version, version);
        assert_eq!(p.component_revision, revision);
        assert!(p.mate.is_none());
        match &p.source {
            ComponentSourceRef::Generator {
                generator_id,
                generator_version,
                ..
            } => {
                assert_eq!(generator_id, MOUNT_GENERATOR_ID);
                assert_eq!(*generator_version, MOUNT_GENERATOR_VERSION);
            }
            other => panic!("a seeded starter must depend on no blob, got {other:?}"),
        }
    }

    #[test]
    fn an_existing_template_directory_is_kept_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let id = SEED_TEMPLATES[0].id;
        let mine = dir.path().join(TEMPLATES_DIR).join(id);
        std::fs::create_dir_all(&mine).unwrap();
        std::fs::write(mine.join("template.onecad"), b"mine").unwrap();

        let (installed, kept) = install(dir.path());
        assert_eq!(kept, vec![id.to_string()]);
        assert_eq!(installed.len(), SEED_TEMPLATES.len() - 1);
        assert_eq!(
            std::fs::read(mine.join("template.onecad")).unwrap(),
            b"mine"
        );
    }
}
