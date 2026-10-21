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
