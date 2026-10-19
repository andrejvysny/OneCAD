use super::*;
use onecad_core::{document::variables::Scalar, edit::SketchEditOp, ids::ConstraintId};

impl SketchInteraction {
    fn mutation_ready(&self, key: &SketchSessionKey) -> bool {
        self.session.as_ref().is_some_and(|session| {
            session.key == *key
                && session.phase == SketchPhase::Active
                && session.draft.is_none()
                && self.context.as_ref() == Some(&session.document)
        })
    }

    pub(crate) fn set_dimension(
        &mut self,
        key: SketchSessionKey,
        constraint: ConstraintId,
        value: Scalar,
    ) -> Vec<SketchEffect> {
        if !self.mutation_ready(&key) {
            return Vec::new();
        }
        let dimensional = self.session.as_ref().is_some_and(|session| {
            session
                .authoritative
                .constraints()
                .iter()
                .any(|existing| existing.id() == constraint && existing.value().is_some())
        });
        if !dimensional || !value.value.is_finite() {
            return self.refuse(
                "Dimension requires an existing dimensional constraint and a finite value",
            );
        }
        self.submit(
            vec![SketchEditOp::SetDimension {
                constraint,
                value: value.clone(),
            }],
            AfterPrimitive::Preserve,
        )
        .into_iter()
        .map(|effect| match effect {
            SketchEffect::Upsert { tag, .. } => SketchEffect::SetDimension {
                tag,
                constraint,
                value: value.clone(),
            },
            other => other,
        })
        .collect()
    }

    pub(crate) fn repair(
        &mut self,
        key: SketchSessionKey,
        ops: Vec<SketchEditOp>,
    ) -> Vec<SketchEffect> {
        if !self.mutation_ready(&key) {
            return Vec::new();
        }
        if ops.is_empty()
            || ops.len() > 128
            || ops.iter().any(|op| {
                !matches!(
                    op,
                    SketchEditOp::RemoveEntity { .. }
                        | SketchEditOp::RemoveConstraint { .. }
                        | SketchEditOp::SetEntityConstruction { .. }
                )
            })
        {
            return self.refuse("Repair requires 1 to 128 removals or construction changes");
        }
        self.submit(ops.clone(), AfterPrimitive::Preserve)
            .into_iter()
            .map(|effect| match effect {
                SketchEffect::Upsert { tag, .. } => SketchEffect::Repair {
                    tag,
                    ops: ops.clone(),
                },
                other => other,
            })
            .collect()
    }
}
