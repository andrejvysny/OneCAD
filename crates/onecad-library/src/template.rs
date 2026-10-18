//! Project templates (spec §8) — frozen `.onecad` documents stored under
//! `templates/` in the library root.
//!
//! **Same storage mechanics as a component, deliberately a different index.**
//! Spec §8 puts templates in the same root with the same integrity and
//! future-registry story, and this reuses the package discipline (one directory
//! per entry, a small TOML manifest beside the payload, atomic writes). What it
//! does NOT do is put them in `library.json`: a template is not placeable, has
//! no parameters, no attachments and no source kind, and giving it an
//! `IndexEntry` would mean every consumer of the component index has to learn
//! to skip a kind of entry it can never use. Templates are listed by reading
//! the directory — there are a handful of them, and the read happens on the
//! start screen, not in a regen loop.
//!
//! A template is IMMUTABLE by never being the save target: "new from template"
//! copies its bytes into a fresh untitled document (the app crate detaches the
//! runtime from the file it loaded). Nothing here enforces read-only
//! permissions on disk, and nothing should — a user who edits their own
//! template file is not doing something wrong.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{LibraryError, LibraryResult};

pub const TEMPLATES_DIR: &str = "templates";
pub const TEMPLATE_MANIFEST_FILE: &str = "template.toml";
pub const TEMPLATE_DOCUMENT_FILE: &str = "template.onecad";
pub const TEMPLATE_PREVIEW_FILE: &str = "preview.png";

/// `template.toml` — identity and metadata only. The document beside it is the
/// content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemplateManifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// One template on disk, as [`list`] found it.
#[derive(Debug, Clone, PartialEq)]
pub struct TemplateEntry {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// The frozen `.onecad`, absolute.
    pub document_path: PathBuf,
    /// The thumbnail, absolute — `None` when the template ships without one.
    pub preview_path: Option<PathBuf>,
}

/// Input to [`save`].
#[derive(Debug, Clone)]
pub struct NewTemplate {
    /// Directory name and stable id. Not required to be namespaced: a template
    /// is never referenced from a document, so it carries no identity that
    /// could collide with a placed instance's recorded one.
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// The frozen document to instantiate copies from.
    pub document: Vec<u8>,
    pub preview_png: Option<Vec<u8>>,
}

/// Every template under `<root>/templates/`, sorted by name.
///
/// A directory that fails to parse is SKIPPED, never fatal — one malformed
/// template must not empty the start screen's row, the same rule the component
/// reindex follows.
pub fn list(root: &Path) -> Vec<TemplateEntry> {
    let dir = root.join(TEMPLATES_DIR);
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return out; // no templates directory yet is an empty list, not an error
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(path.join(TEMPLATE_MANIFEST_FILE)) else {
            continue;
        };
        let Ok(manifest) = toml::from_str::<TemplateManifest>(&raw) else {
            continue;
        };
        let document_path = path.join(TEMPLATE_DOCUMENT_FILE);
        if !document_path.is_file() {
            continue; // a manifest with no document instantiates nothing
        }
        let preview_path = path.join(TEMPLATE_PREVIEW_FILE);
        out.push(TemplateEntry {
            id: manifest.id,
            name: manifest.name,
            description: manifest.description,
            document_path,
            preview_path: preview_path.is_file().then_some(preview_path),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Resolves one template by id.
pub fn get(root: &Path, id: &str) -> Option<TemplateEntry> {
    list(root).into_iter().find(|t| t.id == id)
}

/// Writes a template. An existing id is REFUSED rather than overwritten — the
/// same rule `save_component` follows, for the weaker but still real reason
/// that silently replacing what a user starts new projects from is not a save,
/// it is a substitution.
///
/// # Errors
/// [`LibraryError::InvalidIdentity`] for an empty/unsafe id, [`LibraryError::Io`]
/// for a taken id or a filesystem failure.
pub fn save(root: &Path, new: NewTemplate) -> LibraryResult<TemplateEntry> {
    let id = new.id.trim().to_string();
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
    {
        return Err(LibraryError::InvalidIdentity(format!(
            "template id `{id}` must be non-empty and use only letters, digits, `-` or `.`"
        )));
    }
    if new.name.trim().is_empty() {
        return Err(LibraryError::InvalidIdentity(
            "a template needs a name".into(),
        ));
    }
    let dir = root.join(TEMPLATES_DIR).join(&id);
    if dir.exists() {
        return Err(LibraryError::Io(format!(
            "template `{id}` already exists — pick another name"
        )));
    }
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(TEMPLATE_DOCUMENT_FILE), &new.document)?;
    if let Some(png) = &new.preview_png {
        std::fs::write(dir.join(TEMPLATE_PREVIEW_FILE), png)?;
    }
    let manifest = TemplateManifest {
        id: id.clone(),
        name: new.name.trim().to_string(),
        description: new.description.clone(),
    };
    let text = toml::to_string_pretty(&manifest)
        .map_err(|e| LibraryError::Io(format!("serialize template manifest: {e}")))?;
    std::fs::write(dir.join(TEMPLATE_MANIFEST_FILE), text)?;

    get(root, &id).ok_or_else(|| LibraryError::Io(format!("template `{id}` did not index back")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_template(id: &str, name: &str) -> NewTemplate {
        NewTemplate {
            id: id.to_string(),
            name: name.to_string(),
            description: Some("A starter".to_string()),
            document: b"frozen onecad container".to_vec(),
            preview_png: Some(b"\x89PNG\r\n".to_vec()),
        }
    }

    #[test]
    fn save_then_list_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let saved = save(dir.path(), new_template("blank", "Blank")).unwrap();
        assert_eq!(saved.name, "Blank");
        assert!(saved.preview_path.is_some());
        assert_eq!(
            std::fs::read(&saved.document_path).unwrap(),
            b"frozen onecad container"
        );

        let listed = list(dir.path());
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0], saved);
        assert_eq!(get(dir.path(), "blank"), Some(saved));
    }

    #[test]
    fn listing_a_root_with_no_templates_is_empty_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(list(dir.path()).is_empty());
        assert!(get(dir.path(), "blank").is_none());
    }

    #[test]
    fn an_existing_id_is_refused_rather_than_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), new_template("blank", "Blank")).unwrap();
        let err = save(dir.path(), new_template("blank", "Blank again")).unwrap_err();
        assert!(format!("{err:?}").contains("already exists"), "{err:?}");
        assert_eq!(list(dir.path())[0].name, "Blank", "the original stands");
    }

    #[test]
    fn a_malformed_template_is_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), new_template("good", "Good")).unwrap();
        // A directory with a manifest but no document instantiates nothing…
        let orphan = dir.path().join(TEMPLATES_DIR).join("orphan");
        std::fs::create_dir_all(&orphan).unwrap();
        std::fs::write(
            orphan.join(TEMPLATE_MANIFEST_FILE),
            "id = \"orphan\"\nname = \"O\"\n",
        )
        .unwrap();
        // …and one with unparsable TOML is skipped too.
        let broken = dir.path().join(TEMPLATES_DIR).join("broken");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join(TEMPLATE_MANIFEST_FILE), "not = toml = at all").unwrap();
        std::fs::write(broken.join(TEMPLATE_DOCUMENT_FILE), b"x").unwrap();

        let listed = list(dir.path());
        assert_eq!(
            listed.len(),
            1,
            "one bad template must not hide the good one"
        );
        assert_eq!(listed[0].id, "good");
    }

    #[test]
    fn an_unsafe_or_nameless_id_is_refused_before_touching_disk() {
        let dir = tempfile::tempdir().unwrap();
        // Path traversal is the one that matters: an id is a directory name.
        assert!(save(dir.path(), new_template("../escape", "Escape")).is_err());
        assert!(save(dir.path(), new_template("", "Empty")).is_err());
        let mut nameless = new_template("ok", "");
        nameless.name = "   ".to_string();
        assert!(save(dir.path(), nameless).is_err());
        assert!(list(dir.path()).is_empty());
    }
}
