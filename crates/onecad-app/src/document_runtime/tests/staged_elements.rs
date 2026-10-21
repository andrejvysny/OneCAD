use super::*;

fn edge_entry(body: BodyId) -> ElementEntry {
    ElementEntry::new(body, ElementKind::Edge)
}

#[test]
fn leak_guard_strips_unadopted_staged_ids_and_keeps_recorded_ones() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let body = BodyId::new();
    let (leaked, recorded) = (ElementId::new("el_leaked"), ElementId::new("el_recorded"));
    rt.staged.insert(leaked.clone(), edge_entry(body));
    rt.staged.insert(recorded.clone(), edge_entry(body));
    // A persisting writer (face entry, legacy promotion) recorded this one.
    rt.regen.elements.insert(recorded.clone(), edge_entry(body));
    let mut scratch = RegenSession::with_timeline(rt.regen.timeline.clone());
    scratch.elements = rt.regen.elements.clone();
    // A regen delta folded both into the scratch mirror.
    scratch.elements.insert(leaked.clone(), edge_entry(body));
    rt.guard_staged_leaks(&mut scratch);
    assert!(!scratch.elements.contains(&leaked));
    assert!(scratch.elements.contains(&recorded));
}

#[test]
fn commit_adopts_referenced_staged_ids_and_failed_apply_adopts_nothing() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let body = BodyId::new();
    let (used, unused) = (ElementId::new("el_used"), ElementId::new("el_unused"));
    rt.staged.insert(used.clone(), edge_entry(body));
    rt.staged.insert(unused.clone(), edge_entry(body));
    let record = fillet_record(
        0x315E,
        body,
        used.as_str(),
        Vec3::new(0.0, 0.0, 0.0).unwrap(),
    );
    rt.read_only = true;
    assert!(rt
        .apply(EditCommand::AddOperation {
            record: record.clone(),
            at_cursor: true,
        })
        .is_err());
    assert!(rt.staged.contains_key(&used) && !rt.regen.elements.contains(&used));
    rt.read_only = false;
    rt.apply(EditCommand::AddOperation {
        record,
        at_cursor: true,
    })
    .expect("apply");
    assert!(rt.regen.elements.contains(&used) && !rt.staged.contains_key(&used));
    assert!(rt.staged.contains_key(&unused) && !rt.regen.elements.contains(&unused));
}

#[test]
fn transaction_adopts_staged_ids_only_after_it_commits() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let body = BodyId::new();
    let used = ElementId::new("el_tx");
    rt.staged.insert(used.clone(), edge_entry(body));
    let add = |seed| EditCommand::AddOperation {
        record: fillet_record(seed, body, "el_tx", Vec3::new(0.0, 0.0, 0.0).unwrap()),
        at_cursor: true,
    };
    // A duplicate record id rejects the second member and rolls the whole back.
    assert!(rt
        .apply_transaction("Fillet", vec![add(0x315F), add(0x315F)])
        .is_err());
    assert!(rt.staged.contains_key(&used));
    rt.apply_transaction("Fillet", vec![add(0x3160)])
        .expect("transaction");
    assert!(rt.regen.elements.contains(&used) && !rt.staged.contains_key(&used));
}

/// The UI scope of the current head, as `api::preview_context` would capture it.
fn head_scope(rt: &DocumentRuntime) -> crate::read::PreviewContext {
    crate::read::PreviewContext {
        document_id: rt.document_id(),
        runtime_session: rt.runtime_session(),
        revision: rt.revision(),
        worker_epoch: rt.fencing.get().1,
        snapshot_id: rt.head_snapshot_id().expect("published head"),
        expected_base_hash: onecad_core::regen::HistoryPrefixHash::new(""),
    }
}

fn delta_entry(id: &ElementId, key: &str, body: BodyId) -> onecad_core::regen::ElementMapEntry {
    onecad_core::regen::ElementMapEntry {
        element_id: id.clone(),
        topo_key: TopoKey::new(key),
        kind: ElementKind::Edge,
        body,
    }
}

/// Drives the guard through its production call site: a worker that folds a
/// staged-but-unadopted id into a published delta must not reach `regen.elements`
/// or the save payload, while an id a persisting writer recorded survives.
#[tokio::test]
async fn published_regen_strips_a_folded_staged_id_and_keeps_a_promoted_one() {
    let backend = Arc::new(FakeBackend::new());
    let mut rt = runtime_with(backend.clone());
    rt.apply(add_extrude(0x10, 25.0)).unwrap();
    let report = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    assert!(matches!(report.outcome, Outcome::Published(_)));
    let body = BodyId(Uuid::from_u128(0x10));
    let ticket = rt.stage_ticket(&head_scope(&rt)).expect("current scope");
    let staged = rt
        .stage_selection(
            &ticket,
            body,
            vec![TopoKey::new("e:1"), TopoKey::new("e:2")],
        )
        .await
        .expect("stage two edges");
    let id = |i: usize| staged[i].primary.as_ref().unwrap().element.clone();
    let (cancelled, recorded) = (id(0), id(1));
    let head = rt.head_snapshot_id().unwrap();
    let promoted = rt
        .promote_selection(head, body, vec![(TopoKey::new("e:2"), None)])
        .await
        .expect("legacy promotion records the second edge");
    assert_eq!(promoted[0].element_id, recorded.as_str());
    assert_eq!(rt.staged_element_count(), 1);
    *backend.step_delta.lock().unwrap() = ElementMapDelta {
        added: vec![delta_entry(&cancelled, "e:1", body)],
        removed: Vec::new(),
        relabeled: vec![delta_entry(&recorded, "e:2", body)],
    };
    rt.apply(add_extrude(0x11, 10.0)).unwrap();
    let report = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    assert!(matches!(report.outcome, Outcome::Published(_)));
    assert!(!rt.regen.elements.contains(&cancelled), "guard stripped it");
    assert!(
        rt.regen.elements.contains(&recorded),
        "recorded id survives"
    );
    assert_eq!(
        rt.staged_element_count(),
        1,
        "the cancelled stage stays staged"
    );
    let payload = rt.build_save_payload(test_save_meta(), SaveCaches::explicit());
    assert!(!payload.doc.elements.contains(&cancelled));
    assert!(payload.doc.elements.contains(&recorded));
}

fn host_face(id: &str, body: BodyId) -> ElementRef {
    ElementRef {
        primary: Some(PrimaryRef {
            body,
            element: ElementId::new(id),
            kind: ElementKind::Face,
            extra: Default::default(),
        }),
        intent: None,
        anchor: None,
        extra: Default::default(),
    }
}

#[test]
fn every_command_that_persists_an_element_id_reports_its_staged_ids() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let body = BodyId::new();
    for id in ["el_sketch", "el_attach", "el_input", "el_color"] {
        rt.staged.insert(ElementId::new(id), edge_entry(body));
    }
    let mut sketch = Sketch::on_world_plane(SketchId::new(), "On face", WorldPlane::XY);
    sketch.attachment = SketchAttachment::HostFace {
        face: host_face("el_sketch", body),
        projected_boundary_version: 0,
    };
    let plane = sketch.plane;
    let cases = [
        ("el_sketch", EditCommand::AddSketch { sketch }),
        (
            "el_attach",
            EditCommand::UpdateSketchAttachment {
                sketch: SketchId::new(),
                plane,
                attachment: SketchAttachment::HostFace {
                    face: host_face("el_attach", body),
                    projected_boundary_version: 1,
                },
            },
        ),
        (
            "el_input",
            EditCommand::EditOperationInput {
                record: RecordId::new(),
                path: InputPath::FilletEdges { index: 0 },
                reference: InputRef::Element(host_face("el_input", body)),
            },
        ),
        (
            "el_color",
            EditCommand::SetFaceColor {
                body,
                element_id: ElementId::new("el_color"),
                color: Some([1, 2, 3, 255]),
            },
        ),
    ];
    for (expected, command) in cases {
        assert_eq!(
            rt.staged_ids_in(std::slice::from_ref(&command)),
            [ElementId::new(expected)],
            "{}",
            command.label()
        );
    }
    let unstaged = EditCommand::SetFaceColor {
        body,
        element_id: ElementId::new("el_persisted"),
        color: None,
    };
    assert!(rt.staged_ids_in(&[unstaged]).is_empty());
}
