//! Borrowed preflight for the native visible-body cap.
use super::*;
use onecad_core::{
    document::component::{ComponentDef, Occurrence},
    edit::{AssemblyEdit, VisibilityTarget},
};

pub(super) fn check(
    runtime: &DocumentRuntime,
    edit: &AssemblyEdit,
    maximum: usize,
) -> Result<(), ApiError> {
    let document = runtime.session.document();
    let mut count = runtime
        .regen
        .bodies
        .bodies()
        .iter()
        .filter(|body| {
            document
                .bodies
                .get(body.id)
                .map_or(body.visible, |authored| authored.visible)
        })
        .count();
    let inserted = match edit {
        AssemblyEdit::InsertOccurrence { occurrence } => Some(occurrence),
        _ => None,
    };
    for occurrence in document.assembly.occurrences.iter().chain(inserted) {
        if occurrence.parent.is_some() {
            return Err(ApiError::InvalidCommand(
                "nested occurrences are unsupported".into(),
            ));
        }
        let visible = match edit {
            AssemblyEdit::SetVisibility {
                occurrence: id,
                visible,
            } if *id == occurrence.id => *visible,
            _ => document
                .assembly
                .visibility
                .get(&occurrence.id)
                .copied()
                .unwrap_or(true),
        };
        if !visible {
            continue;
        }
        let definition = document
            .definitions
            .get(&occurrence.def)
            .ok_or_else(|| ApiError::InvalidCommand("occurrence definition unavailable".into()))?;
        let added = count_definition(runtime, definition, occurrence, edit)?;
        count = count
            .checked_add(added)
            .ok_or_else(|| ApiError::InvalidCommand("visible body count overflow".into()))?;
        if count > maximum {
            return Err(over_limit(maximum));
        }
    }
    if count > maximum {
        return Err(over_limit(maximum));
    }
    Ok(())
}

fn count_definition(
    runtime: &DocumentRuntime,
    definition: &ComponentDef,
    occurrence: &Occurrence,
    edit: &AssemblyEdit,
) -> Result<usize, ApiError> {
    let bodies = definition.bodies.bodies();
    let mut rows = bodies.len();
    let mut visible = bodies.iter().filter(|body| body.visible).count();
    let definition_edit = match edit {
        AssemblyEdit::EditDefinition { id, command } if *id == definition.id => {
            Some(command.as_ref())
        }
        _ => None,
    };
    if let Some(command) = definition_edit {
        match command {
            EditCommand::AddBody { body } if definition.bodies.get(body.id).is_none() => {
                rows += 1;
                visible += usize::from(body.visible);
            }
            EditCommand::DeleteBody { body } => {
                if let Some(existing) = definition.bodies.get(*body) {
                    rows -= 1;
                    visible -= usize::from(existing.visible);
                }
            }
            EditCommand::SetVisibility {
                target: VisibilityTarget::Body(id),
                visible: next,
            } => {
                if let Some(existing) = definition.bodies.get(*id) {
                    visible = visible - usize::from(existing.visible) + usize::from(*next);
                }
            }
            _ => {}
        }
    }
    if rows != 0 || definition.timeline.records().is_empty() {
        return Ok(visible);
    }
    if matches!(edit, AssemblyEdit::MakeIndependent { occurrence: id, .. } if *id == occurrence.id)
        || runtime.component_generation(definition.id).is_none()
    {
        return Err(ApiError::InvalidCommand(
            "component geometry must be replayed before showing an occurrence".into(),
        ));
    }
    Ok(runtime
        .component_cache
        .get(&definition.id)
        .map_or(0, |cache| cache.meshes.len()))
}

fn over_limit(maximum: usize) -> ApiError {
    ApiError::InvalidCommand(format!("assembly exceeds the {maximum} visible body limit"))
}
