use super::*;

fn add_profile_line(ops: &mut Vec<SketchEditOp>, next: &mut u128, a: [f64; 2], b: [f64; 2]) {
    let first = EntityId(Uuid::from_u128(0xfac1000 + *next));
    let second = EntityId(Uuid::from_u128(0xfac1001 + *next));
    let curve = EntityId(Uuid::from_u128(0xfac1002 + *next));
    *next += 3;
    for (id, p) in [(first, a), (second, b)] {
        ops.push(SketchEditOp::AddEntity {
            entity: SketchEntity::point(id, Vec2::new_unchecked(p[0], p[1]), false, false),
        });
    }
    ops.push(SketchEditOp::AddEntity {
        entity: SketchEntity::line(curve, first, second, false),
    });
}

fn add_profile_arc(
    ops: &mut Vec<SketchEditOp>,
    next: &mut u128,
    center: [f64; 2],
    a: [f64; 2],
    b: [f64; 2],
) {
    let point = EntityId(Uuid::from_u128(0xfac1000 + *next));
    let arc = EntityId(Uuid::from_u128(0xfac1001 + *next));
    *next += 2;
    let angle = |p: [f64; 2]| (p[1] - center[1]).atan2(p[0] - center[0]);
    let mut start = angle(a);
    let mut end = angle(b);
    if (end - start).rem_euclid(std::f64::consts::TAU) > std::f64::consts::PI {
        std::mem::swap(&mut start, &mut end);
    }
    end = start + (end - start).rem_euclid(std::f64::consts::TAU);
    let radius = (a[0] - center[0]).hypot(a[1] - center[1]);
    ops.push(SketchEditOp::AddEntity {
        entity: SketchEntity::point(
            point,
            Vec2::new_unchecked(center[0], center[1]),
            false,
            false,
        ),
    });
    ops.push(SketchEditOp::AddEntity {
        entity: SketchEntity::arc(arc, point, radius, start, end, false).unwrap(),
    });
}

pub(crate) async fn connector_flare_profile(h: &Harness) -> onecad_app::dto::FinishSketchDto {
    let sid = sketch(100);
    let datum = DatumPlaneId(Uuid::from_u128(0xfac0300));
    api::apply_edit_command(
        &h.state,
        h.host.clone(),
        EditCommand::AddDatumPlane {
            datum: DatumPlane::offset_from_plane(datum, "Connector flare side", "XZ", -2.925),
        },
    )
    .await
    .unwrap();
    let mut sketch = Sketch::on_world_plane(sid, "Framework connector flare", WorldPlane::XZ);
    sketch.attachment = SketchAttachment::Datum { datum };
    api::apply_edit_command(&h.state, h.host.clone(), EditCommand::AddSketch { sketch })
        .await
        .unwrap();
    api::enter_sketch(&h.state, h.host.clone(), sid.to_string(), Some(true))
        .await
        .unwrap();
    let c1 = [30.15355183919, 4.66193088108];
    let c2 = [30.7, 5.730291497888];
    let v = [c2[0] - c1[0], c2[1] - c1[1]];
    let distance = f64::hypot(v[0], v[1]);
    let d = [c1[0] + v[0] / distance, c1[1] + v[1] / distance];
    let a = [31.4, 4.8];
    let b = [31.24, 4.88];
    let c = [30.95355183919, 5.26193088108];
    let e = [30.5, 5.730291497888];
    let mirror = |p: [f64; 2]| [p[0], 7.0 - p[1]];
    let mut ops = Vec::new();
    let mut n = 0;
    add_profile_arc(&mut ops, &mut n, [31.4, 5.0], a, b);
    add_profile_line(&mut ops, &mut n, b, c);
    add_profile_arc(&mut ops, &mut n, c1, c, d);
    add_profile_arc(&mut ops, &mut n, c2, d, e);
    add_profile_line(&mut ops, &mut n, e, mirror(e));
    add_profile_arc(&mut ops, &mut n, mirror(c2), mirror(e), mirror(d));
    add_profile_arc(&mut ops, &mut n, mirror(c1), mirror(d), mirror(c));
    add_profile_line(&mut ops, &mut n, mirror(c), mirror(b));
    add_profile_arc(&mut ops, &mut n, mirror([31.4, 5.0]), mirror(b), mirror(a));
    add_profile_line(&mut ops, &mut n, mirror(a), a);
    api::sketch_upsert(&h.state, h.host.clone(), sid.to_string(), ops)
        .await
        .unwrap();
    let finish = api::finish_sketch(&h.state, h.host.clone(), sid.to_string())
        .await
        .unwrap();
    h.settled(&h.projection().await).await;
    finish
}

pub(crate) async fn connector_end_profile(
    h: &Harness,
    n: u128,
    x: f64,
) -> (onecad_app::dto::FinishSketchDto, EntityId) {
    let sid = sketch(n);
    let datum = DatumPlaneId(Uuid::from_u128(0xfac0300 + n));
    api::apply_edit_command(
        &h.state,
        h.host.clone(),
        EditCommand::AddDatumPlane {
            datum: DatumPlane::offset_from_plane(datum, "Connector end plane", "XZ", x),
        },
    )
    .await
    .unwrap();
    let mut s = Sketch::on_world_plane(sid, "Connector end flare", WorldPlane::XZ);
    s.attachment = SketchAttachment::Datum { datum };
    api::apply_edit_command(
        &h.state,
        h.host.clone(),
        EditCommand::AddSketch { sketch: s },
    )
    .await
    .unwrap();
    api::enter_sketch(&h.state, h.host.clone(), sid.to_string(), Some(true))
        .await
        .unwrap();
    let c1 = [30.15355183919, 4.66193088108];
    let c2 = [30.7, 5.730291497888];
    let v = [c2[0] - c1[0], c2[1] - c1[1]];
    let dist = f64::hypot(v[0], v[1]);
    let d = [c1[0] + v[0] / dist, c1[1] + v[1] / dist];
    let a = [31.4, 4.8];
    let b = [31.24, 4.88];
    let c = [30.95355183919, 5.26193088108];
    let e = [30.5, 5.730291497888];
    let mut ops = Vec::new();
    let mut i = 0;
    add_profile_arc(&mut ops, &mut i, [31.4, 5.0], a, b);
    add_profile_line(&mut ops, &mut i, b, c);
    add_profile_arc(&mut ops, &mut i, c1, c, d);
    add_profile_arc(&mut ops, &mut i, c2, d, e);
    add_profile_line(&mut ops, &mut i, e, [30.5, 3.5]);
    add_profile_line(&mut ops, &mut i, [30.5, 3.5], [31.4, 3.5]);
    add_profile_line(&mut ops, &mut i, [31.4, 3.5], a);
    let p = EntityId(Uuid::from_u128(0xfac1100));
    let q = EntityId(Uuid::from_u128(0xfac1101));
    let axis = EntityId(Uuid::from_u128(0xfac1102));
    for (id, at) in [(p, [30.5, 3.5]), (q, [31.4, 3.5])] {
        ops.push(SketchEditOp::AddEntity {
            entity: SketchEntity::point(id, Vec2::new_unchecked(at[0], at[1]), true, false),
        });
    }
    ops.push(SketchEditOp::AddEntity {
        entity: SketchEntity::line(axis, p, q, true),
    });
    api::sketch_upsert(&h.state, h.host.clone(), sid.to_string(), ops)
        .await
        .unwrap();
    let finish = api::finish_sketch(&h.state, h.host.clone(), sid.to_string())
        .await
        .unwrap();
    h.settled(&h.projection().await).await;
    (finish, axis)
}
