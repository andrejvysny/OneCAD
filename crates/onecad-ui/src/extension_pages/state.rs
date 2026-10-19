//! Mutable facade state for the extensions pages. Transitions are pure so tests can drive them.
use super::catalog::{self, CATALOG, CATEGORY_CHIPS, Ext, update_info};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ManagerTab {
    #[default]
    Discover,
    Installed,
    Updates,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DetailTab {
    #[default]
    Overview,
    Settings,
    Changelog,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RowState {
    Normal,
    Failed,
    Unsupported,
    /// Mock install progress in 0..=1.
    Installing(f32),
    Removed,
    Cancelled,
}

#[derive(Clone, Debug)]
pub struct InstRow {
    pub id: &'static str,
    pub name: &'static str,
    pub icon: &'static str,
    pub version: &'static str,
    pub note: &'static str,
    pub state: RowState,
    pub enabled: bool,
    pub update: Option<&'static str>,
}

/// How a catalogue card presents an extension.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Status {
    Installed,
    Update(&'static str),
    Available,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sheet {
    Install(&'static str),
    Update(&'static str),
}

#[derive(Clone, Debug)]
pub struct Toast {
    pub text: String,
    pub until: f64,
}

/// CNC Prep settings (20a); held here because the host would persist them per extension.
#[derive(Clone, Debug, PartialEq)]
pub struct CncSettings {
    pub relief: usize,
    pub diameter: String,
    pub mark_unreachable: bool,
    pub resolution: usize,
}

impl Default for CncSettings {
    fn default() -> Self {
        Self {
            relief: 0,
            diameter: "6.00".to_owned(),
            mark_unreachable: true,
            resolution: 1,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ExtensionsUi {
    pub tab: ManagerTab,
    pub category: usize,
    pub search: String,
    pub source: usize,
    pub detail: &'static str,
    pub detail_tab: DetailTab,
    pub rows: Vec<InstRow>,
    pub sheet: Option<Sheet>,
    pub toast: Option<Toast>,
    pub settings: CncSettings,
    pub developer: bool,
    /// Seconds (mock clock) of the last developer reload; `None` = initial 12:04:31.
    pub reloaded_at: Option<u32>,
}

impl Default for ExtensionsUi {
    fn default() -> Self {
        Self {
            tab: ManagerTab::default(),
            category: 0,
            search: String::new(),
            source: 0,
            detail: CATALOG[0].id,
            detail_tab: DetailTab::default(),
            rows: initial_rows(),
            sheet: None,
            toast: None,
            settings: CncSettings::default(),
            developer: false,
            reloaded_at: None,
        }
    }
}

fn row(ext: &Ext, state: RowState, enabled: bool) -> InstRow {
    InstRow {
        id: ext.id,
        name: ext.name,
        icon: ext.icon,
        version: ext.version,
        note: ext.meta,
        state,
        enabled,
        update: None,
    }
}

fn catalog_ext(id: &str) -> &'static Ext {
    catalog::find(id).unwrap_or(&CATALOG[0])
}

fn initial_rows() -> Vec<InstRow> {
    let mut print = row(catalog_ext("print-prep"), RowState::Normal, true);
    print.update = Some("1.3.0");
    let mut kicad = row(catalog_ext("kicad-bridge"), RowState::Removed, true);
    kicad.note = "Removed from the catalogue by the publisher · still works";
    let mut wood = row(
        catalog_ext("wood-joinery"),
        RowState::Installing(0.62),
        true,
    );
    wood.note = "";
    vec![
        row(catalog_ext("cnc-prep"), RowState::Normal, true),
        print,
        InstRow {
            id: "topology-check",
            name: "Topology Check",
            icon: "measure",
            version: "0.9.2",
            note: "Stopped responding at 14:02 · native code",
            state: RowState::Failed,
            enabled: true,
            update: None,
        },
        InstRow {
            id: "fixture-plates",
            name: "Fixture Plates",
            icon: "body",
            version: "2.1.0",
            note: "Needs OneCAD 0.4 — you have 0.3.2",
            state: RowState::Unsupported,
            enabled: false,
            update: None,
        },
        wood,
        kicad,
    ]
}

/// Cards with these states count as not installed (Install stays offered).
fn offers_install(state: RowState) -> bool {
    matches!(
        state,
        RowState::Installing(_) | RowState::Removed | RowState::Cancelled
    )
}

/// Case-insensitive match of the query against name or publisher.
pub fn matches_query(ext: &Ext, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty()
        || ext.name.to_lowercase().contains(&q)
        || ext.publisher.to_lowercase().contains(&q)
}

impl ExtensionsUi {
    pub fn row(&self, id: &str) -> Option<&InstRow> {
        self.rows.iter().find(|r| r.id == id)
    }

    fn row_mut(&mut self, id: &str) -> Option<&mut InstRow> {
        self.rows.iter_mut().find(|r| r.id == id)
    }

    pub fn status(&self, id: &str) -> Status {
        match self.row(id) {
            Some(r) if !offers_install(r.state) => {
                r.update.map_or(Status::Installed, Status::Update)
            }
            _ => Status::Available,
        }
    }

    /// Catalogue indices passing the search text, category chip and source select.
    pub fn visible(&self) -> Vec<usize> {
        let category = CATEGORY_CHIPS.get(self.category).and_then(|c| c.1);
        CATALOG
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                matches_query(e, &self.search)
                    && category.is_none_or(|c| e.category == c)
                    && (self.source == 0 || self.source == catalog::source_of(e))
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// (installed, active, updates) for the manager meta line.
    pub fn counts(&self) -> (usize, usize, usize) {
        let active =
            |r: &&InstRow| r.enabled && matches!(r.state, RowState::Normal | RowState::Removed);
        let installed = self
            .rows
            .iter()
            .filter(|r| r.state != RowState::Cancelled)
            .count();
        let updates = self.rows.iter().filter(|r| r.update.is_some()).count();
        (installed, self.rows.iter().filter(active).count(), updates)
    }

    pub fn request_install(&mut self, id: &'static str) {
        self.sheet = Some(Sheet::Install(id));
    }

    pub fn request_update(&mut self, id: &'static str) {
        if update_info(id).is_some() {
            self.sheet = Some(Sheet::Update(id));
        }
    }

    /// Marks the sheet's extension installed and enabled; returns the toast text.
    pub fn confirm_install(&mut self) -> Option<String> {
        let Some(Sheet::Install(id)) = self.sheet.take() else {
            return None;
        };
        let ext = catalog_ext(id);
        let fresh = row(ext, RowState::Normal, true);
        match self.row_mut(id) {
            Some(existing) => *existing = fresh,
            None => self.rows.push(fresh),
        }
        Some(format!("{} added · {}", ext.name, ext.meta))
    }

    /// Applies the pending update: new version, update cleared.
    pub fn confirm_update(&mut self) {
        let Some(Sheet::Update(id)) = self.sheet.take() else {
            return;
        };
        let Some(info) = update_info(id) else { return };
        if let Some(r) = self.row_mut(id) {
            r.version = info.to;
            r.update = None;
        }
    }

    pub fn close_sheet(&mut self) {
        self.sheet = None;
    }

    pub fn restart(&mut self, id: &str) {
        if let Some(r) = self.row_mut(id) {
            r.state = RowState::Normal;
            r.enabled = true;
            r.note = "Running";
        }
    }

    pub fn cancel_install(&mut self, id: &str) {
        if let Some(r) = self.row_mut(id) {
            r.state = RowState::Cancelled;
        }
    }

    pub fn uninstall(&mut self, id: &str) {
        self.rows.retain(|r| r.id != id);
    }

    pub fn reset_settings(&mut self) {
        self.settings = CncSettings::default();
    }

    pub fn reload_developer(&mut self, now_secs: u32) {
        self.reloaded_at = Some(now_secs);
    }

    pub fn show_toast(&mut self, text: String, now: f64) {
        self.toast = Some(Toast {
            text,
            until: now + 3.0,
        });
    }
}

/// Mock developer clock: 12:04:31 plus the seconds since the first reload request.
pub fn mock_clock(offset_secs: Option<u32>) -> String {
    let total = 12 * 3600 + 4 * 60 + 31 + offset_secs.unwrap_or(0);
    format!(
        "{:02}:{:02}:{:02}",
        total / 3600 % 24,
        total / 60 % 60,
        total % 60
    )
}
