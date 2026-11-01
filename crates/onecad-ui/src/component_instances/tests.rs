//! Pure behaviour of the instance view-model, review selection and request sequencing.
use super::{
    InstanceView, Mutated, Op, Reply, Scope,
    model::{self, Tone},
    review::{REPLANNED, Review, STALE},
    state::{Outcome, State},
};
use crate::library_catalog::testing::{component, iso_specs};
use onecad_app::dto::{
    ComponentInstanceDto, ComponentStatusDto, DroppedOverrideDto, UpgradeItemDto,
};
use onecad_core::document::record::ComponentParamValue as Value;
use std::collections::BTreeMap;

fn scope(session: &str) -> Scope {
    Scope {
        document_id: "doc".into(),
        runtime_session: session.into(),
    }
}

fn dto(id: &str, status: ComponentStatusDto) -> ComponentInstanceDto {
    let defaults = BTreeMap::from([
        ("thread".to_owned(), Value::Text("M6".into())),
        ("length".to_owned(), Value::Number(20.0)),
        ("thread_detail".to_owned(), Value::Text("cosmetic".into())),
    ]);
    let mut effective = defaults.clone();
    effective.insert("length".into(), Value::Number(30.0));
    ComponentInstanceDto {
        record_id: id.into(),
        name: format!("Screw {id}"),
        component_id: "iso".into(),
        component_version: "1.0.0".into(),
        source_kind: "generator".into(),
        linked: !matches!(status, ComponentStatusDto::Detached),
        designation: Some("ISO 4762 M6x30".into()),
        effective,
        defaults,
        overrides: vec!["length".into()],
        status,
    }
}

fn view(id: &str, status: ComponentStatusDto) -> InstanceView {
    InstanceView {
        dto: dto(id, status),
        component: Some(component("iso", "1.0.0", iso_specs())),
    }
}

fn item(id: &str, applicable: bool) -> UpgradeItemDto {
    UpgradeItemDto {
        record_id: id.into(),
        from_version: "1.0.0".into(),
        to_version: "1.1.0".into(),
        to_revision: String::new(),
        kept_overrides: BTreeMap::new(),
        dropped_overrides: vec![DroppedOverrideDto {
            key: "length".into(),
            value: Value::Number(30.0),
            reason: "below the new minimum".into(),
        }],
        dropped_mate_attachment: None,
        applicable,
        refusal: (!applicable).then(|| "definition changed".to_owned()),
    }
}

#[test]
fn rows_show_designation_link_and_status_badges() {
    let linked = dto("a", ComponentStatusDto::Linked);
    assert_eq!(model::title(&linked), "ISO 4762 M6x30");
    let mut plain = linked.clone();
    plain.designation = None;
    assert_eq!(model::title(&plain), "Screw a");
    assert_eq!(model::link_badge(&linked), ("Linked", Tone::Accent));
    let copy = dto("c", ComponentStatusDto::Detached);
    assert_eq!(model::link_badge(&copy), ("Copy", Tone::Neutral));
    let badge = |s| model::status_badge(&s);
    assert_eq!(badge(ComponentStatusDto::Linked), None);
    assert_eq!(badge(ComponentStatusDto::Detached), None);
    let newer = ComponentStatusDto::UpgradeAvailable {
        version: "1.1.0".into(),
    };
    assert_eq!(badge(newer), Some("Upgrade available"));
    assert_eq!(
        badge(ComponentStatusDto::DefinitionMissing),
        Some("Missing definition")
    );
    let unreadable = ComponentStatusDto::DefinitionUnreadable { reason: "x".into() };
    assert_eq!(badge(unreadable), Some("Unreadable"));
    let changed = ComponentStatusDto::RevisionMismatch {
        recorded: "a".into(),
        current: "b".into(),
    };
    assert_eq!(badge(changed), Some("Revision changed"));
}

#[test]
fn status_wording_never_promises_to_move_or_drop_geometry() {
    let missing = model::status_text(&dto("a", ComponentStatusDto::DefinitionMissing));
    assert!(missing.contains("geometry stays as placed"), "{missing}");
    assert!(missing.contains("no longer has this definition"));
    let newer = model::status_text(&dto(
        "a",
        ComponentStatusDto::UpgradeAvailable {
            version: "1.1.0".into(),
        },
    ));
    assert!(newer.contains("1.1.0") && newer.contains("stays on 1.0.0"));
    for status in [
        ComponentStatusDto::Linked,
        ComponentStatusDto::Detached,
        ComponentStatusDto::DefinitionUnreadable {
            reason: "bad toml".into(),
        },
        ComponentStatusDto::RevisionMismatch {
            recorded: "a".into(),
            current: "b".into(),
        },
    ] {
        let text = model::status_text(&dto("a", status));
        assert!(!text.to_lowercase().contains("delete"), "{text}");
    }
}

#[test]
fn only_linked_instances_with_a_definition_are_editable() {
    assert!(model::editable(&dto("a", ComponentStatusDto::Linked)));
    let newer = ComponentStatusDto::UpgradeAvailable {
        version: "1.1.0".into(),
    };
    assert!(model::editable(&dto("a", newer)));
    assert!(!model::editable(&dto("a", ComponentStatusDto::Detached)));
    assert!(!model::editable(&dto(
        "a",
        ComponentStatusDto::DefinitionMissing
    )));
    let unreadable = ComponentStatusDto::DefinitionUnreadable { reason: "x".into() };
    assert!(!model::editable(&dto("a", unreadable)));
    assert!(model::draft_for(&view("c", ComponentStatusDto::Detached)).is_none());
}

#[test]
fn parameter_rows_mark_overrides_against_their_defaults() {
    let rows = model::param_rows(&dto("a", ComponentStatusDto::Linked));
    let length = rows.iter().find(|r| r.key == "length").unwrap();
    assert!(length.overridden);
    assert_eq!(
        (length.effective.as_str(), length.default.as_deref()),
        ("30", Some("20"))
    );
    let thread = rows.iter().find(|r| r.key == "thread").unwrap();
    assert!(!thread.overridden);
    assert_eq!(thread.effective, "M6");
}

#[test]
fn the_draft_starts_at_the_effective_values_and_commits_a_delta_from_defaults() {
    let v = view("a", ComponentStatusDto::Linked);
    let mut draft = model::draft_for(&v).expect("editable");
    assert_eq!(draft.text("length"), "30");
    assert_eq!(draft.designation().as_deref(), Some("ISO 4762 M6x30"));
    assert!(!model::dirty(&draft, &v.dto.effective));
    assert!(
        model::pending_commit(&draft, &v.dto).is_none(),
        "unchanged commits nothing"
    );
    // Re-typing the same number in another spelling is not an edit.
    draft.set_text("length", "30.0".into());
    assert!(model::pending_commit(&draft, &v.dto).is_none());
    draft.set_text("length", "42".into());
    let Some(model::Commit::Set(map)) = model::pending_commit(&draft, &v.dto) else {
        panic!("a changed value commits a delta");
    };
    assert_eq!(
        map.len(),
        1,
        "only keys that differ from the default, never pinned defaults"
    );
    assert_eq!(map["length"], Value::Number(42.0));
    // Typing the package default back removes the override instead of pinning it.
    draft.set_text("length", "20".into());
    assert_eq!(
        model::pending_commit(&draft, &v.dto),
        Some(model::Commit::Reset(vec!["length".into()]))
    );
    // Changes go first; the restore follows as the next commit.
    draft.set_text("thread", "M8".into());
    let Some(model::Commit::Set(map)) = model::pending_commit(&draft, &v.dto) else {
        panic!("changes first");
    };
    assert_eq!(map.keys().collect::<Vec<_>>(), ["thread"]);
    draft.set_text("thread", "M6".into());
    // A refused value commits nothing.
    draft.set_text("length", "1".into());
    assert!(model::pending_commit(&draft, &v.dto).is_none());
    assert!(model::dirty(&draft, &v.dto.effective));
    assert!(draft.errors().contains_key("length"));
}

#[test]
fn review_selects_applicable_items_and_ignores_the_rest() {
    let mut review = Review::opening(vec!["a".into(), "b".into(), "c".into()]);
    assert!(review.loading);
    review.planned(vec![item("a", true), item("b", false), item("c", true)]);
    assert!(!review.loading);
    let ids =
        |items: Vec<UpgradeItemDto>| items.into_iter().map(|i| i.record_id).collect::<Vec<_>>();
    assert_eq!(ids(review.approved_selected()), ["a", "c"]);
    review.toggle("b");
    assert!(
        !review.selected.contains("b"),
        "a refused item cannot be selected"
    );
    review.toggle("a");
    assert_eq!(ids(review.approved_selected()), ["c"]);
    assert_eq!(ids(review.approved_all()), ["a", "c"]);
    review.toggle("a");
    assert_eq!(ids(review.approved_selected()), ["a", "c"]);
}

#[test]
fn a_replan_keeps_only_choices_that_still_apply() {
    let mut review = Review::opening(vec!["a".into(), "b".into()]);
    review.planned(vec![item("a", true), item("b", true)]);
    review.toggle("b");
    review.stale();
    assert!(review.loading);
    assert_eq!(review.notice.as_deref(), Some(REPLANNED));
    review.planned(vec![item("a", false), item("b", true)]);
    assert!(
        review.selected.is_empty(),
        "a was deselected-by-refusal, b was the user's choice off"
    );
    let mut again = Review::opening(vec!["a".into()]);
    again.planned(vec![item("a", true)]);
    again.stale();
    again.planned(vec![item("a", true)]);
    assert!(again.selected.contains("a"));
}

fn listed(state: &mut State, scope: &Scope, views: Vec<InstanceView>) {
    let sequence = state.take_outbox().last().expect("list queued").sequence;
    let outcome = state.accept(scope, sequence, Ok(Reply::Listed(views)));
    assert_eq!(outcome, Outcome::Listed);
}

#[test]
fn the_listing_refreshes_on_revision_scope_and_library_changes_only() {
    let mut state = State::default();
    let s = scope("one");
    state.sync(&s, 3, 0);
    assert_eq!(state.take_outbox().len(), 1);
    state.sync(&s, 3, 0);
    assert!(state.take_outbox().is_empty(), "same stamp asks nothing");
    state.sync(&s, 4, 0);
    assert_eq!(state.take_outbox().len(), 1);
    state.sync(&s, 4, 1);
    assert_eq!(state.take_outbox().len(), 1, "a reindex refreshes statuses");
}

#[test]
fn replies_for_another_document_or_an_older_listing_are_dropped() {
    let mut state = State::default();
    let s = scope("one");
    state.sync(&s, 1, 0);
    let first = state.take_outbox()[0].sequence;
    state.sync(&s, 2, 0);
    let second = state.take_outbox()[0].sequence;
    let views = vec![view("a", ComponentStatusDto::Linked)];
    assert_eq!(
        state.accept(&s, first, Ok(Reply::Listed(views.clone()))),
        Outcome::Dropped
    );
    assert!(state.items().is_empty());
    // The reply is for session one while the document now runs session two.
    let moved = scope("two");
    assert_eq!(
        state.accept(&moved, second, Ok(Reply::Listed(views.clone()))),
        Outcome::Dropped
    );
    assert!(state.items().is_empty());
    state.sync(&s, 2, 5);
    let third = state.take_outbox()[0].sequence;
    assert_eq!(
        state.accept(&s, third, Ok(Reply::Listed(views))),
        Outcome::Listed
    );
    assert_eq!(state.items().len(), 1);
    state.sync(&moved, 1, 0);
    assert!(
        state.items().is_empty(),
        "another document forgets the old listing"
    );
}

#[test]
fn one_mutation_at_a_time_and_a_refused_submit_settles_it() {
    let mut state = State::default();
    let s = scope("one");
    let detach = Op::Detach {
        record_id: "a".into(),
    };
    assert!(state.mutate(&s, "Detach component", detach.clone()));
    assert!(state.blocks());
    assert!(!state.mutate(&s, "Detach component", detach), "serialised");
    let sequence = state.take_outbox()[0].sequence;
    let outcome = state.refuse(&s, sequence, "Command queue full".into());
    assert!(matches!(
        outcome,
        Outcome::Failed {
            what: "Detach component",
            ..
        }
    ));
    assert!(!state.blocks());
}

#[test]
fn a_stale_upgrade_replans_the_same_instances_and_says_so() {
    let mut state = State::default();
    let s = scope("one");
    state.sync(&s, 1, 0);
    listed(
        &mut state,
        &s,
        vec![
            view(
                "a",
                ComponentStatusDto::UpgradeAvailable {
                    version: "1.1.0".into(),
                },
            ),
            view(
                "b",
                ComponentStatusDto::UpgradeAvailable {
                    version: "1.1.0".into(),
                },
            ),
            view("c", ComponentStatusDto::Linked),
        ],
    );
    state.open_review(&s);
    let plan = state.take_outbox();
    let Op::Plan { record_ids } = &plan[0].op else {
        panic!("plan")
    };
    assert_eq!(record_ids, &["a", "b"]);
    state.accept(
        &s,
        plan[0].sequence,
        Ok(Reply::Planned(vec![item("a", true), item("b", true)])),
    );
    let approved = state.review.as_ref().unwrap().approved_all();
    assert!(state.mutate(&s, "Update components", Op::Upgrade { approved }));
    let apply = state.take_outbox();
    let outcome = state.accept(&s, apply[0].sequence, Err(format!("x: {STALE}")));
    assert!(matches!(outcome, Outcome::Failed { .. }));
    assert!(!state.blocks());
    let review = state.review.as_ref().expect("the review stays open");
    assert!(review.loading);
    assert_eq!(review.notice.as_deref(), Some(REPLANNED));
    let again = state.take_outbox();
    let Op::Plan { record_ids } = &again[0].op else {
        panic!("re-plan")
    };
    assert_eq!(record_ids, &["a", "b"]);
}

#[test]
fn a_committed_upgrade_closes_the_review_and_a_failed_one_keeps_it_with_the_reason() {
    let mut state = State::default();
    let s = scope("one");
    state.review = Some(Review::default());
    state.mutate(
        &s,
        "Update components",
        Op::Upgrade {
            approved: vec![item("a", true)],
        },
    );
    let sequence = state.take_outbox()[0].sequence;
    let outcome = state.accept(&s, sequence, Err("worker unavailable".into()));
    assert!(matches!(outcome, Outcome::Failed { .. }));
    assert_eq!(
        state.review.as_ref().unwrap().error.as_deref(),
        Some("worker unavailable")
    );
    state.mutate(
        &s,
        "Update components",
        Op::Upgrade {
            approved: vec![item("a", true)],
        },
    );
    let sequence = state.take_outbox()[0].sequence;
    let done = Mutated {
        what: "Update components",
        noop: false,
        failure: None,
    };
    let outcome = state.accept(&s, sequence, Ok(Reply::Mutated(Box::new(done.clone()))));
    assert_eq!(outcome, Outcome::Mutated(done));
    assert!(state.review.is_none());
}

#[test]
fn the_edit_follows_the_listing_unless_the_user_has_typed() {
    let mut state = State::default();
    let s = scope("one");
    state.sync(&s, 1, 0);
    listed(&mut state, &s, vec![view("a", ComponentStatusDto::Linked)]);
    assert_eq!(state.edit_for("a").unwrap().draft.text("length"), "30");
    state
        .edit_for("a")
        .unwrap()
        .draft
        .set_text("length", "55".into());
    state.sync(&s, 2, 0);
    let mut newer = view("a", ComponentStatusDto::Linked);
    newer
        .dto
        .effective
        .insert("length".into(), Value::Number(40.0));
    listed(&mut state, &s, vec![newer.clone()]);
    assert_eq!(
        state.edit_for("a").unwrap().draft.text("length"),
        "55",
        "typing is kept"
    );
    state
        .edit_for("a")
        .unwrap()
        .draft
        .set_text("length", "30".into());
    state.sync(&s, 3, 0);
    newer
        .dto
        .effective
        .insert("length".into(), Value::Number(41.0));
    listed(&mut state, &s, vec![newer]);
    assert_eq!(state.edit_for("a").unwrap().draft.text("length"), "41");
    assert!(state.edit_for("missing").is_none());
}

#[test]
fn edits_wait_for_the_listing_of_the_current_revision() {
    let mut state = State::default();
    let s = scope("one");
    state.sync(&s, 1, 0);
    assert!(!state.settled(1), "list in flight");
    listed(&mut state, &s, vec![]);
    assert!(state.settled(1));
    assert!(
        !state.settled(2),
        "a newer revision has not been listed yet"
    );
}
