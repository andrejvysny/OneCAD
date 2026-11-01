//! Catalog store: lane sequencing, filters, detail selection and preview fencing.
use super::{
    Lane, PreviewState, Reply, Request, Source, Status,
    testing::{component, iso_specs},
};
use onecad_app::dto::{LibraryComponentDto, SearchSortDto};
use onecad_core::document::record::ComponentParamValue as Value;

fn ctx() -> egui::Context {
    egui::Context::default()
}

fn hit(id: &str) -> LibraryComponentDto {
    component(id, "1.0.0", iso_specs())
}

fn sequence_of(request: &Request) -> u64 {
    request.sequence()
}

#[test]
fn first_prepare_loads_once_with_latest_only_name_sort() {
    let mut catalog = super::Catalog::default();
    catalog.prepare();
    catalog.prepare();
    let queued = catalog.take_outbox();
    assert_eq!(queued.len(), 1);
    let Request::Search { query, .. } = &queued[0] else {
        panic!("expected a search, got {queued:?}");
    };
    assert!(query.latest_only && query.text.is_none() && query.category.is_none());
    assert_eq!(query.sort, SearchSortDto::Name);
    assert_eq!(*catalog.search_status(), Status::Loading);
    assert!(!catalog.loaded());
}

#[test]
fn a_stale_search_reply_is_dropped_and_the_newest_is_applied() {
    let ctx = ctx();
    let mut catalog = super::Catalog::default();
    catalog.prepare();
    let first = sequence_of(&catalog.take_outbox()[0]);
    catalog.query.text = "scr".into();
    catalog.requery();
    catalog.prepare();
    let second = sequence_of(&catalog.take_outbox()[0]);
    assert!(second > first);

    assert!(!catalog.accept(first, Ok(Reply::Searched(vec![hit("old")])), &ctx));
    assert!(catalog.results().is_empty());
    assert_eq!(*catalog.search_status(), Status::Loading);

    assert!(catalog.accept(second, Ok(Reply::Searched(vec![hit("new")])), &ctx));
    assert_eq!(catalog.results()[0].id, "new");
    assert!(catalog.loaded());
    // A reply is applied once.
    assert!(!catalog.accept(second, Ok(Reply::Searched(vec![hit("again")])), &ctx));
    assert_eq!(catalog.results()[0].id, "new");
}

#[test]
fn query_text_is_trimmed_and_filters_reach_the_request() {
    let mut catalog = super::Catalog::default();
    catalog.prepare();
    catalog.take_outbox();
    catalog.query.text = "  m6  ".into();
    catalog.query.category = Some("fasteners".into());
    catalog.query.source = Source::Generator;
    catalog.query.sort = SearchSortDto::Id;
    catalog.requery();
    catalog.prepare();
    let queued = catalog.take_outbox();
    let Request::Search { query, .. } = &queued[0] else {
        panic!("expected a search");
    };
    assert_eq!(query.text.as_deref(), Some("m6"));
    assert_eq!(query.category.as_deref(), Some("fasteners"));
    assert_eq!(query.source_kind.as_deref(), Some("generator"));
    assert_eq!(query.sort, SearchSortDto::Id);
    assert!(catalog.has_filters());
    catalog.clear_filters();
    assert!(!catalog.has_filters());
}

#[test]
fn a_failed_search_clears_results_and_reports_the_error() {
    let ctx = ctx();
    let mut catalog = super::Catalog::default();
    catalog.prepare();
    let seq = sequence_of(&catalog.take_outbox()[0]);
    catalog.accept(seq, Ok(Reply::Searched(vec![hit("a")])), &ctx);
    catalog.requery();
    catalog.prepare();
    let seq = sequence_of(&catalog.take_outbox()[0]);
    catalog.accept(seq, Err("index unreadable".into()), &ctx);
    assert!(catalog.results().is_empty());
    assert_eq!(
        *catalog.search_status(),
        Status::Failed("index unreadable".into())
    );
}

#[test]
fn categories_survive_a_narrowing_filter() {
    let ctx = ctx();
    let mut catalog = super::Catalog::default();
    catalog.prepare();
    let seq = sequence_of(&catalog.take_outbox()[0]);
    let mut bearing = hit("b");
    bearing.category = vec!["bearings".into()];
    catalog.accept(seq, Ok(Reply::Searched(vec![hit("a"), bearing])), &ctx);
    catalog.query.category = Some("fasteners".into());
    catalog.requery();
    catalog.prepare();
    let seq = sequence_of(&catalog.take_outbox()[0]);
    catalog.accept(seq, Ok(Reply::Searched(vec![hit("a")])), &ctx);
    assert_eq!(
        catalog.categories().collect::<Vec<_>>(),
        ["bearings", "fasteners"]
    );
}

#[test]
fn detail_reply_for_a_replaced_selection_is_dropped() {
    let ctx = ctx();
    let mut catalog = super::Catalog::default();
    catalog.open_detail("one", "1.0.0");
    let first = sequence_of(&catalog.take_outbox()[0]);
    catalog.open_detail("two", "1.0.0");
    let second = sequence_of(&catalog.take_outbox()[0]);
    let detail = |id: &str| {
        Reply::Detail(Box::new(onecad_app::dto::ComponentDetailDto {
            component: hit(id),
            versions: vec!["1.0.0".into(), "1.1.0".into()],
        }))
    };
    assert!(!catalog.accept(first, Ok(detail("one")), &ctx));
    assert!(catalog.detail().expect("open").component.is_none());
    assert!(catalog.accept(second, Ok(detail("two")), &ctx));
    let view = catalog.detail().expect("open");
    assert_eq!(view.versions, ["1.0.0", "1.1.0"]);
    assert_eq!(
        view.draft.as_ref().and_then(|d| d.designation()).as_deref(),
        Some("ISO 4762 M6x20")
    );
    assert_eq!(view.status, Status::Idle);
}

#[test]
fn editing_the_draft_invalidates_a_preview_in_flight() {
    let ctx = ctx();
    let mut catalog = super::Catalog::default();
    catalog.prepare();
    let seq = sequence_of(&catalog.take_outbox()[0]);
    catalog.accept(seq, Ok(Reply::Searched(vec![hit("iso")])), &ctx);
    catalog.open_detail("iso", "1.0.0");
    catalog.take_outbox();
    catalog.request_preview();
    let queued = catalog.take_outbox();
    let Request::Preview {
        sequence, params, ..
    } = &queued[0]
    else {
        panic!("expected a preview, got {queued:?}");
    };
    assert_eq!(params["length"], Value::Number(20.0));
    assert_eq!(*catalog.preview(), PreviewState::Running);

    catalog
        .detail_mut()
        .and_then(|v| v.draft.as_mut())
        .expect("draft")
        .set_text("length", "30".into());
    catalog.draft_edited();
    let summary = super::PreviewSummary {
        size: [1.0; 3],
        bodies: 1,
        triangles: 12,
    };
    assert!(!catalog.accept(*sequence, Ok(Reply::Previewed(summary)), &ctx));
    assert_eq!(*catalog.preview(), PreviewState::Idle);
}

#[test]
fn an_invalid_draft_sends_no_preview() {
    let ctx = ctx();
    let mut catalog = super::Catalog::default();
    catalog.prepare();
    let seq = sequence_of(&catalog.take_outbox()[0]);
    catalog.accept(seq, Ok(Reply::Searched(vec![hit("iso")])), &ctx);
    catalog.open_detail("iso", "1.0.0");
    catalog.take_outbox();
    let draft = catalog
        .detail_mut()
        .and_then(|v| v.draft.as_mut())
        .expect("draft");
    draft.set_text("thread", "M7".into());
    catalog.request_preview();
    assert!(catalog.take_outbox().is_empty());
    assert_eq!(*catalog.preview(), PreviewState::Idle);
}

#[test]
fn reindex_reply_requeries_and_a_refused_submission_fails_the_lane() {
    let ctx = ctx();
    let mut catalog = super::Catalog::default();
    catalog.prepare();
    let seq = sequence_of(&catalog.take_outbox()[0]);
    catalog.accept(seq, Ok(Reply::Searched(vec![hit("a")])), &ctx);
    catalog.reindex();
    let reindex = catalog.take_outbox();
    assert_eq!(reindex[0].lane(), Lane::Reindex);
    let report = onecad_app::dto::ReindexReportDto {
        total: 2,
        indexed: 1,
        skipped: vec!["bad: x".into()],
    };
    catalog.accept(reindex[0].sequence(), Ok(Reply::Reindexed(report)), &ctx);
    catalog.prepare();
    assert_eq!(catalog.take_outbox()[0].lane(), Lane::Search);

    catalog.reindex();
    let seq = sequence_of(&catalog.take_outbox()[0]);
    catalog.accept(
        seq,
        Err("Command queue full; wait for current operation".into()),
        &ctx,
    );
    assert!(
        matches!(catalog.reindex_state(), super::ReindexState::Failed(e) if e.contains("queue full"))
    );
}

#[test]
fn thumbnails_are_requested_once_and_failures_fall_back() {
    let ctx = ctx();
    let mut catalog = super::Catalog::default();
    catalog.prepare();
    let seq = sequence_of(&catalog.take_outbox()[0]);
    let mut with_preview = hit("pic");
    with_preview.preview_path = Some("/nonexistent/preview.png".into());
    catalog.accept(
        seq,
        Ok(Reply::Searched(vec![with_preview, hit("plain")])),
        &ctx,
    );
    catalog.prepare();
    let queued = catalog.take_outbox();
    let Request::Thumbnails { sequence, items } = &queued[0] else {
        panic!("expected thumbnails");
    };
    assert_eq!(items.len(), 1);
    // One batch in flight at a time; nothing is re-requested meanwhile.
    catalog.prepare();
    assert!(catalog.take_outbox().is_empty());
    catalog.accept(*sequence, Err("decode stopped".into()), &ctx);
    catalog.prepare();
    assert!(
        catalog.take_outbox().is_empty(),
        "a failed preview is not retried"
    );
    assert!(catalog.texture("pic", "1.0.0").is_none());
}

fn thumbnail_hit(id: &str) -> LibraryComponentDto {
    let mut c = hit(id);
    c.preview_path = Some(format!("/nonexistent/{id}.png"));
    c
}

#[test]
fn duplicate_hits_queue_one_thumbnail() {
    let ctx = ctx();
    let mut catalog = super::Catalog::default();
    catalog.prepare();
    let seq = sequence_of(&catalog.take_outbox()[0]);
    let hits = vec![thumbnail_hit("dup"), thumbnail_hit("dup")];
    catalog.accept(seq, Ok(Reply::Searched(hits)), &ctx);
    catalog.prepare();
    let queued = catalog.take_outbox();
    let Request::Thumbnails { items, .. } = &queued[0] else {
        panic!("expected thumbnails");
    };
    assert_eq!(items.len(), 1);
}

#[test]
fn an_unanswered_thumbnail_batch_is_abandoned_and_its_late_reply_dropped() {
    let ctx = ctx();
    let mut catalog = super::Catalog::default();
    catalog.prepare();
    let seq = sequence_of(&catalog.take_outbox()[0]);
    catalog.accept(seq, Ok(Reply::Searched(vec![thumbnail_hit("a")])), &ctx);
    catalog.prepare();
    let first = sequence_of(&catalog.take_outbox()[0]);
    catalog.prepare();
    assert!(catalog.take_outbox().is_empty(), "one batch in flight");
    catalog.prepare_at(std::time::Instant::now() + std::time::Duration::from_secs(11));
    let again = catalog.take_outbox();
    assert_eq!(again.len(), 1, "the abandoned key is requested again");
    assert!(again[0].sequence() > first);
    assert!(!catalog.accept(first, Ok(Reply::Thumbnails(vec![])), &ctx));
}

#[test]
fn reindex_drops_thumbnail_replies_from_before_it() {
    let ctx = ctx();
    let mut catalog = super::Catalog::default();
    catalog.prepare();
    let seq = sequence_of(&catalog.take_outbox()[0]);
    catalog.accept(seq, Ok(Reply::Searched(vec![thumbnail_hit("a")])), &ctx);
    catalog.prepare();
    let batch = sequence_of(&catalog.take_outbox()[0]);
    catalog.reindex();
    let reindex = sequence_of(&catalog.take_outbox()[0]);
    let report = onecad_app::dto::ReindexReportDto {
        total: 1,
        indexed: 1,
        skipped: vec![],
    };
    catalog.accept(reindex, Ok(Reply::Reindexed(report)), &ctx);
    let stale = Reply::Thumbnails(vec![(
        ("a".into(), "1.0.0".into()),
        Ok(super::thumbs::Decoded {
            size: [1, 1],
            rgba: vec![255; 4],
        }),
    )]);
    assert!(!catalog.accept(batch, Ok(stale), &ctx));
    assert!(catalog.texture("a", "1.0.0").is_none());
}

#[test]
fn typing_settles_before_the_search_is_sent() {
    let ctx = ctx();
    let mut catalog = super::Catalog::default();
    catalog.prepare();
    let seq = sequence_of(&catalog.take_outbox()[0]);
    catalog.accept(seq, Ok(Reply::Searched(vec![])), &ctx);
    catalog.query.text = "m6".into();
    catalog.text_edited();
    assert!(catalog.debounce_pending());
    catalog.prepare();
    assert!(catalog.take_outbox().is_empty(), "still typing");
    catalog.prepare_at(std::time::Instant::now() + std::time::Duration::from_millis(300));
    let queued = catalog.take_outbox();
    let Request::Search { query, .. } = &queued[0] else {
        panic!("expected a search");
    };
    assert_eq!(query.text.as_deref(), Some("m6"));
    assert!(!catalog.debounce_pending());
}
