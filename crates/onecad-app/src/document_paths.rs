//! Resolve saved references after an Apple application container is relocated.
//!
//! A matching filename is insufficient evidence: only a relative location in an
//! application-owned Documents directory and a matching document UUID qualify.
use std::path::{Component, Path, PathBuf};

use onecad_core::{ids::DocumentId, io::container::ContainerReader};

/// Whether a path names a file inside an Apple application container's Documents.
/// This recognizes both device and Simulator prefixes without guessing external
/// file locations or treating an arbitrary directory called Documents as owned.
pub fn is_container_document(path: &Path) -> bool {
    container_documents(path).is_some()
}

fn container_documents(path: &Path) -> Option<&Path> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return None;
    }
    path.ancestors().find(|root| {
        root.file_name().is_some_and(|name| name == "Documents")
            && root.parent().is_some_and(|container| {
                container
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| uuid::Uuid::parse_str(name).is_ok())
                    && container.parent().is_some_and(|application| {
                        application.ends_with("Containers/Data/Application")
                    })
            })
    })
}

/// Return a validated current location for a missing, relocated saved path.
/// Existing sources and external paths are never redirected. Invalid candidates
/// are errors so callers can preserve unresolved references instead of pruning.
pub fn relocate_saved_path(
    documents: &Path,
    original: &Path,
    expected: DocumentId,
) -> Result<Option<PathBuf>, String> {
    if original.try_exists().map_err(|error| error.to_string())? {
        return Ok(None);
    }
    let documents = documents
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if container_documents(&documents) != Some(documents.as_path()) {
        return Ok(None);
    }
    let Some(previous) = container_documents(original) else {
        return Ok(None);
    };
    let relative = original.strip_prefix(previous).map_err(|e| e.to_string())?;
    if relative.as_os_str().is_empty() || previous == documents {
        return Ok(None);
    }
    let candidate = documents.join(relative);
    if !candidate.try_exists().map_err(|error| error.to_string())? {
        return Ok(None);
    }
    let candidate = candidate
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !candidate.starts_with(&documents) {
        return Err("Relocated document escapes the current Documents directory".into());
    }
    let reader = ContainerReader::open(&candidate).map_err(|error| error.to_string())?;
    if reader.document().id != expected {
        return Err("Relocated path belongs to a different document; reference retained".into());
    }
    Ok(Some(candidate))
}

#[cfg(test)]
#[path = "document_paths_tests.rs"]
mod tests;
