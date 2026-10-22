use crate::{api, Approval, HostError, Result};

pub(crate) fn validate_approval(manifest: &api::Manifest, approval: &Approval) -> Result<()> {
    if approval.capabilities.len() > manifest.capabilities.len()
        || approval.applications.len() > manifest.applications.len()
        || approval
            .capabilities
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != approval.capabilities.len()
        || approval
            .applications
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != approval.applications.len()
    {
        return Err(HostError::PermissionDenied(
            "duplicate capability or application approval".into(),
        ));
    }
    if approval
        .capabilities
        .iter()
        .any(|c| !manifest.capabilities.contains(c))
    {
        return Err(HostError::PermissionDenied(
            "undeclared capability in approval".into(),
        ));
    }
    if approval
        .applications
        .iter()
        .any(|id| !manifest.applications.iter().any(|a| &a.id == id))
    {
        return Err(HostError::PermissionDenied(
            "undeclared application in approval".into(),
        ));
    }
    if manifest.runtime == api::ExtensionRuntime::NativeTrusted && !approval.trust_native {
        return Err(HostError::PermissionDenied(
            "native extension requires explicit full trust".into(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_execution(
    manifest: &api::Manifest,
    approval: &Approval,
    context: &api::CommandContext,
    execution: &api::CommandExecution,
) -> Result<()> {
    if execution.requests.len() > 8 || execution.message.as_ref().is_some_and(|m| m.len() > 4096) {
        return Err(HostError::InvalidPackage(
            "command response exceeds plan limits".into(),
        ));
    }
    let mut exported = false;
    for request in &execution.requests {
        let capability = request.capability();
        if !manifest.capabilities.contains(&capability)
            || !approval.capabilities.contains(&capability)
        {
            return Err(HostError::PermissionDenied(format!(
                "capability {capability:?} is not approved"
            )));
        }
        match request {
            api::HostRequest::Export3mf {
                scope,
                suggested_name,
            } => {
                if !context.has_document
                    || (*scope == api::ExportScope::Selection && !context.has_selection)
                {
                    return Err(HostError::Unavailable(
                        "export needs a document and the requested selection".into(),
                    ));
                }
                if suggested_name.is_empty()
                    || suggested_name.len() > 200
                    || suggested_name.contains(['/', '\\', ':', '\0'])
                {
                    return Err(HostError::InvalidPackage(
                        "export name must be a plain filename".into(),
                    ));
                }
                exported = true;
            }
            api::HostRequest::LaunchApplication { application, .. } => {
                if !exported
                    || !approval.applications.contains(application)
                    || !manifest.applications.iter().any(|a| &a.id == application)
                {
                    return Err(HostError::PermissionDenied(
                        "application must be explicitly approved and follow an export".into(),
                    ));
                }
            }
        }
    }
    Ok(())
}
