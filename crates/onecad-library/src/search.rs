//! Catalog search over the cached index (`Library::search`).
//!
//! Pure filtering of [`LibraryIndex`]; never touches disk, so the answer is as
//! fresh as the last reindex.

use std::cmp::Ordering;

use crate::index::{compare_versions, IndexEntry, LibraryIndex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchSort {
    /// Case-insensitive display name.
    #[default]
    Name,
    Id,
}

#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    /// Whitespace-separated terms; every term must match (case-insensitive
    /// substring) one of id, name, designation, standard, tags, category.
    pub text: Option<String>,
    /// Exact match against any `category` element.
    pub category: Option<String>,
    /// Exact match against `source_kind`.
    pub source_kind: Option<String>,
    /// Keep only the newest version of each id. The newest is chosen per id
    /// BEFORE the other filters, so an older version matching the query is not
    /// a hit when a newer one exists.
    pub latest_only: bool,
    pub sort: SearchSort,
}

#[derive(Debug, Clone, Copy)]
pub struct SearchHit<'a> {
    pub id: &'a str,
    pub version: &'a str,
    pub entry: &'a IndexEntry,
}

pub(crate) fn run<'a>(index: &'a LibraryIndex, query: &SearchQuery) -> Vec<SearchHit<'a>> {
    let terms: Vec<String> = query
        .text
        .as_deref()
        .map(|t| t.split_whitespace().map(str::to_lowercase).collect())
        .unwrap_or_default();
    let mut hits: Vec<SearchHit<'a>> = Vec::new();
    for (id, versions) in &index.components {
        let latest = query.latest_only.then(|| index.latest(id)).flatten();
        for (version, entry) in versions {
            if latest.is_some_and(|(v, _)| v != version) {
                continue;
            }
            if matches_filters(id, entry, query, &terms) {
                hits.push(SearchHit { id, version, entry });
            }
        }
    }
    hits.sort_by(|a, b| compare_hits(a, b, query.sort));
    hits
}

fn matches_filters(id: &str, entry: &IndexEntry, query: &SearchQuery, terms: &[String]) -> bool {
    if let Some(cat) = &query.category {
        if !entry.category.iter().any(|c| c == cat) {
            return false;
        }
    }
    if let Some(kind) = &query.source_kind {
        if &entry.source_kind != kind {
            return false;
        }
    }
    if terms.is_empty() {
        return true;
    }
    let haystack = haystack(id, entry);
    terms.iter().all(|t| haystack.iter().any(|h| h.contains(t)))
}

fn haystack(id: &str, entry: &IndexEntry) -> Vec<String> {
    let mut fields = vec![id.to_lowercase(), entry.name.to_lowercase()];
    fields.extend(entry.designation.iter().map(|s| s.to_lowercase()));
    fields.extend(entry.standard.iter().map(|s| s.to_lowercase()));
    fields.extend(entry.tags.iter().map(|s| s.to_lowercase()));
    fields.extend(entry.category.iter().map(|s| s.to_lowercase()));
    fields
}

fn compare_hits(a: &SearchHit<'_>, b: &SearchHit<'_>, sort: SearchSort) -> Ordering {
    let primary = match sort {
        SearchSort::Name => a
            .entry
            .name
            .to_lowercase()
            .cmp(&b.entry.name.to_lowercase()),
        SearchSort::Id => Ordering::Equal,
    };
    primary
        .then_with(|| a.id.cmp(b.id))
        .then_with(|| compare_versions(a.version, b.version))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn entry(name: &str, kind: &str, category: &[&str], tags: &[&str]) -> IndexEntry {
        IndexEntry {
            path: name.to_string(),
            name: name.to_string(),
            category: category.iter().map(|s| s.to_string()).collect(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            source_kind: kind.to_string(),
            parameter_keys: Vec::new(),
            revision: "sha256:00".to_string(),
            generator_id: None,
            generator_version: None,
            attachments: BTreeMap::new(),
            parameters: BTreeMap::new(),
            designation: None,
            standard: None,
            unit: None,
            has_preview: false,
            configurations: BTreeMap::new(),
            default_configuration: None,
        }
    }

    fn fixture() -> LibraryIndex {
        let mut idx = LibraryIndex::default();
        let mut screw = entry(
            "Socket Head Cap Screw",
            "generator",
            &["fasteners", "screws"],
            &["bolt"],
        );
        screw.standard = Some("ISO 4762".into());
        screw.designation = Some("ISO 4762 M{thread}x{length}".into());
        let bearing = entry("Bearing 608", "embedded", &["bearings"], &["skate"]);
        let mut screw_old = screw.clone();
        screw_old.name = "Old Screw".into();
        screw_old.tags.clear();
        let mut v = BTreeMap::new();
        v.insert("1.9.0".to_string(), screw_old);
        v.insert("1.10.0".to_string(), screw);
        idx.components.insert("acme.screw".into(), v);
        idx.components.insert(
            "acme.bearing".into(),
            BTreeMap::from([("1.0.0".to_string(), bearing)]),
        );
        idx
    }

    fn q(text: &str) -> SearchQuery {
        SearchQuery {
            text: Some(text.into()),
            ..Default::default()
        }
    }

    fn ids(hits: &[SearchHit<'_>]) -> Vec<String> {
        hits.iter()
            .map(|h| format!("{}@{}", h.id, h.version))
            .collect()
    }

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn empty_query_sorts_by_name_then_id_then_semver() {
        let idx = fixture();
        // "Bearing 608" < "Old Screw" < "Socket Head Cap Screw".
        assert_eq!(
            ids(&run(&idx, &SearchQuery::default())),
            strs(&[
                "acme.bearing@1.0.0",
                "acme.screw@1.9.0",
                "acme.screw@1.10.0"
            ])
        );
    }

    #[test]
    fn sort_by_id_ties_break_on_semver_not_lexicographic() {
        let idx = fixture();
        let query = SearchQuery {
            sort: SearchSort::Id,
            ..Default::default()
        };
        assert_eq!(
            ids(&run(&idx, &query)),
            strs(&[
                "acme.bearing@1.0.0",
                "acme.screw@1.9.0",
                "acme.screw@1.10.0"
            ])
        );
    }

    #[test]
    fn same_name_ties_break_on_id_then_semver() {
        let mut idx = fixture();
        for v in idx.components.get_mut("acme.screw").unwrap().values_mut() {
            v.name = "Same".into();
        }
        idx.components
            .get_mut("acme.bearing")
            .unwrap()
            .get_mut("1.0.0")
            .unwrap()
            .name = "Same".into();
        assert_eq!(
            ids(&run(&idx, &SearchQuery::default())),
            strs(&[
                "acme.bearing@1.0.0",
                "acme.screw@1.9.0",
                "acme.screw@1.10.0"
            ])
        );
    }

    #[test]
    fn text_is_trimmed_case_insensitive_and_every_term_must_match() {
        let idx = fixture();
        assert_eq!(run(&idx, &q("  SCREW  ")).len(), 2);
        assert_eq!(
            ids(&run(&idx, &q("iso bolt"))),
            strs(&["acme.screw@1.10.0"])
        );
        assert!(run(&idx, &q("iso skate")).is_empty());
        assert_eq!(run(&idx, &q("   ")).len(), 3, "blank text is no filter");
        assert_eq!(run(&idx, &q("fasteners")).len(), 2, "category is searched");
        assert_eq!(
            run(&idx, &q("M{thread}")).len(),
            2,
            "designation is searched"
        );
    }

    #[test]
    fn category_and_source_kind_are_exact() {
        let idx = fixture();
        let by_cat = |c: &str| SearchQuery {
            category: Some(c.into()),
            ..Default::default()
        };
        assert_eq!(run(&idx, &by_cat("screws")).len(), 2);
        assert!(run(&idx, &by_cat("screw")).is_empty());
        let by_kind = |k: &str| SearchQuery {
            source_kind: Some(k.into()),
            ..Default::default()
        };
        assert_eq!(
            ids(&run(&idx, &by_kind("embedded"))),
            strs(&["acme.bearing@1.0.0"])
        );
        assert!(run(&idx, &by_kind("embed")).is_empty());
    }

    #[test]
    fn latest_only_keeps_the_semver_newest_per_id() {
        let idx = fixture();
        let query = SearchQuery {
            latest_only: true,
            ..Default::default()
        };
        assert_eq!(
            ids(&run(&idx, &query)),
            strs(&["acme.bearing@1.0.0", "acme.screw@1.10.0"])
        );
    }
}
