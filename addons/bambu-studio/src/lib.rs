//! Bambu Studio integration implemented entirely outside the native host.
use onecad_extension_sdk::{
    Addon, ArtifactRef, CommandContext, CommandExecution, ExportScope, HostRequest,
};

pub const COMMAND_ID: &str = "dev.onecad.bambu-studio.send";
pub const APPLICATION_ID: &str = "dev.onecad.bambu-studio.application";

pub struct BambuStudio;

impl Addon for BambuStudio {
    fn execute(command: &str, context: &CommandContext) -> Result<CommandExecution, String> {
        if command != COMMAND_ID {
            return Err(format!("unknown Bambu Studio command: {command}"));
        }
        if !context.has_document {
            return Err("Open a document before sending it to Bambu Studio.".into());
        }
        let scope = if context.has_selection {
            ExportScope::Selection
        } else {
            ExportScope::Document
        };
        Ok(CommandExecution {
            requests: vec![
                HostRequest::Export3mf {
                    scope,
                    suggested_name: "OneCAD.3mf".into(),
                },
                HostRequest::LaunchApplication {
                    application: APPLICATION_ID.into(),
                    artifact: ArtifactRef::PreviousExport,
                },
            ],
            message: Some("Sent to Bambu Studio".into()),
        })
    }
}

onecad_extension_sdk::export_addon!(BambuStudio);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_is_exported_before_the_opaque_artifact_is_handed_off() {
        let execution = BambuStudio::execute(
            COMMAND_ID,
            &CommandContext {
                has_document: true,
                has_selection: true,
            },
        )
        .unwrap();
        assert!(matches!(&execution.requests[..], [
            HostRequest::Export3mf { scope: ExportScope::Selection, .. },
            HostRequest::LaunchApplication { application, artifact: ArtifactRef::PreviousExport }
        ] if application == APPLICATION_ID));
    }

    #[test]
    fn no_selection_exports_the_document_and_no_document_is_rejected() {
        let execution = BambuStudio::execute(
            COMMAND_ID,
            &CommandContext {
                has_document: true,
                has_selection: false,
            },
        )
        .unwrap();
        assert!(matches!(
            execution.requests[0],
            HostRequest::Export3mf {
                scope: ExportScope::Document,
                ..
            }
        ));
        assert!(BambuStudio::execute(COMMAND_ID, &CommandContext::default()).is_err());
        assert!(BambuStudio::execute("unknown", &CommandContext::default()).is_err());
    }
}
