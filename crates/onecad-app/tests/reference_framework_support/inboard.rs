use super::*;
use onecad_app::worker::ElementQuery;
use onecad_core::{
    document::{
        record::FilletParams,
        refs::{AnchorIntent, ElementKind, ElementRef, PrimaryRef},
    },
    ids::{ElementId, TopoKey},
    math::Vec3,
};

type Sections = [(f64, (Vec<[f64; 2]>, Vec<[f64; 2]>)); 2];

fn source_edges(sections: &Sections) -> [([f64; 3], [f64; 3]); 2] {
    let points = sections.each_ref().map(|(x, (triangle, limiter))| {
        let start = limiter[0][0];
        let tip = triangle[0];
        let at = |end: [f64; 2]| tip[1] + (start - tip[0]) * (end[1] - tip[1]) / (end[0] - tip[0]);
        [[*x, start, at(triangle[1])], [*x, start, at(triangle[2])]]
    });
    [(points[0][0], points[1][0]), (points[0][1], points[1][1])]
}

pub(crate) async fn fillet_inboard_tool(h: &Harness, body: BodyId, sections: &Sections, n: u128) {
    let selected = find_edges(h, body, source_edges(sections), "other").await;
    apply_fillet(h, body, selected, n).await;
}

async fn find_edges(
    h: &Harness,
    body: BodyId,
    expected: [([f64; 3], [f64; 3]); 2],
    curve_type: &str,
) -> Vec<(String, [f64; 3])> {
    let worker = h.state.live_worker().unwrap();
    let snapshot = h
        .state
        .runtime
        .lock()
        .await
        .as_ref()
        .unwrap()
        .head_snapshot_id()
        .unwrap();
    let mut selected = Vec::new();
    for k in 1.. {
        let key = format!("e:{k}");
        let Ok(Some(dto)) = ElementQuery::classify_element_by_topo_key(&worker, body, &key).await
        else {
            break;
        };
        if dto.curve_type != curve_type {
            continue;
        }
        let desc = worker
            .query_element_by_topo_key(snapshot, body, &key)
            .await
            .unwrap()
            .unwrap();
        for (a, b) in expected {
            let midpoint = std::array::from_fn::<_, 3, _>(|i| (a[i] + b[i]) / 2.0);
            let length = (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f64>().sqrt();
            let error = (0..3)
                .map(|i| (desc.center[i] - midpoint[i]).powi(2))
                .sum::<f64>();
            if error < 1e-10 && (desc.magnitude - length).abs() < 1e-5 {
                selected.push((key.clone(), desc.center));
            }
        }
    }
    assert_eq!(selected.len(), 2, "tool requires two source-plane edges");
    selected
}

async fn apply_fillet(h: &Harness, body: BodyId, selected: Vec<(String, [f64; 3])>, n: u128) {
    let mut edges = Vec::new();
    for (key, center) in selected {
        let mut guard = h.state.runtime.lock().await;
        let runtime = guard.as_mut().unwrap();
        let result = runtime
            .promote_selection(
                runtime.head_snapshot_id().unwrap(),
                body,
                vec![(TopoKey::new(&key), None)],
            )
            .await
            .unwrap();
        let id = ElementId::new(&result[0].element_id);
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
                world_point: Vec3::new_unchecked(center[0], center[1], center[2]),
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
                record(n),
                1,
                "Framework inboard cutter fillet",
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
