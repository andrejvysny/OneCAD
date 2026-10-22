//! Stable, toolkit-independent extension protocol. No document or platform handles
//! cross this boundary. Version 1 returns a plan, validated before any side effect.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const SCHEMA_VERSION: u32 = 1;
pub const MANIFEST_FILE: &str = "manifest.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    pub runtime: ExtensionRuntime,
    pub entry: String,
    #[serde(default)]
    pub capabilities: Vec<Capability>,
    #[serde(default)]
    pub applications: Vec<ApplicationContribution>,
    #[serde(default)]
    pub contributions: Contributions,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionRuntime {
    PortableComponent,
    /// Recognized for explicit trust review; the portable host never loads native code.
    NativeTrusted,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    #[serde(rename = "export_3mf")]
    Export3mf,
    LaunchApplication,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ApplicationContribution {
    pub id: String,
    pub name: String,
    /// A declarative discovery hint, never an executable command or path.
    #[serde(default)]
    pub macos_bundle_name: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Contributions {
    #[serde(default)]
    pub commands: Vec<CommandContribution>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CommandContribution {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub requires_document: bool,
    #[serde(default)]
    pub requires_selection: bool,
    #[serde(default)]
    pub menu: Option<CommandMenu>,
    /// Stable host locations: `export`, `menu`, and `palette`.
    #[serde(default)]
    pub locations: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandMenu {
    Export,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CommandContext {
    pub has_document: bool,
    pub has_selection: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CommandExecution {
    #[serde(default)]
    pub requests: Vec<HostRequest>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostRequest {
    #[serde(rename = "export_3mf")]
    Export3mf {
        scope: ExportScope,
        suggested_name: String,
    },
    LaunchApplication {
        application: String,
        artifact: ArtifactRef,
    },
}

impl HostRequest {
    pub fn capability(&self) -> Capability {
        match self {
            Self::Export3mf { .. } => Capability::Export3mf,
            Self::LaunchApplication { .. } => Capability::LaunchApplication,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExportScope {
    Selection,
    Document,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactRef {
    PreviousExport,
}

#[derive(Debug, thiserror::Error)]
#[error("invalid extension manifest: {0}")]
pub struct ManifestError(pub String);

impl Manifest {
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.schema_version != SCHEMA_VERSION {
            return invalid("unsupported schema_version");
        }
        if !valid_id(&self.id) || !self.id.contains('.') || !valid_label(&self.name, 200) {
            return invalid("invalid package id or name");
        }
        if !valid_label(&self.version, 100) {
            return invalid("invalid version");
        }
        if self.entry.is_empty() || self.entry.contains('\\') || self.entry.contains(':') {
            return invalid("entry must be a relative package path");
        }
        if !std::path::Path::new(&self.entry)
            .components()
            .all(|p| matches!(p, std::path::Component::Normal(_)))
        {
            return invalid("entry must stay inside the package");
        }
        self.validate_contributions()
    }

    fn validate_contributions(&self) -> Result<(), ManifestError> {
        if self.capabilities.iter().collect::<BTreeSet<_>>().len() != self.capabilities.len() {
            return invalid("duplicate capabilities");
        }
        let mut ids = BTreeSet::new();
        if self.contributions.commands.len() > 128 || self.applications.len() > 16 {
            return invalid("too many contributions");
        }
        for command in &self.contributions.commands {
            if !valid_id(&command.id)
                || !command.id.starts_with(&format!("{}.", self.id))
                || !ids.insert(&command.id)
                || !valid_label(&command.title, 200)
            {
                return invalid("invalid or duplicate command");
            }
            if command
                .locations
                .iter()
                .any(|s| !matches!(s.as_str(), "export" | "menu" | "palette"))
            {
                return invalid("unsupported command location");
            }
        }
        ids.clear();
        for app in &self.applications {
            if !valid_id(&app.id)
                || !app.id.starts_with(&format!("{}.", self.id))
                || !ids.insert(&app.id)
                || !valid_label(&app.name, 200)
            {
                return invalid("invalid or duplicate application");
            }
            if app
                .macos_bundle_name
                .as_ref()
                .is_some_and(|s| !valid_label(s, 200) || s.contains(['/', '\\', ':']))
            {
                return invalid("application discovery hint must be a bundle name");
            }
        }
        Ok(())
    }
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 160
        && id.as_bytes()[0].is_ascii_lowercase()
        && id.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'_')
        })
        && !id.contains("..")
}

fn invalid<T>(reason: &str) -> Result<T, ManifestError> {
    Err(ManifestError(reason.to_owned()))
}

fn valid_label(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}
