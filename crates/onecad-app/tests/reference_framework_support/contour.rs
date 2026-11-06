use super::*;

pub(crate) async fn fillet_lower_front(h: &Harness, body: BodyId) {
    fillet_bottom_edge(h, body, 0.0, 0.8, 20).await;
}

pub(crate) async fn fillet_drafted_underside(h: &Harness, body: BodyId) {
    let y = 28.8 - 0.65 * 1.0_f64.to_radians().tan();
    fillet_bottom_edge(h, body, y, 0.3, 23).await;
}

async fn fillet_bottom_edge(h: &Harness, body: BodyId, y: f64, radius: f64, n: u128) {
    use onecad_app::worker::ElementQuery;
    use onecad_core::{
        document::{
            record::FilletParams,
            refs::{AnchorIntent, ElementKind, ElementRef, PrimaryRef},
            variables::Scalar,
        },
        ids::{ElementId, TopoKey},
        math::Vec3,
    };
    let worker = h.state.live_worker().unwrap();
    let mut candidates = Vec::new();
    for k in 1.. {
        let key = format!("e:{k}");
        let Ok(Some(dto)) = ElementQuery::classify_element_by_topo_key(&worker, body, &key).await
        else {
            break;
        };
        let Some(frame) = dto.frame.filter(|_| dto.curve_type == "line") else {
            continue;
        };
        let Some(axis) = frame.axis else { continue };
        if (frame.origin[1] - y).abs() < 1e-9
            && frame.origin[2].abs() < 1e-10
            && axis[0].abs() > 1.0 - 1e-12
        {
            candidates.push(key);
        }
    }
    assert_eq!(
        candidates.len(),
        1,
        "lower front edge must be unambiguous: {candidates:?}"
    );
    let key = TopoKey::new(&candidates[0]);
    let id = if key.is_valid() {
        let mut guard = h.state.runtime.lock().await;
        let runtime = guard.as_mut().unwrap();
        let result = runtime
            .promote_selection(runtime.head_snapshot_id().unwrap(), body, vec![(key, None)])
            .await
            .unwrap();
        ElementId::new(&result[0].element_id)
    } else {
        ElementId::new(key.as_str())
    };
    let reference = ElementRef {
        import_source_identity: None,
        primary: Some(PrimaryRef {
            body,
            element: id.clone(),
            kind: ElementKind::Edge,
            extra: Default::default(),
        }),
        intent: None,
        anchor: Some(AnchorIntent {
            world_point: Vec3::new_unchecked(0.0, y, 0.0),
            surface_uv: None,
            local_frame: None,
            adjacency_hint: None,
            extra: Default::default(),
        }),
        extra: Default::default(),
    };
    let params = FilletParams {
        radius: Scalar::new(radius),
        edge_ids: vec![id],
        edges: vec![reference],
        chain_tangent_edges: false,
        tangent_closure_version: None,
        overflow: None,
        overflow_version: None,
        extra: Default::default(),
    };
    let applied = api::apply_operation(
        &h.state,
        h.host.clone(),
        vec![EditCommand::AddOperation {
            record: OperationRecord::new(
                record(n),
                1,
                "Framework lower front radius",
                Operation::Known(KnownOperation::Fillet(params)),
            ),
            at_cursor: true,
        }],
        None,
    )
    .await
    .unwrap();
    h.settled(&applied.projection).await;
}

pub(crate) async fn fillet_upper_outer(h: &Harness, body: BodyId) {
    use onecad_app::worker::ElementQuery;
    use onecad_core::{
        document::{
            record::FilletParams,
            refs::{AnchorIntent, ElementKind, ElementRef, PrimaryRef},
            variables::Scalar,
        },
        ids::{ElementId, TopoKey},
        math::Vec3,
    };
    let worker = h.state.live_worker().unwrap();
    let mut keys = Vec::new();
    for k in 1.. {
        let key = format!("e:{k}");
        let Ok(Some(dto)) = ElementQuery::classify_element_by_topo_key(&worker, body, &key).await
        else {
            break;
        };
        let Some(f) = dto.frame.filter(|_| dto.curve_type == "line") else {
            continue;
        };
        let Some(axis) = f.axis else { continue };
        if (f.origin[2] - 6.8).abs() > 1e-10 {
            continue;
        }
        let anchor = if (f.origin[0].abs() - 15.0).abs() < 1e-10 && axis[1].abs() > 1.0 - 1e-12 {
            Some(Vec3::new_unchecked(f.origin[0], 16.0, 6.8))
        } else if (f.origin[1] - 32.0).abs() < 1e-10 && axis[0].abs() > 1.0 - 1e-12 {
            Some(Vec3::new_unchecked(0.0, 32.0, 6.8))
        } else {
            None
        };
        if let Some(anchor) = anchor {
            keys.push((key, anchor));
        }
    }
    assert_eq!(keys.len(), 3, "expected three outer upper edges: {keys:?}");
    let mut edges = Vec::new();
    for (key, at) in keys {
        let key = TopoKey::new(&key);
        let id = if key.is_valid() {
            let mut guard = h.state.runtime.lock().await;
            let runtime = guard.as_mut().unwrap();
            let result = runtime
                .promote_selection(runtime.head_snapshot_id().unwrap(), body, vec![(key, None)])
                .await
                .unwrap();
            ElementId::new(&result[0].element_id)
        } else {
            ElementId::new(key.as_str())
        };
        edges.push(ElementRef {
            import_source_identity: None,
            primary: Some(PrimaryRef {
                body,
                element: id,
                kind: ElementKind::Edge,
                extra: Default::default(),
            }),
            intent: None,
            anchor: Some(AnchorIntent {
                world_point: at,
                surface_uv: None,
                local_frame: None,
                adjacency_hint: None,
                extra: Default::default(),
            }),
            extra: Default::default(),
        });
    }
    let ids = edges
        .iter()
        .map(|edge| edge.primary.as_ref().unwrap().element.clone())
        .collect();
    let params = FilletParams {
        radius: Scalar::new(0.3),
        edge_ids: ids,
        edges,
        chain_tangent_edges: false,
        tangent_closure_version: None,
        overflow: None,
        overflow_version: None,
        extra: Default::default(),
    };
    let applied = api::apply_operation(
        &h.state,
        h.host.clone(),
        vec![EditCommand::AddOperation {
            record: OperationRecord::new(
                record(21),
                1,
                "Framework outer top radii",
                Operation::Known(KnownOperation::Fillet(params)),
            ),
            at_cursor: true,
        }],
        None,
    )
    .await
    .unwrap();
    h.settled(&applied.projection).await;
}

pub(crate) async fn fillet_connector_lip(h: &Harness, body: BodyId) {
    use onecad_app::worker::ElementQuery;
    use onecad_core::{
        document::{
            record::FilletParams,
            refs::{AnchorIntent, ElementKind, ElementRef, PrimaryRef},
            variables::Scalar,
        },
        ids::{ElementId, TopoKey},
        math::Vec3,
    };
    let worker = h.state.live_worker().unwrap();
    let mut keys = Vec::new();
    for k in 1.. {
        let key = format!("e:{k}");
        let Ok(Some(dto)) = ElementQuery::classify_element_by_topo_key(&worker, body, &key).await
        else {
            break;
        };
        let kind = dto.curve_type;
        let Some(f) = dto.frame else { continue };
        let Some(axis) = f.axis else { continue };
        if (f.origin[1] - 32.0).abs() > 1e-10 {
            continue;
        }
        let anchor = if kind == "line"
            && axis[0].abs() > 1.0 - 1e-12
            && ((f.origin[2] - 2.2).abs() < 1e-10 || (f.origin[2] - 4.8).abs() < 1e-10)
        {
            Some(Vec3::new_unchecked(0.0, 32.0, f.origin[2]))
        } else if kind == "circle"
            && axis[1].abs() > 1.0 - 1e-12
            && f.radius.is_some_and(|r| (r - 1.3).abs() < 1e-10)
            && (f.origin[0].abs() - 2.925).abs() < 1e-10
            && (f.origin[2] - 3.5).abs() < 1e-10
        {
            Some(Vec3::new_unchecked(
                f.origin[0] + 1.3 * f.origin[0].signum(),
                32.0,
                3.5,
            ))
        } else {
            None
        };
        if let Some(anchor) = anchor {
            keys.push((key, anchor));
        }
    }
    assert_eq!(keys.len(), 4, "connector exterior contour: {keys:?}");
    let mut edges = Vec::new();
    for (key, at) in keys {
        let key = TopoKey::new(&key);
        let id = if key.is_valid() {
            let mut guard = h.state.runtime.lock().await;
            let runtime = guard.as_mut().unwrap();
            let result = runtime
                .promote_selection(runtime.head_snapshot_id().unwrap(), body, vec![(key, None)])
                .await
                .unwrap();
            ElementId::new(&result[0].element_id)
        } else {
            ElementId::new(key.as_str())
        };
        edges.push(ElementRef {
            import_source_identity: None,
            primary: Some(PrimaryRef {
                body,
                element: id,
                kind: ElementKind::Edge,
                extra: Default::default(),
            }),
            intent: None,
            anchor: Some(AnchorIntent {
                world_point: at,
                surface_uv: None,
                local_frame: None,
                adjacency_hint: None,
                extra: Default::default(),
            }),
            extra: Default::default(),
        });
    }
    let ids = edges
        .iter()
        .map(|edge| edge.primary.as_ref().unwrap().element.clone())
        .collect();
    let params = FilletParams {
        radius: Scalar::new(0.2),
        edge_ids: ids,
        edges,
        chain_tangent_edges: false,
        tangent_closure_version: None,
        overflow: None,
        overflow_version: None,
        extra: Default::default(),
    };
    let applied = api::apply_operation(
        &h.state,
        h.host.clone(),
        vec![EditCommand::AddOperation {
            record: OperationRecord::new(
                record(22),
                1,
                "Framework connector exterior lip",
                Operation::Known(KnownOperation::Fillet(params)),
            ),
            at_cursor: true,
        }],
        None,
    )
    .await
    .unwrap();
    h.settled(&applied.projection).await;
}

pub(crate) async fn chamfer_rear_ramp(h: &Harness, body: BodyId) {
    use onecad_app::worker::ElementQuery;
    use onecad_core::{
        document::{
            record::{ChamferParams, ChamferReferenceFace},
            refs::{AnchorIntent, ElementKind, ElementRef, PrimaryRef},
            variables::Scalar,
        },
        ids::{ElementId, TopoKey},
        math::Vec3,
    };
    let at = Vec3::new_unchecked(0.0, 27.0, 1.2);
    let floor = promote_cap(h, body, at).await;
    let worker = h.state.live_worker().unwrap();
    let mut keys = Vec::new();
    for k in 1.. {
        let key = format!("e:{k}");
        let Ok(Some(dto)) = ElementQuery::classify_element_by_topo_key(&worker, body, &key).await
        else {
            break;
        };
        let kind = dto.curve_type;
        let Some(f) = dto.frame else { continue };
        let Some(axis) = f.axis else { continue };
        if (f.origin[2] - 1.2).abs() > 1e-9 {
            continue;
        }
        let anchor = if kind == "line"
            && (f.origin[0].abs() - 6.0).abs() < 1e-9
            && axis[1].abs() > 1.0 - 1e-12
        {
            Some(Vec3::new_unchecked(f.origin[0], 28.0, 1.2))
        } else if kind == "line" && (f.origin[1] - 24.5).abs() < 1e-9 && axis[0].abs() > 1.0 - 1e-12
        {
            Some(Vec3::new_unchecked(0.0, 24.5, 1.2))
        } else if kind == "circle"
            && (f.origin[0].abs() - 4.0).abs() < 1e-9
            && (f.origin[1] - 26.5).abs() < 1e-9
            && f.radius.is_some_and(|r| (r - 2.0).abs() < 1e-9)
        {
            Some(Vec3::new_unchecked(
                f.origin[0] + f.origin[0].signum() * std::f64::consts::SQRT_2,
                26.5 - std::f64::consts::SQRT_2,
                1.2,
            ))
        } else {
            None
        };
        if let Some(anchor) = anchor {
            keys.push((key, anchor));
        }
    }
    assert_eq!(keys.len(), 5, "rear ramp contour: {keys:?}");
    let mut edges = Vec::new();
    for (key, at) in keys {
        let key = TopoKey::new(&key);
        let id = if key.is_valid() {
            let mut guard = h.state.runtime.lock().await;
            let runtime = guard.as_mut().unwrap();
            let result = runtime
                .promote_selection(runtime.head_snapshot_id().unwrap(), body, vec![(key, None)])
                .await
                .unwrap();
            ElementId::new(&result[0].element_id)
        } else {
            ElementId::new(key.as_str())
        };
        edges.push(ElementRef {
            import_source_identity: None,
            primary: Some(PrimaryRef {
                body,
                element: id,
                kind: ElementKind::Edge,
                extra: Default::default(),
            }),
            intent: None,
            anchor: Some(AnchorIntent {
                world_point: at,
                surface_uv: None,
                local_frame: None,
                adjacency_hint: None,
                extra: Default::default(),
            }),
            extra: Default::default(),
        });
    }
    let face_ref = || ElementRef {
        import_source_identity: None,
        primary: Some(PrimaryRef {
            body,
            element: floor.clone(),
            kind: ElementKind::Face,
            extra: Default::default(),
        }),
        intent: None,
        anchor: Some(AnchorIntent {
            world_point: at,
            surface_uv: None,
            local_frame: None,
            adjacency_hint: None,
            extra: Default::default(),
        }),
        extra: Default::default(),
    };
    let ids: Vec<_> = edges
        .iter()
        .map(|edge| edge.primary.as_ref().unwrap().element.clone())
        .collect();
    let duplicate = std::env::var_os("ONECAD_FRAMEWORK_DUPLICATE_RAMP_REFS").is_some();
    let pair_ids = if duplicate { ids.as_slice() } else { &ids[..1] };
    let pairs = pair_ids
        .iter()
        .map(|edge_id| ChamferReferenceFace {
            edge_id: edge_id.clone(),
            face_id: floor.clone(),
        })
        .collect();
    let faces = pair_ids.iter().map(|_| face_ref()).collect();
    let params = ChamferParams {
        radius: Scalar::new(3.0),
        distance2: Some(Scalar::new(0.3)),
        angle_deg: None,
        edge_ids: ids,
        edges,
        reference_faces: pairs,
        reference_face_refs: faces,
        chain_tangent_edges: !duplicate,
        tangent_closure_version: (!duplicate).then_some(1),
        overflow: None,
        overflow_version: None,
        extra: Default::default(),
    };
    let applied = api::apply_operation(
        &h.state,
        h.host.clone(),
        vec![EditCommand::AddOperation {
            record: OperationRecord::new(
                record(70),
                1,
                "Framework rear pocket ramp",
                Operation::Known(KnownOperation::Chamfer(params)),
            ),
            at_cursor: true,
        }],
        None,
    )
    .await
    .unwrap();
    let terminal = h.terminal(&applied.projection).await;
    if !terminal.repair_steps.is_empty() {
        let guard = h.state.runtime.lock().await;
        eprintln!(
            "rear chamfer repair: {:?}",
            guard.as_ref().unwrap().repair_items()
        );
    }
    assert!(
        terminal.repair_steps.is_empty(),
        "rear chamfer: {terminal:?}"
    );
    assert!(
        terminal.failed_steps.is_empty(),
        "rear chamfer: {terminal:?}"
    );
}
