use crate::*;
use onecad_core::{
    document::{
        record::{
            BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord,
        },
        variables::Scalar,
    },
    edit::EditCommand,
    ids::RecordId,
};

impl Interaction {
    pub(crate) fn begin_extrude(
        &mut self,
        record_id: RecordId,
        gesture: u64,
        step_index: u32,
    ) -> Vec<Effect> {
        let Some(selection) = self.selection.as_ref() else {
            return Vec::new();
        };
        if self.context.as_ref() != Some(&selection.context) {
            return Vec::new();
        }
        let params = ExtrudeParams {
            profile: Some(selection.profile.clone()),
            distance: Scalar::new(10.0),
            draft_angle_deg: Scalar::new(0.0),
            mode: ExtrudeMode::Blind,
            boolean_mode: BooleanMode::NewBody,
            target_body: None,
            target_face: None,
            two_directions: false,
            mode2: ExtrudeMode::Blind,
            distance2: Scalar::new(0.0),
            target_face2: None,
            extra: Default::default(),
        };
        let record = OperationRecord::new(
            record_id,
            step_index,
            "Extrude",
            Operation::Known(KnownOperation::Extrude(params)),
        );
        let distance = ParameterKey::from(NumericParameter::ExtrudeDistance);
        let fields = declared_keys(&record.op);
        self.begin_operation(record, fields, Some(distance), gesture)
    }

    pub(crate) fn begin_operation(
        &mut self,
        record: OperationRecord,
        fields: Vec<ParameterKey>,
        focus: Option<ParameterKey>,
        gesture: u64,
    ) -> Vec<Effect> {
        self.start(record, &fields, focus, gesture, false)
    }

    pub(crate) fn begin_edit(&mut self, record: OperationRecord, gesture: u64) -> Vec<Effect> {
        self.begin_parameter_edit(record, NumericParameter::ExtrudeDistance, gesture)
    }

    pub(crate) fn begin_operation_edit(
        &mut self,
        record: OperationRecord,
        gesture: u64,
    ) -> Vec<Effect> {
        let fields = declared_keys(&record.op);
        let focus = fields.first().copied();
        self.start(record, &fields, focus, gesture, true)
    }

    pub(crate) fn begin_parameter_edit(
        &mut self,
        record: OperationRecord,
        parameter: NumericParameter,
        gesture: u64,
    ) -> Vec<Effect> {
        let fields = declared_keys(&record.op);
        self.start(record, &fields, Some(parameter.into()), gesture, true)
    }

    fn start(
        &mut self,
        record: OperationRecord,
        fields: &[ParameterKey],
        focus: Option<ParameterKey>,
        gesture: u64,
        editing_existing: bool,
    ) -> Vec<Effect> {
        self.start_with_repair(record, fields, focus, gesture, editing_existing, None)
    }

    pub(crate) fn begin_input_repair(
        &mut self,
        repair: onecad_app::api::InputRepair,
        gesture: u64,
    ) -> Vec<Effect> {
        self.start_with_repair(
            repair.record().clone(),
            &[],
            None,
            gesture,
            true,
            Some(repair),
        )
    }

    fn start_with_repair(
        &mut self,
        record: OperationRecord,
        fields: &[ParameterKey],
        focus: Option<ParameterKey>,
        gesture: u64,
        editing_existing: bool,
        repair: Option<onecad_app::api::InputRepair>,
    ) -> Vec<Effect> {
        if self.tool.as_ref().is_some_and(|tool| {
            matches!(
                tool.phase,
                SessionPhase::Committing | SessionPhase::Accepted
            )
        }) {
            return Vec::new();
        }
        let Some(context) = self.context.clone() else {
            return Vec::new();
        };
        let params = match declare(&record, fields, focus, &context, editing_existing) {
            Ok(params) => params,
            Err(message) => {
                let effects = self.cancel();
                self.status = CommitStatus::Rejected(message);
                return effects;
            }
        };
        let mut effects = self.cancel();
        let mut tool = ModelSession {
            tag: RequestTag {
                context,
                record_id: record.record_id,
                gesture,
                sequence: self.sequence,
            },
            origin: Box::new(record.op.clone()),
            record,
            editing_existing,
            repair,
            params,
            draft: String::new(),
            draft_status: DraftStatus::Pending,
            preview: PreviewStatus::Empty,
            phase: SessionPhase::Editing,
            drag: None,
        };
        if let Some(field) = focus.and_then(|key| tool.params.field_mut(key)) {
            field.status = FieldStatus::Pending;
        }
        self.tool = Some(tool);
        self.status = CommitStatus::Idle;
        self.deferred_completion = None;
        effects.extend(self.changed(focus));
        effects
    }

    /// Typed text for the focused field.
    pub(crate) fn change_draft(&mut self, draft: String) -> Vec<Effect> {
        let Some(key) = self.tool.as_ref().and_then(|tool| tool.params.focus()) else {
            return Vec::new();
        };
        self.change_parameter(key, draft)
    }

    /// Only an editing tool with a draft has anything to re-evaluate; a commit in
    /// flight is not a candidate and is never re-sent.
    pub(crate) fn republish(&mut self) -> Vec<Effect> {
        if self.tool.as_ref().is_none_or(|tool| {
            tool.phase != SessionPhase::Editing
                || tool.params.active().next().is_some() && tool.draft.is_empty()
        }) {
            return Vec::new();
        }
        if self.tool.as_ref().is_some_and(|tool| tool.repair.is_some()) {
            return self.changed(None);
        }
        self.refresh()
    }

    pub(crate) fn confirm(&mut self) -> Vec<Effect> {
        let Some(tool) = self.tool.as_mut().filter(|tool| tool.ready()) else {
            return Vec::new();
        };
        let operation = match tool.params.candidate(&tool.record.op) {
            Ok(operation) => operation,
            Err(error) => {
                tool.params.op_error = Some(error);
                return Vec::new();
            }
        };
        if let Err(message) = crate::extrude::validate_target(&operation) {
            tool.preview = PreviewStatus::Failed(message);
            return Vec::new();
        }
        let clear = Effect::ClearPreview {
            gesture: tool.tag.gesture,
        };
        if let Some(repair) = tool.repair.clone() {
            tool.phase = SessionPhase::Committing;
            self.status = CommitStatus::Pending(tool.tag.clone());
            return vec![
                clear,
                Effect::ApplyInputRepair {
                    tag: tool.tag.clone(),
                    repair: Box::new(repair),
                },
            ];
        }
        if tool.editing_existing && operation == *tool.origin {
            // Nothing changed: no command, no history entry, no revision.
            self.tool = None;
            self.status = CommitStatus::Idle;
            return vec![clear];
        }
        tool.drag = None;
        tool.record.op = operation;
        let (command, hide_sketch) = commit_command(tool);
        tool.phase = SessionPhase::Committing;
        self.status = CommitStatus::Pending(tool.tag.clone());
        vec![
            clear,
            Effect::ApplyOperation {
                tag: tool.tag.clone(),
                command: Box::new(command),
                hide_sketch,
            },
        ]
    }

    /// Cancel the transient tool; selection and accepted document state stay intact.
    pub(crate) fn cancel(&mut self) -> Vec<Effect> {
        let Some(tool) = self.tool.as_ref() else {
            return Vec::new();
        };
        let effect = Effect::ClearPreview {
            gesture: tool.tag.gesture,
        };
        if matches!(tool.phase, SessionPhase::Editing | SessionPhase::Completed) {
            if let Some(repair) = &tool.repair {
                repair.cancel();
            }
            self.tool = None;
        }
        vec![effect]
    }
}

fn declare(
    record: &OperationRecord,
    fields: &[ParameterKey],
    focus: Option<ParameterKey>,
    context: &DocumentContext,
    editing_existing: bool,
) -> Result<ParameterSet, String> {
    if context.document_id.is_empty() || context.runtime_session.is_empty() {
        return Err("Document context is incomplete".into());
    }
    if !editing_existing {
        validate_creation(record, fields)?;
    }
    ParameterSet::declare(&record.op, fields, focus)
}

fn commit_command(tool: &ModelSession) -> (EditCommand, Option<onecad_core::ids::SketchId>) {
    if tool.editing_existing {
        return (
            EditCommand::UpdateOperationParams {
                record: tool.tag.record_id,
                op: tool.record.op.clone(),
            },
            None,
        );
    }
    let sketch = profile_sketch(&tool.record.op);
    (
        EditCommand::AddOperation {
            record: tool.record.clone(),
            at_cursor: true,
        },
        sketch,
    )
}

pub(crate) fn profile_sketch(operation: &Operation) -> Option<onecad_core::ids::SketchId> {
    match operation {
        Operation::Known(KnownOperation::Extrude(p)) => p.profile.as_ref().map(|p| p.sketch),
        Operation::Known(KnownOperation::Revolve(p)) => p.profile.as_ref().map(|p| p.sketch),
        _ => None,
    }
}

fn validate_creation(record: &OperationRecord, fields: &[ParameterKey]) -> Result<(), String> {
    let Operation::Known(operation) = &record.op else {
        return Err("Unknown operations cannot start native authoring".into());
    };
    if record.suppressed || !record.outputs.is_empty() {
        return Err("New operation must be unsuppressed and have no authored outputs".into());
    }
    if matches!(operation, KnownOperation::TransformBody(_))
        && record.inputs != record.op.derive_inputs()
    {
        return Err("New TransformBody inputs must match its target bodies".into());
    }
    validate_bindings(operation)?;
    for key in fields {
        let parameter = key.parameter();
        let scalar = parameter.current_scalar(&record.op)?;
        let mut candidate = record.op.clone();
        parameter.apply(&mut candidate, scalar)?;
    }
    let mut checked = operation.clone();
    if checked
        .scalars_mut()
        .iter()
        .any(|(_, _, s)| !s.value.is_finite())
    {
        return Err("New operation parameters must be finite".into());
    }
    Ok(())
}

fn validate_bindings(operation: &KnownOperation) -> Result<(), String> {
    use onecad_core::document::refs::ElementKind;
    match operation {
        KnownOperation::Extrude(p) => {
            validate_profile(p.profile.as_ref())?;
            validate_boolean_target(p.boolean_mode, p.target_body)?;
            if p.mode == ExtrudeMode::ToFace {
                require_face(p.target_face.as_ref())?;
            }
            if p.two_directions && p.mode2 == ExtrudeMode::ToFace {
                require_face(p.target_face2.as_ref())?;
            }
            Ok(())
        }
        KnownOperation::Revolve(p) => {
            validate_profile(p.profile.as_ref())?;
            validate_boolean_target(p.boolean_mode, p.target_body)?;
            validate_axis(p.axis.as_ref())
        }
        KnownOperation::Loft(p) => p.validate(),
        KnownOperation::Fillet(p) => validate_edges(&p.edge_ids, &p.edges),
        KnownOperation::Chamfer(p) => {
            validate_edges(&p.edge_ids, &p.edges)?;
            validate_chamfer_faces(p)
        }
        KnownOperation::Shell(p) => {
            let body = p.target_body.ok_or("New Shell requires target body")?;
            validate_faces(&p.open_faces, &p.faces, body)
        }
        KnownOperation::Hole(p) => {
            validate_primary(&p.face, ElementKind::Face, Some(p.target_body))
        }
        KnownOperation::OffsetFace(p) => validate_faces(&p.face_ids, &p.faces, p.target_body),
        KnownOperation::TransformBody(params) => params.validate(),
        KnownOperation::Gear(params) => params.validate(),
        KnownOperation::LinearPattern(_)
        | KnownOperation::CircularPattern(_)
        | KnownOperation::MirrorBody(_)
        | KnownOperation::Boolean(_) => crate::modeling_options::validate(operation),
        KnownOperation::FeaturePattern(_) => {
            Err("FeaturePattern preview requires authoritative source expansion".into())
        }
        _ => Err("Operation is not supported by native parameterized creation".into()),
    }
}

fn validate_chamfer_faces(
    params: &onecad_core::document::record::ChamferParams,
) -> Result<(), String> {
    if !params.is_asymmetric() {
        return if params.reference_faces.is_empty() && params.reference_face_refs.is_empty() {
            Ok(())
        } else {
            Err("Equal-leg Chamfer does not carry reference faces".into())
        };
    }
    if params.reference_faces.is_empty() {
        return Err("New asymmetric Chamfer requires promoted reference faces".into());
    }
    let body = params
        .edges
        .first()
        .and_then(|edge| edge.primary.as_ref())
        .ok_or("New asymmetric Chamfer requires promoted edges")?
        .body;
    let ids: Vec<_> = params
        .reference_faces
        .iter()
        .map(|pair| pair.face_id.clone())
        .collect();
    validate_refs(
        &ids,
        &params.reference_face_refs,
        onecad_core::document::refs::ElementKind::Face,
        body,
    )
}

fn validate_boolean_target(
    mode: BooleanMode,
    target: Option<onecad_core::ids::BodyId>,
) -> Result<(), String> {
    if mode != BooleanMode::NewBody && target.is_none() {
        return Err("Boolean feature requires an explicit target body".into());
    }
    Ok(())
}

fn validate_profile(
    profile: Option<&onecad_core::document::refs::SketchRegionRef>,
) -> Result<(), String> {
    let profile = profile.ok_or("New profile operation requires an explicit sketch region")?;
    if profile.region.as_str().is_empty() {
        return Err("Sketch region identity is empty".into());
    }
    Ok(())
}

fn validate_axis(axis: Option<&onecad_core::document::refs::AxisRef>) -> Result<(), String> {
    use onecad_core::document::refs::{AxisRef, ElementKind};
    match axis {
        Some(AxisRef::SketchLine { .. }) => Ok(()),
        Some(AxisRef::Element {
            body,
            edge,
            edge_ref: Some(reference),
            ..
        }) => {
            validate_primary(reference, ElementKind::Edge, Some(*body))?;
            if reference
                .primary
                .as_ref()
                .is_none_or(|p| p.element != *edge)
            {
                return Err("Revolve axis IDs do not match".into());
            }
            Ok(())
        }
        _ => Err("New Revolve requires an explicit sketch axis or promoted edge axis".into()),
    }
}

fn require_face(face: Option<&onecad_core::document::refs::ElementRef>) -> Result<(), String> {
    validate_primary(
        face.ok_or("ToFace extrusion requires a promoted target face")?,
        onecad_core::document::refs::ElementKind::Face,
        None,
    )
}

fn validate_edges(
    ids: &[onecad_core::ids::ElementId],
    refs: &[onecad_core::document::refs::ElementRef],
) -> Result<(), String> {
    let first = refs
        .first()
        .and_then(|r| r.primary.as_ref())
        .ok_or("New edge operation requires promoted edges")?;
    validate_refs(
        ids,
        refs,
        onecad_core::document::refs::ElementKind::Edge,
        first.body,
    )
}

fn validate_faces(
    ids: &[onecad_core::ids::ElementId],
    refs: &[onecad_core::document::refs::ElementRef],
    body: onecad_core::ids::BodyId,
) -> Result<(), String> {
    validate_refs(
        ids,
        refs,
        onecad_core::document::refs::ElementKind::Face,
        body,
    )
}

fn validate_refs(
    ids: &[onecad_core::ids::ElementId],
    refs: &[onecad_core::document::refs::ElementRef],
    kind: onecad_core::document::refs::ElementKind,
    body: onecad_core::ids::BodyId,
) -> Result<(), String> {
    if ids.len() != refs.len() {
        return Err("New operation requires complete typed reference slots".into());
    }
    for (id, reference) in ids.iter().zip(refs) {
        validate_primary(reference, kind, Some(body))?;
        if reference.primary.as_ref().is_none_or(|p| p.element != *id) {
            return Err("Typed reference ID does not match operation input".into());
        }
    }
    Ok(())
}

fn validate_primary(
    reference: &onecad_core::document::refs::ElementRef,
    kind: onecad_core::document::refs::ElementKind,
    body: Option<onecad_core::ids::BodyId>,
) -> Result<(), String> {
    let primary = reference
        .primary
        .as_ref()
        .ok_or("New operation requires promoted primary references")?;
    if primary.kind != kind || body.is_some_and(|body| body != primary.body) {
        return Err("New operation reference body or kind mismatch".into());
    }
    if primary.element.as_str().is_empty()
        || onecad_core::ids::TopoKey::new(primary.element.as_str()).is_valid()
    {
        return Err("New operation requires persistent element IDs, not snapshot TopoKeys".into());
    }
    Ok(())
}
