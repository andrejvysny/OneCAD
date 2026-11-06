//! Captures and commits joint solves under the document writer.
use super::*;
use crate::error::ApiError;
use onecad_assembly::{
    joint::{Joint, JointId},
    solver::{SolveRequest, SolverBody},
};

pub(crate) struct JointCapture {
    pub session: String,
    pub generation: u64,
    pub revision: DocumentRevision,
    pub epoch: WorkerEpoch,
    pub snapshot: SnapshotId,
    pub base: onecad_core::regen::HistoryPrefixHash,
    pub request: SolveRequest,
    pub bindings: Option<Vec<onecad_core::document::component::JointBinding>>,
    pub engine: Arc<dyn GeometryEngine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JointHead {
    epoch: WorkerEpoch,
    snapshot: SnapshotId,
    base: onecad_core::regen::HistoryPrefixHash,
}

impl DocumentRuntime {
    pub(crate) fn joint_head(&self) -> JointHead {
        JointHead {
            epoch: self.fencing.get().1,
            snapshot: self.head_snapshot_id().unwrap_or(SnapshotId(0)),
            base: self.export_history_hash(),
        }
    }

    pub(crate) fn mark_joint_origin_repair(
        &mut self,
        session: &str,
        generation: u64,
        expected_head: JointHead,
        expected: &onecad_core::document::component::JointBinding,
        frame: onecad_assembly::math::Frame,
    ) -> Result<bool, ApiError> {
        if !self.matches_runtime_session(session)
            || self.persisted_generation() != generation
            || self.joint_head() != expected_head
        {
            return Err(ApiError::StalePreview("joint repair is stale".into()));
        }
        let assembly = &self.session.document().assembly;
        let stored_frame = assembly
            .joints
            .iter()
            .find(|joint| joint.id == expected.joint)
            .map(|joint| match expected.side {
                onecad_core::document::component::JointSide::A => joint.a.frame,
                onecad_core::document::component::JointSide::B => joint.b.frame,
            });
        if !assembly.joint_bindings.contains(expected) || stored_frame != Some(frame) {
            return Ok(false);
        }
        if self.is_read_only() {
            return Ok(self
                .transient_joint_repairs
                .insert((expected.joint, expected.side)));
        }
        let before = self.persisted_generation();
        self.apply_authoritative(EditCommand::Assembly {
            edit: Box::new(onecad_core::edit::AssemblyEdit::MarkJointOriginRepair {
                joint: expected.joint,
                side: expected.side,
            }),
        })?;
        Ok(before != self.persisted_generation())
    }

    pub(crate) fn prepare_joint_solve(
        &self,
        session: &str,
        joints: Vec<Joint>,
        drives: Vec<(JointId, f64)>,
    ) -> Result<JointCapture, ApiError> {
        self.prepare_bound_joint_solve(session, joints, drives, None)
    }

    pub(crate) fn prepare_bound_joint_solve(
        &self,
        session: &str,
        joints: Vec<Joint>,
        drives: Vec<(JointId, f64)>,
        bindings: Option<Vec<onecad_core::document::component::JointBinding>>,
    ) -> Result<JointCapture, ApiError> {
        if !self.matches_runtime_session(session) {
            return Err(ApiError::StalePreview("joint solve runtime changed".into()));
        }
        if self.is_read_only() {
            return Err(ApiError::InvalidCommand(
                "joint solve requires a writable document".into(),
            ));
        }
        let doc = self.session.document();
        if bindings.is_none() && !doc.assembly.joint_bindings.is_empty() {
            return Err(ApiError::InvalidCommand(
                "geometry-bound joint origins require revalidation before solving".into(),
            ));
        }
        let bodies: Vec<_> = doc
            .assembly
            .occurrences
            .iter()
            .map(|o| SolverBody {
                id: o.id,
                pose: o.local,
                grounded: o.grounded,
            })
            .collect();
        super::super::joint_execution::validate_tree(&bodies, &joints, &drives)?;
        let mut assembly = doc.assembly.clone();
        assembly.joints = joints.clone();
        if let Some(bindings) = &bindings {
            assembly.joint_bindings = bindings.clone();
        }
        assembly
            .validate(doc.root_definition_id, &doc.definitions)
            .map_err(ApiError::InvalidCommand)?;
        let mut request = SolveRequest::new(bodies, joints);
        request.drive = drives;
        Ok(JointCapture {
            session: self.runtime_session(),
            generation: self.persisted_generation(),
            revision: self.revision(),
            epoch: self.fencing.get().1,
            snapshot: self.head_snapshot_id().unwrap_or(SnapshotId(0)),
            base: self.export_history_hash(),
            request,
            bindings,
            engine: self.engine_arc(),
        })
    }

    pub(crate) fn commit_joint_solve(
        &mut self,
        capture: &JointCapture,
        joints: Vec<Joint>,
        poses: Vec<(
            onecad_assembly::occurrence::OccurrenceId,
            onecad_assembly::math::RigidTransform,
        )>,
    ) -> Result<(), ApiError> {
        if !self.matches_runtime_session(&capture.session)
            || self.persisted_generation() != capture.generation
            || self.revision() != capture.revision
            || self.fencing.get().1 != capture.epoch
            || self.head_snapshot_id().unwrap_or(SnapshotId(0)) != capture.snapshot
            || self.export_history_hash() != capture.base
        {
            return Err(ApiError::StalePreview("joint solve is stale".into()));
        }
        let edit = match &capture.bindings {
            Some(bindings) => onecad_core::edit::AssemblyEdit::SetBoundJointsAndPoses {
                joints,
                bindings: bindings.clone(),
                poses,
            },
            None => onecad_core::edit::AssemblyEdit::SetJointsAndPoses { joints, poses },
        };
        self.apply_authoritative(EditCommand::Assembly {
            edit: Box::new(edit),
        })
        .map_err(|error| ApiError::InvalidCommand(error.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use onecad_assembly::{
        joint::{Joint, JointId, JointKind, JointOrigin},
        math::{Frame, RigidTransform, Vec3d},
        occurrence::{ComponentDefId, Occurrence, OccurrenceId},
        solver::{self, SolveOutcome},
    };
    use uuid::Uuid;

    fn setup() -> (DocumentRuntime, Vec<Joint>) {
        let backend = Arc::new(crate::worker::PendingBackend);
        let mut runtime = DocumentRuntime::new_blank(backend.clone(), backend.clone(), backend);
        let def = ComponentDefId(Uuid::from_u128(3));
        runtime
            .apply_assembly_edit(onecad_core::edit::AssemblyEdit::CreateDefinition {
                id: def,
                name: "part".into(),
            })
            .unwrap();
        for n in 0..2 {
            runtime
                .apply_assembly_edit(onecad_core::edit::AssemblyEdit::InsertOccurrence {
                    occurrence: Occurrence {
                        id: OccurrenceId(Uuid::from_u128(n + 1)),
                        def,
                        parent: None,
                        local: RigidTransform::IDENTITY,
                        grounded: n == 0,
                        name: format!("part {n}"),
                    },
                })
                .unwrap();
        }
        let frame = Frame {
            origin: Vec3d::ZERO,
            z: Vec3d::Z,
            x: Vec3d::X,
        };
        let joint = Joint {
            id: JointId(Uuid::from_u128(4)),
            name: "slider".into(),
            kind: JointKind::Slider,
            a: JointOrigin {
                occurrence: OccurrenceId(Uuid::from_u128(1)),
                frame,
            },
            b: JointOrigin {
                occurrence: OccurrenceId(Uuid::from_u128(2)),
                frame,
            },
            limits: None,
            value: 0.0,
            suppressed: false,
        };
        (runtime, vec![joint])
    }

    #[test]
    fn read_only_revalidation_reports_repair_without_mutating_document() {
        use onecad_core::document::component::{
            JointBinding, JointSide, JointSurface, OccurrencePath,
        };
        let (mut runtime, joints) = setup();
        let body = onecad_core::ids::BodyId(Uuid::from_u128(9));
        let element = serde_json::from_value(serde_json::json!({
            "primary": {"bodyId": body, "elementId": "el_test", "kind": "face"},
            "intent": {"version": 1, "kind": "face", "descriptor": {}},
            "anchor": {"worldPoint": [0.0, 0.0, 0.0]}
        }))
        .unwrap();
        let binding = JointBinding {
            joint: joints[0].id,
            side: JointSide::A,
            path: OccurrencePath(vec![joints[0].a.occurrence]),
            definition: ComponentDefId(Uuid::from_u128(3)),
            body,
            element,
            surface: JointSurface::Plane,
            radius: None,
            orientation_x: Vec3d::X,
            needs_repair: false,
        };
        let poses = [joints[0].a.occurrence, joints[0].b.occurrence]
            .into_iter()
            .map(|id| (id, RigidTransform::IDENTITY))
            .collect();
        runtime
            .apply_authoritative(EditCommand::Assembly {
                edit: Box::new(onecad_core::edit::AssemblyEdit::SetBoundJointsAndPoses {
                    joints: joints.clone(),
                    bindings: vec![binding.clone()],
                    poses,
                }),
            })
            .unwrap();
        runtime.read_only = true;
        let generation = runtime.persisted_generation();
        let session = runtime.runtime_session();
        let head = runtime.joint_head();
        runtime.fencing.set_epoch(head.epoch.0 + 1);
        assert!(matches!(
            runtime.mark_joint_origin_repair(
                &session,
                generation,
                head,
                &binding,
                joints[0].a.frame
            ),
            Err(ApiError::StalePreview(_))
        ));
        assert!(!runtime.assembly_projection().unwrap().joint_bindings[0].needs_repair);
        let head = runtime.joint_head();
        assert!(runtime
            .mark_joint_origin_repair(&session, generation, head, &binding, joints[0].a.frame)
            .unwrap());
        assert_eq!(runtime.persisted_generation(), generation);
        assert!(!runtime.session.document().assembly.joint_bindings[0].needs_repair);
        assert!(runtime.assembly_projection().unwrap().joint_bindings[0].needs_repair);
    }

    #[test]
    fn solved_joint_commits_as_one_undo_and_roundtrips() {
        let (mut runtime, joints) = setup();
        let capture = runtime
            .prepare_joint_solve(
                &runtime.runtime_session(),
                joints.clone(),
                vec![(joints[0].id, 2.0)],
            )
            .unwrap();
        let SolveOutcome::Converged {
            poses,
            remaining_dof,
            ..
        } = solver::solve(&capture.request)
        else {
            panic!("solve failed")
        };
        assert_eq!(remaining_dof, 0);
        let before = runtime.undo_depth();
        let mut committed = joints;
        committed[0].value = 2.0;
        runtime
            .commit_joint_solve(&capture, committed, poses)
            .unwrap();
        assert_eq!(runtime.undo_depth(), before + 1);
        let document = runtime.session.document();
        assert_eq!(document.assembly.joints[0].value, 2.0);
        assert!(
            (document
                .assembly
                .occurrences
                .get(OccurrenceId(Uuid::from_u128(2)))
                .unwrap()
                .local
                .translation
                .z
                - 2.0)
                .abs()
                < 1e-6
        );
        let encoded = serde_json::to_vec(document).unwrap();
        let reopened: Document = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(reopened.assembly.joints[0].value, 2.0);
        let area = tempfile::tempdir().unwrap();
        let path = area.path().join("joints.onecad");
        let meta = onecad_core::io::container::SaveMeta {
            app_version: "test".into(),
            occt_fingerprint: None,
            created: String::new(),
            modified: String::new(),
        };
        onecad_core::io::container::ContainerWriter::save(
            &path,
            document,
            &onecad_core::io::container::ContainerCaches::none(),
            &meta,
        )
        .unwrap();
        let loaded = onecad_core::io::container::ContainerReader::open(&path).unwrap();
        assert_eq!(loaded.document().assembly.joints[0].value, 2.0);
        runtime.undo().unwrap();
        assert!(runtime.session.document().assembly.joints.is_empty());
        runtime.redo().unwrap().unwrap();
        assert_eq!(runtime.session.document().assembly.joints[0].value, 2.0);
    }

    #[test]
    fn untrusted_solved_pose_edit_is_refused_without_mutation() {
        let (mut runtime, joints) = setup();
        let before = serde_json::to_vec(runtime.session.document()).unwrap();
        let result =
            runtime.apply_assembly_edit(onecad_core::edit::AssemblyEdit::SetJointsAndPoses {
                joints,
                poses: vec![(OccurrenceId(Uuid::from_u128(1)), RigidTransform::IDENTITY)],
            });
        assert!(result.is_err());
        assert_eq!(
            before,
            serde_json::to_vec(runtime.session.document()).unwrap()
        );
    }

    #[test]
    fn same_revision_published_head_change_refuses_commit() {
        let (mut runtime, joints) = setup();
        let capture = runtime
            .prepare_joint_solve(&runtime.runtime_session(), joints.clone(), vec![])
            .unwrap();
        let SolveOutcome::Converged { poses, .. } = solver::solve(&capture.request) else {
            panic!("solve failed")
        };
        let snapshot = Arc::new(ModelSnapshot {
            id: SnapshotId(1),
            generation: 1,
            step_index: None,
            bodies: Vec::new(),
            stopped_reason: onecad_core::regen::StoppedReason::Completed,
            step_states: Vec::new(),
            signatures: None,
            diagnostics: Vec::new(),
            diagnostics_by_step: BTreeMap::new(),
            repair_summary: Default::default(),
            sketch_placement_by_step: BTreeMap::new(),
        });
        runtime.latest_snapshot = Some(snapshot);
        assert_eq!(runtime.revision(), capture.revision);
        assert!(runtime.commit_joint_solve(&capture, joints, poses).is_err());
    }

    #[test]
    fn replacement_and_epoch_change_fence_joint_commit() {
        let (mut runtime, joints) = setup();
        let capture = runtime
            .prepare_joint_solve(&runtime.runtime_session(), joints.clone(), vec![])
            .unwrap();
        let SolveOutcome::Converged { poses, .. } = solver::solve(&capture.request) else {
            panic!("solve failed")
        };
        runtime.fencing.set_epoch(capture.epoch.0 + 1);
        assert!(runtime
            .commit_joint_solve(&capture, joints.clone(), poses.clone())
            .is_err());
        let backend = Arc::new(crate::worker::PendingBackend);
        let mut replacement = DocumentRuntime::from_document(
            runtime.session.document().clone(),
            "same UUID".into(),
            None,
            false,
            backend.clone(),
            backend.clone(),
            backend,
        );
        assert_eq!(replacement.document_uuid(), runtime.document_uuid());
        assert!(replacement
            .commit_joint_solve(&capture, joints, poses)
            .is_err());
    }
}
