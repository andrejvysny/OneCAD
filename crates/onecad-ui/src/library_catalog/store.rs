//! Catalog state read by the library pages: query, results, selected component, replies.
use super::{
    ClassifyTarget, Lane, Placed, PreviewSummary, Reply, Request,
    draft::ParamDraft,
    thumbs::{self, Key},
};
use onecad_app::dto::{LibraryComponentDto, ReindexReportDto, SearchQueryDto, SearchSortDto};
use onecad_core::document::record::PlaceComponentParams;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::time::{Duration, Instant};

/// Results whose previews are decoded; the rest keep their icon.
const MAX_THUMBNAILS: usize = 64;
/// A thumbnail batch with no reply this long is abandoned so new ones can queue.
const BATCH_TIMEOUT: Duration = Duration::from_secs(10);
/// Typing settles this long before a search is sent.
pub(crate) const SEARCH_DEBOUNCE: Duration = Duration::from_millis(250);
const THUMBNAIL_BATCH: usize = 24;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Idle,
    Loading,
    Failed(String),
}

/// `source_kind` filter; the strings are the index's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    All,
    Generator,
    Embedded,
    Document,
    Profile,
}

impl Source {
    pub(crate) const ALL: [Self; 5] = [
        Self::All,
        Self::Generator,
        Self::Embedded,
        Self::Document,
        Self::Profile,
    ];

    pub(crate) fn kind(self) -> Option<&'static str> {
        match self {
            Self::All => None,
            Self::Generator => Some("generator"),
            Self::Embedded => Some("embedded"),
            Self::Document => Some("document"),
            Self::Profile => Some("profile"),
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::All => "All sources",
            Self::Generator => "Generated parts",
            Self::Embedded => "Embedded solids",
            Self::Document => "Document sources",
            Self::Profile => "Profiles",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Query {
    pub text: String,
    pub category: Option<String>,
    pub source: Source,
    pub sort: SearchSortDto,
}

impl Query {
    /// Catalog browsing always shows the newest version of each component.
    fn dto(&self) -> SearchQueryDto {
        let text = self.text.trim();
        SearchQueryDto {
            text: (!text.is_empty()).then(|| text.to_owned()),
            category: self.category.clone(),
            source_kind: self.source.kind().map(str::to_owned),
            latest_only: true,
            sort: self.sort,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PreviewState {
    Idle,
    Running,
    Done(PreviewSummary),
    Failed(String),
}

#[derive(Clone, Debug)]
pub(crate) struct DetailView {
    pub id: String,
    pub version: String,
    pub component: Option<LibraryComponentDto>,
    pub versions: Vec<String>,
    pub status: Status,
    pub draft: Option<ParamDraft>,
}

#[derive(Clone, Debug)]
pub(crate) enum ReindexState {
    Idle,
    Running,
    Done(ReindexReportDto),
    Failed(String),
}

#[derive(Clone, Debug)]
struct Batch {
    keys: Vec<Key>,
    sent: Instant,
}

#[derive(Clone, Debug)]
pub(crate) struct Catalog {
    pub query: Query,
    results: Vec<LibraryComponentDto>,
    search: Status,
    categories: BTreeSet<String>,
    started: bool,
    dirty: bool,
    detail: Option<DetailView>,
    preview: PreviewState,
    reindex: ReindexState,
    generation: u64,
    thumbs: thumbs::Cache,
    outbox: Vec<Request>,
    next: u64,
    latest: BTreeMap<Lane, u64>,
    batches: BTreeMap<u64, Batch>,
    text_edited: Option<Instant>,
    /// Placement replies by request sequence, read by the gesture that issued them.
    placement: BTreeMap<u64, Result<Placed, String>>,
}

impl Default for Catalog {
    fn default() -> Self {
        Self {
            query: Query {
                text: String::new(),
                category: None,
                source: Source::All,
                sort: SearchSortDto::Name,
            },
            results: Vec::new(),
            search: Status::Idle,
            categories: BTreeSet::new(),
            started: false,
            dirty: false,
            detail: None,
            preview: PreviewState::Idle,
            reindex: ReindexState::Idle,
            generation: 0,
            thumbs: Default::default(),
            outbox: Vec::new(),
            next: 0,
            latest: BTreeMap::new(),
            batches: BTreeMap::new(),
            text_edited: None,
            placement: BTreeMap::new(),
        }
    }
}

impl Catalog {
    // ---- reads ----
    pub(crate) fn results(&self) -> &[LibraryComponentDto] {
        &self.results
    }
    pub(crate) fn search_status(&self) -> &Status {
        &self.search
    }
    /// True once a search reply has been applied (an empty list is then a real answer).
    pub(crate) fn loaded(&self) -> bool {
        self.started && self.search == Status::Idle && !self.dirty
    }
    pub(crate) fn categories(&self) -> impl Iterator<Item = &str> {
        self.categories.iter().map(String::as_str)
    }
    pub(crate) fn detail(&self) -> Option<&DetailView> {
        self.detail.as_ref()
    }
    pub(crate) fn detail_mut(&mut self) -> Option<&mut DetailView> {
        self.detail.as_mut()
    }
    pub(crate) fn preview(&self) -> &PreviewState {
        &self.preview
    }
    /// Counts completed reindexes; instance statuses depend on the indexed library.
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }
    pub(crate) fn reindex_state(&self) -> &ReindexState {
        &self.reindex
    }
    pub(crate) fn texture(&self, id: &str, version: &str) -> Option<(egui::TextureId, [usize; 2])> {
        self.thumbs.texture(&(id.to_owned(), version.to_owned()))
    }
    pub(crate) fn take_outbox(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.outbox)
    }

    // ---- query ----
    pub(crate) fn requery(&mut self) {
        self.text_edited = None;
        self.dirty = true;
    }

    /// Search text changed: the search goes out once typing pauses.
    pub(crate) fn text_edited(&mut self) {
        self.text_edited = Some(Instant::now());
    }

    pub(crate) fn debounce_pending(&self) -> bool {
        self.text_edited.is_some()
    }

    pub(crate) fn clear_filters(&mut self) {
        self.query.text.clear();
        self.query.category = None;
        self.query.source = Source::All;
        self.requery();
    }

    pub(crate) fn has_filters(&self) -> bool {
        !self.query.text.trim().is_empty()
            || self.query.category.is_some()
            || self.query.source != Source::All
    }

    // ---- detail ----
    /// Selects a component; its record and version list arrive from the service, while
    /// the search hit (when there is one) fills the page immediately.
    pub(crate) fn open_detail(&mut self, id: &str, version: &str) {
        let hit = self
            .results
            .iter()
            .find(|c| c.id == id && c.version == version)
            .cloned();
        let draft = hit.as_ref().map(ParamDraft::from_component);
        self.detail = Some(DetailView {
            id: id.to_owned(),
            version: version.to_owned(),
            component: hit,
            versions: Vec::new(),
            status: Status::Loading,
            draft,
        });
        self.invalidate(Lane::Preview);
        self.preview = PreviewState::Idle;
        self.issue_detail();
    }

    pub(crate) fn close_detail(&mut self) {
        self.detail = None;
        self.invalidate(Lane::Detail);
        self.invalidate(Lane::Preview);
        self.preview = PreviewState::Idle;
    }

    // ---- actions ----
    pub(crate) fn reindex(&mut self) {
        let sequence = self.issue(Lane::Reindex);
        self.reindex = ReindexState::Running;
        self.outbox.push(Request::Reindex { sequence });
    }

    /// The draft changed: an earlier preview (even one in flight) no longer describes it.
    pub(crate) fn draft_edited(&mut self) {
        self.invalidate(Lane::Preview);
        self.preview = PreviewState::Idle;
    }

    /// Requests a preview of the draft; refused while any field is refused.
    pub(crate) fn request_preview(&mut self) {
        let Some(view) = &self.detail else { return };
        let Some(draft) = &view.draft else { return };
        let Ok(params) = draft.resolve() else { return };
        let (id, version) = (view.id.clone(), view.version.clone());
        let sequence = self.issue(Lane::Preview);
        self.preview = PreviewState::Running;
        self.outbox.push(Request::Preview {
            sequence,
            id,
            version,
            params,
        });
    }

    // ---- placement lanes ----
    /// Stages and authors a draft for `request`; the reply is read with
    /// [`Self::take_placement_reply`].
    pub(crate) fn request_prepare(
        &mut self,
        request: onecad_app::library::PlaceComponentRequest,
    ) -> u64 {
        let sequence = self.issue(Lane::Prepare);
        self.outbox.push(Request::Prepare {
            sequence,
            generation: None,
            request: Box::new(request),
        });
        sequence
    }

    pub(crate) fn request_prepare_edit(&mut self, params: PlaceComponentParams) -> u64 {
        let sequence = self.issue(Lane::Prepare);
        self.outbox.push(Request::PrepareEdit {
            sequence,
            params: Box::new(params),
        });
        sequence
    }

    pub(crate) fn request_classify(&mut self, target: ClassifyTarget) -> u64 {
        let sequence = self.issue(Lane::Classify);
        self.outbox.push(Request::Classify {
            sequence,
            target: Box::new(target),
        });
        sequence
    }

    /// The reply to placement request `sequence`, once it has arrived and is not superseded.
    pub(crate) fn take_placement_reply(&mut self, sequence: u64) -> Option<Result<Placed, String>> {
        self.placement.remove(&sequence)
    }

    /// Forgets every placement request: in-flight replies are dropped on arrival.
    pub(crate) fn drop_placement(&mut self) {
        self.invalidate(Lane::Prepare);
        self.invalidate(Lane::Classify);
        self.placement.clear();
    }

    /// A placement request the host could not submit fails its lane immediately.
    pub(crate) fn refuse(&mut self, sequence: u64, error: String) {
        for lane in [Lane::Prepare, Lane::Classify] {
            if self.latest.get(&lane) == Some(&sequence) {
                self.latest.remove(&lane);
                self.placement.insert(sequence, Err(error.clone()));
                return;
            }
        }
    }

    // ---- frame hook ----
    /// Queues the first load, a pending re-query and any missing thumbnails.
    pub(crate) fn prepare(&mut self) {
        self.prepare_at(Instant::now());
    }

    pub(crate) fn prepare_at(&mut self, now: Instant) {
        if self
            .text_edited
            .is_some_and(|at| now.duration_since(at) >= SEARCH_DEBOUNCE)
        {
            self.requery();
        }
        self.expire_batches(now);
        if !self.started {
            self.started = true;
            self.dirty = true;
        }
        if self.dirty {
            self.dirty = false;
            self.issue_search();
        }
        self.queue_thumbnails();
    }

    fn issue(&mut self, lane: Lane) -> u64 {
        self.next += 1;
        self.latest.insert(lane, self.next);
        self.outbox.retain(|queued| queued.lane() != lane);
        self.next
    }

    /// Forgets a lane's newest request so its reply, if it still arrives, is dropped.
    fn invalidate(&mut self, lane: Lane) {
        self.latest.remove(&lane);
        self.outbox.retain(|queued| queued.lane() != lane);
    }

    fn issue_search(&mut self) {
        let sequence = self.issue(Lane::Search);
        self.search = Status::Loading;
        let query = self.query.dto();
        self.outbox.push(Request::Search { sequence, query });
    }

    fn issue_detail(&mut self) {
        let Some(view) = &self.detail else { return };
        let (id, version) = (view.id.clone(), view.version.clone());
        let sequence = self.issue(Lane::Detail);
        self.outbox.push(Request::Detail {
            sequence,
            id,
            version,
        });
    }

    /// Abandons batches that never replied; their late reply is then dropped.
    fn expire_batches(&mut self, now: Instant) {
        let stale: Vec<u64> = self
            .batches
            .iter()
            .filter(|(_, b)| now.duration_since(b.sent) >= BATCH_TIMEOUT)
            .map(|(sequence, _)| *sequence)
            .collect();
        for sequence in stale {
            if let Some(batch) = self.batches.remove(&sequence) {
                batch
                    .keys
                    .iter()
                    .for_each(|k| self.thumbs.forget_pending(k));
            }
        }
    }

    fn queue_thumbnails(&mut self) {
        if !self.batches.is_empty() {
            return;
        }
        let mut seen = HashSet::new();
        let detail = self.detail.as_ref().and_then(|d| d.component.as_ref());
        let wanted: Vec<(Key, std::path::PathBuf)> = detail
            .into_iter()
            .chain(self.results.iter().take(MAX_THUMBNAILS))
            .filter_map(|c| {
                let path = c.preview_path.as_ref()?;
                Some(((c.id.clone(), c.version.clone()), path.into()))
            })
            .filter(|(key, _)| self.thumbs.absent(key) && seen.insert(key.clone()))
            .take(THUMBNAIL_BATCH)
            .collect();
        if wanted.is_empty() {
            return;
        }
        self.next += 1;
        let sequence = self.next;
        for (key, _) in &wanted {
            self.thumbs.mark_pending(key.clone());
        }
        self.batches.insert(
            sequence,
            Batch {
                keys: wanted.iter().map(|(key, _)| key.clone()).collect(),
                sent: Instant::now(),
            },
        );
        self.outbox.push(Request::Thumbnails {
            sequence,
            items: wanted,
        });
    }

    // ---- replies ----
    /// Applies a reply; false when it was superseded (a newer request of its lane exists,
    /// or the lane was invalidated) and therefore dropped.
    pub(crate) fn accept(
        &mut self,
        sequence: u64,
        result: Result<Reply, String>,
        ctx: &egui::Context,
    ) -> bool {
        if let Some(batch) = self.batches.remove(&sequence) {
            self.settle_thumbnails(batch.keys, result, ctx);
            return true;
        }
        let Some(lane) = self
            .latest
            .iter()
            .find_map(|(lane, latest)| (*latest == sequence).then_some(*lane))
        else {
            return false;
        };
        self.latest.remove(&lane);
        match (lane, result) {
            (Lane::Search, result) => self.searched(result),
            (Lane::Detail, result) => self.detailed(result),
            (Lane::Reindex, result) => self.reindexed(result),
            (Lane::Prepare | Lane::Classify, result) => {
                let placed = result.map(|reply| match reply {
                    Reply::Prepared(params) => Placed::Prepared(params),
                    Reply::Classified(known) => Placed::Classified(known),
                    Reply::Edit(component) => Placed::Edit(component),
                    _ => Placed::Unexpected,
                });
                self.placement.insert(sequence, placed);
            }
            (_, result) => self.previewed(result),
        }
        true
    }

    fn settle_thumbnails(
        &mut self,
        keys: Vec<Key>,
        result: Result<Reply, String>,
        ctx: &egui::Context,
    ) {
        let mut decoded = match result {
            Ok(Reply::Thumbnails(list)) => list,
            _ => Vec::new(),
        };
        for key in keys {
            match decoded.iter().position(|(k, _)| *k == key) {
                Some(at) => self.thumbs.settle(ctx, key, decoded.swap_remove(at).1),
                None => self.thumbs.fail(key),
            }
        }
    }

    fn searched(&mut self, result: Result<Reply, String>) {
        match result {
            Ok(Reply::Searched(hits)) => {
                // Facets come from any answer not narrowed by category, so choosing a
                // category never removes the other categories from the list.
                if self.query.category.is_none() {
                    self.categories
                        .extend(hits.iter().flat_map(|c| c.category.iter().cloned()));
                }
                self.results = hits;
                self.search = Status::Idle;
            }
            Ok(_) => self.search_failed("unexpected library reply".into()),
            Err(error) => self.search_failed(error),
        }
    }

    fn search_failed(&mut self, error: String) {
        self.results.clear();
        self.search = Status::Failed(error);
    }

    fn detailed(&mut self, result: Result<Reply, String>) {
        let Some(view) = self.detail.as_mut() else {
            return;
        };
        match result {
            Ok(Reply::Detail(detail)) if detail.component.id == view.id => {
                let changed = view.component.as_ref() != Some(&detail.component);
                if changed || view.draft.is_none() {
                    view.draft = Some(ParamDraft::from_component(&detail.component));
                }
                view.component = Some(detail.component);
                view.versions = detail.versions;
                view.status = Status::Idle;
            }
            Ok(_) => view.status = Status::Failed("unexpected library reply".into()),
            Err(error) => view.status = Status::Failed(error),
        }
    }

    fn reindexed(&mut self, result: Result<Reply, String>) {
        match result {
            Ok(Reply::Reindexed(report)) => {
                self.reindex = ReindexState::Done(report);
                self.generation += 1;
                // Pre-reindex thumbnail replies no longer describe the library.
                self.batches.clear();
                self.thumbs.clear();
                self.categories.clear();
                self.dirty = true;
                if self.detail.is_some() {
                    self.issue_detail();
                }
            }
            Ok(_) => self.reindex = ReindexState::Failed("unexpected library reply".into()),
            Err(error) => self.reindex = ReindexState::Failed(error),
        }
    }

    fn previewed(&mut self, result: Result<Reply, String>) {
        self.preview = match result {
            Ok(Reply::Previewed(summary)) => PreviewState::Done(summary),
            Ok(_) => PreviewState::Failed("unexpected library reply".into()),
            Err(error) => PreviewState::Failed(error),
        };
    }
}
