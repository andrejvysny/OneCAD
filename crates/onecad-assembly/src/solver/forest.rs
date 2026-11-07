//! Exact local rank certificate for regular, acyclic relative-pose constraints.
use super::*;

fn root(parents: &[usize], mut body: usize) -> usize {
    while parents[body] != body {
        body = parents[body];
    }
    body
}

// A loose requested solve tolerance must not certify a singular rotation chart.
fn regular(poses: &[RigidTransform], constraint: &Constraint) -> bool {
    let (a, b) = joint_frames(poses, constraint);
    let Some(relative) = (a.rotation.conjugate() * b.rotation).normalized() else {
        return false;
    };
    let transverse = relative.x * relative.x + relative.y * relative.y;
    let off_axis = match constraint.kind {
        JointKind::Revolute => transverse,
        JointKind::Rigid | JointKind::Slider => transverse + relative.z * relative.z,
    };
    if off_axis > RANK_REL_TOL {
        return false;
    }
    constraint.kind != JointKind::Revolute
        || constraint.target.is_none_or(|target| {
            wrap_angle(relative_angle(a.rotation, b.rotation) - target).abs() <= RANK_REL_TOL
        })
}

pub(super) fn dof(poses: &[RigidTransform], problem: &Problem) -> Option<(usize, usize)> {
    let mut parents: Vec<_> = (0..problem.layout.offsets.len()).collect();
    let mut grounds: Vec<_> = problem
        .layout
        .offsets
        .iter()
        .map(|offset| usize::from(offset.is_none()))
        .collect();
    let mut rank = 0usize;
    for constraint in problem.cons {
        if !regular(poses, constraint) {
            return None;
        }
        let a = root(&parents, constraint.a);
        let b = root(&parents, constraint.b);
        if a == b || grounds[a] + grounds[b] > 1 {
            return None;
        }
        parents[b] = a;
        grounds[a] += grounds[b];
        let codimension = match constraint.kind {
            JointKind::Rigid => 6,
            JointKind::Revolute | JointKind::Slider => 5 + usize::from(constraint.target.is_some()),
        };
        rank = rank.checked_add(codimension)?;
    }
    // Root each tree at its ground, or retain six global DOFs if floating. Each
    // edge introduces a fresh body's six relative coordinates with independent
    // 5/6 rows. No cycles or second ground can create cross-edge dependencies.
    problem
        .layout
        .unknowns
        .checked_sub(rank)
        .map(|free| (free, 0))
}
