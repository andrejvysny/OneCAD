use onecad_app::dto::{AssemblyDefinitionDto, AssemblyProjectionDto};
use onecad_assembly::occurrence::ComponentDefId;
use onecad_core::{
    document::record::{KnownOperation, Operation, OperationRecord, TransformBodyParams},
    document::variables::Scalar,
    edit::{AssemblyEdit, EditCommand},
    ids::{BodyId, RecordId},
};

use super::{Action, submit};
use crate::app::App;

pub(super) fn panel(app: &mut App, ui: &mut egui::Ui, projection: &AssemblyProjectionDto) {
    let Some(definition) = app.doc.assembly_controls.definition.and_then(|id| {
        projection
            .definitions
            .iter()
            .find(|definition| definition.id == id.to_string())
    }) else {
        return;
    };
    if definition.id == projection.root_definition {
        return;
    }
    ui.label("Translate shared definition geometry (mm)");
    ui.label("Adds a local feature to every instance of this definition.");
    super::panel::vector(
        ui,
        "Translation",
        &mut app.doc.assembly_controls.definition_translation,
    );
    if ui
        .add_enabled(
            !definition.bodies.is_empty() && definition.mesh_generation.is_some(),
            egui::Button::new("Apply shared translation"),
        )
        .clicked()
    {
        match translate(definition, app.doc.assembly_controls.definition_translation) {
            Ok(edit) => submit(app, Action::Edit(edit)),
            Err(error) => app.doc.status = error,
        }
    }
}

pub(crate) fn translate(
    definition: &AssemblyDefinitionDto,
    delta: [f64; 3],
) -> Result<AssemblyEdit, String> {
    if definition.mesh_generation.is_none() {
        return Err("Definition geometry is not current; show an instance and retry".into());
    }
    if !delta.iter().all(|value| value.is_finite()) || delta == [0.0; 3] {
        return Err("Translation must be finite and nonzero".into());
    }
    let id =
        ComponentDefId(uuid::Uuid::parse_str(&definition.id).map_err(|error| error.to_string())?);
    let targets: Vec<BodyId> = definition
        .bodies
        .iter()
        .map(|body| {
            body.id
                .parse()
                .map_err(|error| format!("Definition body identity: {error}"))
        })
        .collect::<Result<_, _>>()?;
    if targets.is_empty() {
        return Err("Definition has no accepted bodies".into());
    }
    let op = Operation::Known(KnownOperation::TransformBody(TransformBodyParams {
        targets,
        translate: delta.map(Scalar::new),
        rotate: Default::default(),
        copy: false,
        extra: Default::default(),
    }));
    Ok(AssemblyEdit::EditDefinition {
        id,
        command: Box::new(EditCommand::AddOperation {
            record: OperationRecord::new(RecordId::new(), 0, "Translate shared geometry", op),
            at_cursor: true,
        }),
    })
}
