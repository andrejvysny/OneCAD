use super::{Applications, application_key};
use onecad_extension_host::ExtensionRegistry;
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub(super) const MAX_SETTINGS_BYTES: usize = 65_536;

pub(super) fn reset_application_settings(path: &Path, backup: &Path) -> Result<bool, String> {
    let preserved = match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(format!("Cannot inspect application settings: {error}")),
        Ok(_) => {
            // Hard-link creation never overwrites a backup; atomic replacement
            // below leaves both original and backup untouched if writing fails.
            fs::hard_link(path, backup)
                .map_err(|e| format!("Cannot preserve application settings backup: {e}"))?;
            true
        }
    };
    write_settings(path, &Applications::new())?;
    Ok(preserved)
}

pub(super) fn read_settings(path: &Path) -> Result<Applications, String> {
    let Some(bytes) = read_settings_bytes(path)? else {
        return Ok(Applications::new());
    };
    serde_json::from_slice(&bytes).map_err(|e| format!("Invalid application settings: {e}"))
}

pub(super) fn read_settings_bytes(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Cannot read application settings: {error}")),
        Ok(metadata) if !metadata.is_file() => {
            return Err("Application settings must be a regular file".into());
        }
        Ok(metadata) if metadata.len() > MAX_SETTINGS_BYTES as u64 => {
            return Err("Application settings exceed size limit".into());
        }
        Ok(_) => {}
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take((MAX_SETTINGS_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_SETTINGS_BYTES {
        return Err("Application settings exceed size limit".into());
    }
    Ok(Some(bytes))
}

pub(super) fn write_settings<T: serde::Serialize>(
    path: &Path,
    applications: &T,
) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(applications).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_SETTINGS_BYTES {
        return Err("Application settings exceed size limit".into());
    }
    let parent = path
        .parent()
        .ok_or("Invalid application settings location")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temporary = parent.join(format!(
        ".extension-applications-{}.tmp",
        uuid::Uuid::new_v4()
    ));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|e| format!("Cannot save application settings: {e}"))
}

pub(super) fn save_application(
    registry: &ExtensionRegistry,
    applications: &mut Applications,
    extension: &str,
    application: &str,
    executable: Option<PathBuf>,
) -> Result<(), String> {
    if !registry.packages().iter().any(|package| {
        package.manifest.id == extension
            && package
                .manifest
                .applications
                .iter()
                .any(|app| app.id == application)
    }) {
        return Err("Application is not declared by this installed package".into());
    }
    let key = application_key(extension, application);
    if let Some(path) = executable {
        let bundle = cfg!(target_os = "macos")
            && path.is_dir()
            && path.extension().is_some_and(|ext| ext == "app");
        if !path.is_absolute() || !(path.is_file() || bundle) {
            return Err("Choose an existing executable with an absolute path".into());
        }
        applications.insert(key, path);
    } else {
        applications.remove(&key);
    }
    Ok(())
}
