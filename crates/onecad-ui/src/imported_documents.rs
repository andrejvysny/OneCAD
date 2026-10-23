//! Legacy Inbox paths are inputs; editable copies retain their native identity.
use onecad_core::io::container::ContainerReader;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(crate) fn migrate(documents: &Path, source: &Path) -> Result<PathBuf, String> {
    let source = source.canonicalize().map_err(|error| error.to_string())?;
    let documents = documents
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !source.starts_with(documents.join("Inbox")) {
        return Ok(source);
    }
    let identity = ContainerReader::open(&source)
        .map_err(|error| error.to_string())?
        .document()
        .id;
    let working = documents.join("Imported").join("Inbox");
    fs::create_dir_all(&working).map_err(|error| error.to_string())?;
    let working = working.canonicalize().map_err(|error| error.to_string())?;
    if !working.starts_with(&documents) {
        return Err("Imported working directory must remain inside Documents".into());
    }
    let target = working.join(source.file_name().ok_or("Missing Inbox filename")?);
    if target
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err("Inbox migration destination must not be a symlink".into());
    }
    if target.exists() {
        let existing = ContainerReader::open(&target).map_err(|error| error.to_string())?;
        if existing.document().id != identity {
            return Err(
                "Inbox migration destination belongs to another document; original retained".into(),
            );
        }
        return Ok(target);
    }
    let temporary = working.join(format!(".migrate-{}", uuid::Uuid::new_v4()));
    let output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    let result = copy_durable(&source, &temporary, &target, identity, output);
    let _ = fs::remove_file(&temporary);
    result.map(|()| target)
}

fn copy_durable(
    source: &Path,
    temporary: &Path,
    target: &Path,
    identity: onecad_core::ids::DocumentId,
    mut output: fs::File,
) -> Result<(), String> {
    let mut linked = false;
    let result = (|| {
        let mut input = fs::File::open(source)?;
        std::io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        if ContainerReader::open(temporary)
            .map_err(|error| std::io::Error::other(error.to_string()))?
            .document()
            .id
            != identity
        {
            return Err(std::io::Error::other(
                "Inbox source identity changed during copy",
            ));
        }
        fs::hard_link(temporary, target)?;
        linked = true;
        fs::File::open(
            target
                .parent()
                .ok_or_else(|| std::io::Error::other("Missing working directory"))?,
        )?
        .sync_all()
    })();
    if let Err(error) = result {
        if linked {
            let _ = fs::remove_file(target);
        }
        return Err(format!(
            "Inbox migration failed; original retained: {error}"
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "imported_documents_tests.rs"]
mod tests;
