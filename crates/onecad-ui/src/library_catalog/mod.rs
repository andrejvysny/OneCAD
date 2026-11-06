//! Component catalog: the library service lane and the store the library pages read.
//!
//! Requests ride `Command::Library` and replies return as `Message::Library`, exactly
//! like `document_items`. The store sequences each lane so only the newest reply of a
//! lane is applied. Pages never talk to services: they change the store, and
//! [`pump`] (called by the page each frame) submits what the store queued.
use crate::{app::App, bridge::Command};
use onecad_app::dto::{
    ClassifyElementDto, ComponentDetailDto, LibraryComponentDto, ReindexReportDto, SearchQueryDto,
};
use onecad_core::document::record::{ComponentParamValue, PlaceComponentParams};
use std::collections::BTreeMap;

pub(crate) mod draft;
mod service;
mod store;
mod thumbs;

#[cfg(test)]
mod draft_tests;
#[cfg(test)]
mod service_tests;
#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
mod tests;

pub(crate) use service::{execute, guarded};
pub(crate) use store::{Catalog, PreviewState, ReindexState, Source, Status};

/// Independent request streams; each keeps only its newest reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Lane {
    Search,
    Detail,
    Reindex,
    Preview,
    Thumbnails,
    /// Placement: stage the draft's blob and author its params.
    Prepare,
    /// Placement: classify the hovered target and mint its identity.
    Classify,
}

#[derive(Clone, Debug)]
pub(crate) enum Request {
    Search {
        sequence: u64,
        query: SearchQueryDto,
    },
    Detail {
        sequence: u64,
        id: String,
        version: String,
    },
    Reindex {
        sequence: u64,
    },
    Preview {
        sequence: u64,
        id: String,
        version: String,
        params: BTreeMap<String, ComponentParamValue>,
    },
    Thumbnails {
        sequence: u64,
        items: Vec<(thumbs::Key, std::path::PathBuf)>,
    },
    Prepare {
        sequence: u64,
        generation: Option<u64>,
        request: Box<onecad_app::library::PlaceComponentRequest>,
    },
    Classify {
        sequence: u64,
        target: Box<ClassifyTarget>,
    },
    /// Re-placing an existing instance: stage its geometry, load its definition.
    PrepareEdit {
        sequence: u64,
        params: Box<PlaceComponentParams>,
    },
}

/// A hovered body sub-shape the placement gesture asks the backend about.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ClassifyTarget {
    pub body_id: String,
    /// A snapshot pick names its TopoKey; a promoted pick names its element id.
    pub topo_key: Option<String>,
    pub element_id: Option<String>,
    /// The pick's world position, the promotion's anchor evidence.
    pub point: [f64; 3],
    /// The editor context the pick came from; a stale one is refused by the reader.
    pub fence: Option<onecad_app::dto::GeometryReadFenceDto>,
}

/// What the backend knows about a hovered target.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Classified {
    pub class: Option<ClassifyElementDto>,
    /// Rust-minted identity, present only when a mate is possible.
    pub element_id: Option<String>,
}

impl Request {
    pub(crate) fn sequence(&self) -> u64 {
        match self {
            Self::Search { sequence, .. }
            | Self::Detail { sequence, .. }
            | Self::Reindex { sequence }
            | Self::Preview { sequence, .. }
            | Self::Thumbnails { sequence, .. }
            | Self::Prepare { sequence, .. }
            | Self::Classify { sequence, .. }
            | Self::PrepareEdit { sequence, .. } => *sequence,
        }
    }

    pub(crate) fn lane(&self) -> Lane {
        match self {
            Self::Search { .. } => Lane::Search,
            Self::Detail { .. } => Lane::Detail,
            Self::Reindex { .. } => Lane::Reindex,
            Self::Preview { .. } => Lane::Preview,
            Self::Thumbnails { .. } => Lane::Thumbnails,
            Self::Prepare { .. } | Self::PrepareEdit { .. } => Lane::Prepare,
            Self::Classify { .. } => Lane::Classify,
        }
    }
}

/// Size of a previewed candidate, from its MESH1 bounding boxes.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PreviewSummary {
    /// Bounding-box extent in the package's units (mm), X, Y, Z.
    pub size: [f64; 3],
    pub bodies: usize,
    pub triangles: u64,
}

/// A placement reply as the gesture reads it.
#[derive(Clone, Debug)]
pub(crate) enum Placed {
    Prepared(Box<PlaceComponentParams>),
    Classified(Box<Classified>),
    Edit(Box<LibraryComponentDto>),
    Unexpected,
}

#[derive(Debug)]
pub(crate) enum Reply {
    Searched(Vec<LibraryComponentDto>),
    Detail(Box<ComponentDetailDto>),
    Reindexed(ReindexReportDto),
    Previewed(PreviewSummary),
    Thumbnails(Vec<(thumbs::Key, Result<thumbs::Decoded, String>)>),
    Prepared(Box<PlaceComponentParams>),
    Classified(Box<Classified>),
    Edit(Box<LibraryComponentDto>),
}

/// Submits what the page queued this frame. A refused submission fails its lane so the
/// page shows the reason instead of waiting for a reply that cannot arrive.
pub(crate) fn pump(app: &mut App, ctx: &egui::Context) {
    app.ui.library.catalog.prepare();
    if app.ui.library.catalog.debounce_pending() {
        ctx.request_repaint_after(store::SEARCH_DEBOUNCE);
    }
    for request in app.ui.library.catalog.take_outbox() {
        let sequence = request.sequence();
        if let Err(error) = app.doc.bridge.submit(Command::Library(request)) {
            app.ui.library.catalog.accept(sequence, Err(error), ctx);
        }
    }
}

/// Submits queued requests outside the library pages (the placement gesture runs in the
/// editor, where no page calls [`pump`]).
pub(crate) fn flush(app: &mut App) {
    for request in app.ui.library.catalog.take_outbox() {
        let sequence = request.sequence();
        if let Err(error) = app.doc.bridge.submit(Command::Library(request)) {
            app.ui.library.catalog.refuse(sequence, error);
        }
    }
}

pub(crate) fn receive(
    app: &mut App,
    sequence: u64,
    result: Result<Reply, String>,
    ctx: &egui::Context,
) {
    app.ui.library.catalog.accept(sequence, result, ctx);
}
