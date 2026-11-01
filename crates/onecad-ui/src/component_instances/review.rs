//! Upgrade review (design 15a/15b): the plan the user approves, item by item.
use onecad_app::dto::UpgradeItemDto;
use std::collections::BTreeSet;

pub(crate) const STALE: &str = "upgrade plan is stale";
pub(crate) const REPLANNED: &str = "The document changed; review the updated plan";

#[derive(Clone, Debug, Default)]
pub(crate) struct Review {
    /// Every instance the review was opened for; a re-plan asks for the same set.
    pub record_ids: Vec<String>,
    pub items: Vec<UpgradeItemDto>,
    pub selected: BTreeSet<String>,
    pub loading: bool,
    pub notice: Option<String>,
    pub error: Option<String>,
}

impl Review {
    pub(crate) fn opening(record_ids: Vec<String>) -> Self {
        Self {
            record_ids,
            loading: true,
            ..Self::default()
        }
    }

    /// A fresh plan arrived: applicable items start selected, a re-plan keeps only the
    /// choices that still apply.
    pub(crate) fn planned(&mut self, items: Vec<UpgradeItemDto>) {
        let keep: Option<BTreeSet<String>> = self.notice.as_ref().map(|_| self.selected.clone());
        self.selected = items
            .iter()
            .filter(|item| item.applicable)
            .filter(|item| keep.as_ref().is_none_or(|k| k.contains(&item.record_id)))
            .map(|item| item.record_id.clone())
            .collect();
        self.items = items;
        self.loading = false;
        self.error = None;
    }

    pub(crate) fn toggle(&mut self, record_id: &str) {
        let applicable = self
            .items
            .iter()
            .any(|item| item.applicable && item.record_id == record_id);
        if !applicable {
            return;
        }
        if !self.selected.remove(record_id) {
            self.selected.insert(record_id.to_owned());
        }
    }

    pub(crate) fn approved_selected(&self) -> Vec<UpgradeItemDto> {
        self.items
            .iter()
            .filter(|item| item.applicable && self.selected.contains(&item.record_id))
            .cloned()
            .collect()
    }

    pub(crate) fn approved_all(&self) -> Vec<UpgradeItemDto> {
        self.items
            .iter()
            .filter(|item| item.applicable)
            .cloned()
            .collect()
    }

    /// The plan went stale: ask again for the same instances and say why.
    pub(crate) fn stale(&mut self) {
        self.loading = true;
        self.notice = Some(REPLANNED.to_owned());
    }
}
